//! One source file as the measure sees it: which of its lines are test code, and whether
//! it holds a shape that would hide code from the measure.

use std::collections::BTreeSet;

use proc_macro2::extra::invalidate_current_thread_spans;
use proc_macro2::{Delimiter, Ident, LexError, LineColumn, Span, TokenStream, TokenTree};
use syn::ext::IdentExt;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{
    AttrStyle, Attribute, ExprClosure, ForeignItem, ImplItem, ImplItemFn, Item, ItemFn, ItemMod,
    Macro, Meta, TraitItem, TraitItemFn,
};

use super::attributes::{is_cfg_gate, is_cfg_test, is_named, problem_of};
use crate::error::SourceProblem;

/// Brackets nested deeper than code a person writes. Within it, the parse fits the stack
/// it runs on. A long chain without brackets, such as `!!!…x` or `Vec<Vec<…>>` a hundred
/// thousand deep, can still exhaust that stack: check-coverage then stops without naming
/// the file, and the gate fails.
const MAX_DEPTH: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub line_count: usize,
    /// The lines, numbered from 1, that hold test code and nothing else.
    pub test_lines: BTreeSet<usize>,
    /// A function or closure body outside test code and under no cfg: unless the `mod`
    /// that declares the file carries one, the report must name the file.
    pub has_function_body: bool,
    /// The out-of-line `mod` lines outside test code, for the files they declare.
    pub modules: Vec<ModuleLine>,
}

/// An out-of-line `mod` line of a source file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleLine {
    /// The names of the inline modules around it, outermost first, then its own name.
    pub path: Vec<String>,
    /// It, or an item or a module around it, carries a cfg other than the test mark: the
    /// file it declares may be compiled out of the measured build.
    pub gated: bool,
}

/// A shape check-coverage refuses, at the line where it starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Finding {
    pub line: usize,
    pub problem: SourceProblem,
}

impl Source {
    /// Reads the Rust source `text`. The first finding by line, if any, is the error. syn
    /// parses by recursion, so this runs on a stack of several megabytes: see `sources.rs`.
    pub fn read(text: &str) -> Result<Self, Finding> {
        let _spans = SpanScope;
        let tokens = tokenize(text)?;
        let file: syn::File = syn::parse2(tokens.clone()).map_err(|error| Finding {
            line: error.span().start().line,
            problem: SourceProblem::Unparsable,
        })?;
        let mut walk = ItemWalk::default();
        walk.visit_file(&file);
        let mut scan = TokenScan {
            test_items: &walk.test_items,
            all_test_code: false,
            production_lines: BTreeSet::new(),
            findings: Vec::new(),
        };
        scan.scan(tokens);
        let findings = walk.findings.iter().chain(&scan.findings);
        if let Some(first) = findings.min_by_key(|finding| finding.line) {
            return Err(*first);
        }
        let test_lines = walk
            .test_items
            .iter()
            .flat_map(|(start, end)| start.line..=end.line)
            .filter(|line| !scan.production_lines.contains(line))
            .collect();
        Ok(Self {
            line_count: text.lines().count(),
            test_lines,
            has_function_body: walk.has_function_body,
            modules: walk.modules,
        })
    }

    /// Checks a test module that cargo-llvm-cov leaves out of its report, a `tests.rs` or a
    /// file under a `tests/` folder. Only `include` and `#[path]` matter there: either could
    /// pull production code into a test build, where it would run without being measured.
    pub fn check_test_module(text: &str) -> Result<(), Finding> {
        let _spans = SpanScope;
        let mut scan = TokenScan {
            test_items: &[],
            all_test_code: true,
            production_lines: BTreeSet::new(),
            findings: Vec::new(),
        };
        scan.scan(tokenize(text)?);
        match scan.findings.iter().min_by_key(|finding| finding.line) {
            Some(first) => Err(*first),
            None => Ok(()),
        }
    }
}

/// The tokens of `text`, refused when they do not lex or nest deeper than `MAX_DEPTH`.
fn tokenize(text: &str) -> Result<TokenStream, Finding> {
    let tokens: TokenStream = text.parse().map_err(|error: LexError| Finding {
        line: error.span().start().line,
        problem: SourceProblem::Unparsable,
    })?;
    if let Some(line) = too_deep(&tokens) {
        return Err(Finding {
            line,
            problem: SourceProblem::TooDeep,
        });
    }
    Ok(tokens)
}

/// The line where the first bracket deeper than `MAX_DEPTH` opens, if any. The walk keeps
/// its own stack, so that no nesting of brackets can exhaust the thread's.
fn too_deep(tokens: &TokenStream) -> Option<usize> {
    let mut open = vec![tokens.clone().into_iter()];
    while let Some(next) = open.last_mut().map(Iterator::next) {
        match next {
            Some(TokenTree::Group(group)) => {
                if open.len() > MAX_DEPTH {
                    return Some(group.span_open().start().line);
                }
                open.push(group.stream().into_iter());
            }
            Some(_) => {}
            None => {
                open.pop();
            }
        }
    }
    None
}

/// proc-macro2 keeps the text of every file it parses in a map of its thread, which only
/// grows, and places every token by a 32-bit offset into it. Clearing the map once a file
/// is read keeps both bounded, however many files one thread reads; no span outlives the
/// call.
struct SpanScope;

impl Drop for SpanScope {
    fn drop(&mut self) {
        invalidate_current_thread_spans();
    }
}

/// Finds the test items, at any depth, the function bodies outside them and outside every
/// cfg, the `mod` lines, the `macro_rules!` bodies that could hide code, and the modules
/// whose files the measure would miss.
#[derive(Default)]
struct ItemWalk {
    /// Where each test item starts and ends, its attributes and doc comments included.
    test_items: Vec<(LineColumn, LineColumn)>,
    /// The names of the inline modules around the walk, innermost last.
    inline_modules: Vec<String>,
    in_test_code: bool,
    /// How many items around the walk carry a cfg other than the test mark.
    gates: usize,
    has_function_body: bool,
    modules: Vec<ModuleLine>,
    findings: Vec<Finding>,
}

impl<'ast> Visit<'ast> for ItemWalk {
    fn visit_file(&mut self, file: &'ast syn::File) {
        // `#![cfg(…)]` at the top of a file gates the whole module, as on its `mod` line.
        let gated = is_gated(&file.attrs);
        self.gates += usize::from(gated);
        visit::visit_file(self, file);
        self.gates -= usize::from(gated);
    }

    fn visit_item(&mut self, item: &'ast Item) {
        self.enter(item_attributes(item), item, |walk| {
            visit::visit_item(walk, item);
        });
    }

    fn visit_impl_item(&mut self, item: &'ast ImplItem) {
        self.enter(impl_item_attributes(item), item, |walk| {
            visit::visit_impl_item(walk, item);
        });
    }

    fn visit_trait_item(&mut self, item: &'ast TraitItem) {
        self.enter(trait_item_attributes(item), item, |walk| {
            visit::visit_trait_item(walk, item);
        });
    }

    fn visit_foreign_item(&mut self, item: &'ast ForeignItem) {
        self.enter(foreign_item_attributes(item), item, |walk| {
            visit::visit_foreign_item(walk, item);
        });
    }

    fn visit_item_mod(&mut self, module: &'ast ItemMod) {
        if let Some(problem) = self.module_problem(module) {
            self.findings.push(Finding {
                line: module.mod_token.span.start().line,
                problem,
            });
        }
        let inline = module.content.is_some();
        let name = module.ident.unraw().to_string();
        if !inline && !self.in_test_code {
            let mut path = self.inline_modules.clone();
            path.push(name.clone());
            self.modules.push(ModuleLine {
                path,
                gated: self.gates > 0,
            });
        }
        if inline {
            self.inline_modules.push(name);
        }
        visit::visit_item_mod(self, module);
        if inline {
            self.inline_modules.pop();
        }
    }

    fn visit_item_fn(&mut self, function: &'ast ItemFn) {
        self.note_a_body();
        visit::visit_item_fn(self, function);
    }

    fn visit_impl_item_fn(&mut self, function: &'ast ImplItemFn) {
        self.note_a_body();
        visit::visit_impl_item_fn(self, function);
    }

    fn visit_trait_item_fn(&mut self, function: &'ast TraitItemFn) {
        if function.default.is_some() {
            self.note_a_body();
        }
        visit::visit_trait_item_fn(self, function);
    }

    fn visit_expr_closure(&mut self, closure: &'ast ExprClosure) {
        self.note_a_body();
        visit::visit_expr_closure(self, closure);
    }

    /// Only a `macro_rules!` definition outside test code is judged: any other macro call
    /// names only what its expansion compiles in both builds, and `cfg!(…)` compiles both
    /// branches, so neither hides code.
    fn visit_macro(&mut self, called: &'ast Macro) {
        if !self.in_test_code && is_named(&called.path, "macro_rules") {
            judge_macro_rules_body(&called.tokens, &mut self.findings);
        }
        visit::visit_macro(self, called);
    }
}

impl ItemWalk {
    /// Visits an item: as test code from there down when it carries exactly `#[cfg(test)]`,
    /// and as gated code when it carries another cfg.
    fn enter<T: Spanned>(
        &mut self,
        attributes: &[Attribute],
        item: &T,
        visit: impl FnOnce(&mut Self),
    ) {
        let marks_tests = !self.in_test_code && is_test_item(attributes);
        let gated = is_gated(attributes);
        if marks_tests {
            let span = item.span();
            self.test_items.push((span.start(), span.end()));
            self.in_test_code = true;
        }
        self.gates += usize::from(gated);
        visit(self);
        self.gates -= usize::from(gated);
        if marks_tests {
            self.in_test_code = false;
        }
    }

    /// A body under a cfg may be compiled out of the measured build, a platform's code on
    /// another platform or a feature left off, so it does not make the report name its file.
    fn note_a_body(&mut self) {
        if !self.in_test_code && self.gates == 0 {
            self.has_function_body = true;
        }
    }

    /// cargo-llvm-cov leaves out of its report every file named `tests.rs` or `*_tests.rs`
    /// and every file under a folder named `tests`, and check-coverage reads every other
    /// file under `src/` as production. So a module's name must say what its file holds.
    fn module_problem(&self, module: &ItemMod) -> Option<SourceProblem> {
        let name = module.ident.unraw().to_string();
        let marked = is_test_item(&module.attrs);
        match (module.content.is_some(), marked, self.in_test_code) {
            (false, true, _) => (name != "tests").then_some(SourceProblem::OutOfLineTestModule),
            (false, false, true) => (name != "tests"
                && !self.inline_modules.iter().any(|module| module == "tests"))
            .then_some(SourceProblem::StrayTestModuleFile),
            (false, false, false) => (name == "tests" || name.ends_with("_tests"))
                .then_some(SourceProblem::UnmarkedTestsModule),
            (true, _, in_test_code) => {
                (!in_test_code && name == "tests").then_some(SourceProblem::UnmarkedTestsModule)
            }
        }
    }
}

/// What a `macro_rules!` body outside test code may not write: the keyword `mod`, whose
/// module and file would escape the module rules, and a test attribute, whose test would
/// count as production wherever the macro is called. Its other attributes and its
/// `include!` are judged by the token scan, as everywhere. An attribute whose path is a
/// metavariable, `#[$meta]`, carries what its caller writes, and is not refused.
fn judge_macro_rules_body(body: &TokenStream, findings: &mut Vec<Finding>) {
    let tokens: Vec<TokenTree> = body.clone().into_iter().collect();
    for (index, token) in tokens.iter().enumerate() {
        match token {
            // `r#mod` is a name, not the keyword, and declares nothing.
            TokenTree::Ident(word) if word == "mod" => findings.push(Finding {
                line: word.span().start().line,
                problem: SourceProblem::MacroWritesModule,
            }),
            TokenTree::Group(group) => {
                if group.delimiter() == Delimiter::Bracket
                    && let Some(pound) = attribute_pound(&tokens[..index])
                    && writes_a_test(group.stream())
                {
                    findings.push(Finding {
                        line: pound.start().line,
                        problem: SourceProblem::MacroWritesTests,
                    });
                }
                judge_macro_rules_body(&group.stream(), findings);
            }
            _ => {}
        }
    }
}

/// `#[test]`, `#[cfg(test)]` or another attribute named `test`, written with its path.
fn writes_a_test(content: TokenStream) -> bool {
    let Ok(meta) = syn::parse2::<Meta>(content) else {
        return false;
    };
    is_cfg_test(&meta)
        || meta
            .path()
            .segments
            .last()
            .is_some_and(|segment| segment.ident.unraw() == "test")
}

fn is_gated(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| is_cfg_gate(&attribute.meta))
}

fn is_test_item(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        matches!(attribute.style, AttrStyle::Outer) && is_cfg_test(&attribute.meta)
    })
}

fn item_attributes(item: &Item) -> &[Attribute] {
    match item {
        Item::Const(item) => &item.attrs,
        Item::Enum(item) => &item.attrs,
        Item::ExternCrate(item) => &item.attrs,
        Item::Fn(item) => &item.attrs,
        Item::ForeignMod(item) => &item.attrs,
        Item::Impl(item) => &item.attrs,
        Item::Macro(item) => &item.attrs,
        Item::Mod(item) => &item.attrs,
        Item::Static(item) => &item.attrs,
        Item::Struct(item) => &item.attrs,
        Item::Trait(item) => &item.attrs,
        Item::TraitAlias(item) => &item.attrs,
        Item::Type(item) => &item.attrs,
        Item::Union(item) => &item.attrs,
        Item::Use(item) => &item.attrs,
        // Raw tokens: a `#[cfg(test)]` inside is no mark, so the token scan refuses it.
        _ => &[],
    }
}

fn impl_item_attributes(item: &ImplItem) -> &[Attribute] {
    match item {
        ImplItem::Const(item) => &item.attrs,
        ImplItem::Fn(item) => &item.attrs,
        ImplItem::Type(item) => &item.attrs,
        ImplItem::Macro(item) => &item.attrs,
        _ => &[],
    }
}

fn trait_item_attributes(item: &TraitItem) -> &[Attribute] {
    match item {
        TraitItem::Const(item) => &item.attrs,
        TraitItem::Fn(item) => &item.attrs,
        TraitItem::Type(item) => &item.attrs,
        TraitItem::Macro(item) => &item.attrs,
        _ => &[],
    }
}

fn foreign_item_attributes(item: &ForeignItem) -> &[Attribute] {
    match item {
        ForeignItem::Fn(item) => &item.attrs,
        ForeignItem::Static(item) => &item.attrs,
        ForeignItem::Type(item) => &item.attrs,
        ForeignItem::Macro(item) => &item.attrs,
        _ => &[],
    }
}

/// Goes over every token of the file, macro bodies included, which the syntax tree keeps
/// as raw tokens: it judges every attribute and every `include`, wherever they stand, and
/// records which lines hold production tokens.
struct TokenScan<'a> {
    test_items: &'a [(LineColumn, LineColumn)],
    /// For a test module file, which is test code from its first line to its last.
    all_test_code: bool,
    /// Lines that hold a token outside every test item, so that a line a test item shares
    /// with production code is never left out.
    production_lines: BTreeSet<usize>,
    findings: Vec<Finding>,
}

impl TokenScan<'_> {
    /// Recursion is safe here: `tokenize` refused anything deeper than `MAX_DEPTH`.
    fn scan(&mut self, stream: TokenStream) {
        let tokens: Vec<TokenTree> = stream.into_iter().collect();
        for (index, token) in tokens.iter().enumerate() {
            let TokenTree::Group(group) = token else {
                self.note(token.span());
                if let TokenTree::Ident(word) = token {
                    self.judge_include(word, &tokens[index + 1..]);
                }
                continue;
            };
            self.note(group.span_open());
            self.note(group.span_close());
            if group.delimiter() == Delimiter::Bracket
                && let Some(pound) = attribute_pound(&tokens[..index])
            {
                self.judge_attribute(pound, group.stream());
            }
            self.scan(group.stream());
        }
    }

    /// `include!`, other than from `OUT_DIR`, and a `use` that names `include`, which would
    /// let it be called by another name. Test code too: an included file is measured, or
    /// dropped, under its own path, so a test build could run production code no line
    /// counts. A field, a method or a variable named `include` is none of these.
    fn judge_include(&mut self, word: &Ident, after: &[TokenTree]) {
        let line = if word.unraw() == "include" {
            let called =
                matches!(after.first(), Some(TokenTree::Punct(bang)) if bang.as_char() == '!');
            (called && !is_out_dir_include(after)).then(|| word.span().start().line)
        } else if word == "use" {
            use_names_include(after)
        } else {
            None
        };
        if let Some(line) = line {
            self.findings.push(Finding {
                line,
                problem: SourceProblem::Include,
            });
        }
    }

    fn note(&mut self, span: Span) {
        if !self.in_test_item(span.start()) {
            self.production_lines
                .extend(span.start().line..=span.end().line);
        }
    }

    fn in_test_item(&self, position: LineColumn) -> bool {
        self.all_test_code
            || self
                .test_items
                .iter()
                .any(|(start, end)| *start <= position && position < *end)
    }

    /// An attribute that does not parse is no attribute rustc would take either: a macro
    /// pattern such as `#[$meta]`, for one.
    fn judge_attribute(&mut self, pound: Span, content: TokenStream) {
        let Ok(meta) = syn::parse2::<Meta>(content) else {
            return;
        };
        if let Some(problem) = problem_of(&meta, self.in_test_item(pound.start())) {
            self.findings.push(Finding {
                line: pound.start().line,
                problem,
            });
        }
    }
}

/// The line of `include` in the path of the `use` declaration that `after` follows, up to its
/// `;`. `use<…>` after `impl Trait +` captures generics, and is no declaration.
fn use_names_include(after: &[TokenTree]) -> Option<usize> {
    if matches!(after.first(), Some(TokenTree::Punct(open)) if open.as_char() == '<') {
        return None;
    }
    let mut open = vec![after.iter().cloned().collect::<TokenStream>().into_iter()];
    while let Some(next) = open.last_mut().map(Iterator::next) {
        match next {
            Some(TokenTree::Punct(end)) if end.as_char() == ';' && open.len() == 1 => return None,
            Some(TokenTree::Ident(name)) if name.unraw() == "include" => {
                return Some(name.span().start().line);
            }
            Some(TokenTree::Group(group)) => open.push(group.stream().into_iter()),
            Some(_) => {}
            None => {
                open.pop();
            }
        }
    }
    None
}

/// What follows `include` in `include!(concat!(env!("OUT_DIR"), "<file>"))`, with no `..`
/// in the file: generated code, which lives under `target/`.
fn is_out_dir_include(after: &[TokenTree]) -> bool {
    let [TokenTree::Punct(bang), TokenTree::Group(arguments), ..] = after else {
        return false;
    };
    let arguments: Vec<TokenTree> = arguments.stream().into_iter().collect();
    let [
        TokenTree::Ident(concat),
        TokenTree::Punct(concat_bang),
        TokenTree::Group(parts),
    ] = arguments.as_slice()
    else {
        return false;
    };
    let parts: Vec<TokenTree> = parts.stream().into_iter().collect();
    let [
        TokenTree::Ident(env),
        TokenTree::Punct(env_bang),
        TokenTree::Group(variable),
        TokenTree::Punct(comma),
        TokenTree::Literal(file),
    ] = parts.as_slice()
    else {
        return false;
    };
    let variable: Vec<TokenTree> = variable.stream().into_iter().collect();
    let file = file.to_string();
    bang.as_char() == '!'
        && concat == "concat"
        && concat_bang.as_char() == '!'
        && env == "env"
        && env_bang.as_char() == '!'
        && matches!(variable.as_slice(), [TokenTree::Literal(name)] if name.to_string() == "\"OUT_DIR\"")
        && comma.as_char() == ','
        && file.starts_with('"')
        && !file.contains("..")
}

/// The `#` that makes the bracket group after `before` an attribute: `#[…]` or `#![…]`.
fn attribute_pound(before: &[TokenTree]) -> Option<Span> {
    match before {
        [.., TokenTree::Punct(pound), TokenTree::Punct(bang)]
            if pound.as_char() == '#' && bang.as_char() == '!' =>
        {
            Some(pound.span())
        }
        [.., TokenTree::Punct(pound)] if pound.as_char() == '#' => Some(pound.span()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use SourceProblem::*;

    // A fixture is a list of lines, never one literal that spans lines, so its line
    // numbers can be read off the list.
    fn read(lines: &[&str]) -> Result<Source, Finding> {
        Source::read(&(lines.join("\n") + "\n"))
    }

    fn test_lines(lines: &[&str]) -> Vec<usize> {
        let source = read(lines).unwrap();
        source.test_lines.into_iter().collect()
    }

    fn finding(lines: &[&str]) -> (usize, SourceProblem) {
        let finding = read(lines).unwrap_err();
        (finding.line, finding.problem)
    }

    #[test]
    fn a_test_module_is_left_out_whatever_sits_between_its_attribute_and_its_name() {
        for (shape, lines) in [
            (
                "a wrapped attribute",
                &[
                    "#[cfg(test)]",
                    "#[expect(",
                    "    clippy::too_many_lines,",
                    "    reason = \"one long table\"",
                    ")]",
                    "mod tests {",
                ][..],
            ),
            (
                "a plain comment",
                &["#[cfg(test)]", "// the fixtures are long", "mod tests {"],
            ),
            (
                "doc comments",
                &["/// Tests.", "#[cfg(test)]", "/// More.", "mod tests {"],
            ),
            (
                "another name and visibility",
                &["#[cfg(test)]", "pub(crate) mod fakes {"],
            ),
            (
                "an attribute before it",
                &["#[allow(dead_code)] // why", "#[cfg(test)]", "mod tests {"],
            ),
        ] {
            let source = [
                &["pub fn production() {}", ""][..],
                lines,
                &["    fn works() {}", "}", "", "pub fn after() {}"],
            ]
            .concat();
            let first = 3;
            let last = 2 + lines.len() + 2;
            assert_eq!(
                test_lines(&source),
                (first..=last).collect::<Vec<_>>(),
                "{shape}"
            );
        }
    }

    #[test]
    fn a_brace_at_column_0_in_a_raw_string_does_not_end_a_test_module() {
        let source = [
            "#[cfg(test)]",
            "mod tests {",
            "    const BODY: &str = r#\"",
            "{",
            "  \"amount\": 1",
            "}",
            "\"#;",
            "    fn works() {}",
            "}",
            "pub fn after() {}",
        ];
        assert_eq!(test_lines(&source), (1..=9).collect::<Vec<_>>());
    }

    #[test]
    fn production_code_after_a_test_module_or_under_a_commented_attribute_is_kept() {
        let after = [
            "#[cfg(test)]",
            "mod tests {",
            "    fn works() {}",
            "} // tests",
            "",
            "mod ledger {",
            "    pub fn balance() {}",
            "}",
        ];
        assert_eq!(test_lines(&after), [1, 2, 3, 4]);
        let commented = [
            "/* Pricing rules. Kept apart from the rest:",
            "#[cfg(test)]",
            "#[doc(hidden)] */",
            "mod pricing {",
            "    pub fn price() {}",
            "}",
        ];
        assert_eq!(test_lines(&commented), [] as [usize; 0]);
    }

    #[test]
    fn any_item_that_carries_cfg_test_is_test_code_at_any_depth() {
        for (shape, source, expected) in [
            (
                "a function",
                &[
                    "#[cfg(test)]",
                    "fn helper() {",
                    "    work();",
                    "}",
                    "fn real() {}",
                ][..],
                &[1, 2, 3, 4][..],
            ),
            (
                "an impl",
                &["#[cfg(test)]", "impl Clock {", "    fn set() {}", "}"],
                &[1, 2, 3, 4],
            ),
            (
                "a method",
                &[
                    "impl Clock {",
                    "    #[cfg(test)]",
                    "    fn set() {",
                    "    }",
                    "    fn now() {}",
                    "}",
                ],
                &[2, 3, 4],
            ),
            (
                "a nested module",
                &[
                    "mod outer {",
                    "    #[cfg(test)]",
                    "    mod tests {",
                    "        fn works() {}",
                    "    }",
                    "}",
                ],
                &[2, 3, 4, 5],
            ),
            (
                "a trait item",
                &["trait Port {", "    #[cfg(test)]", "    fn fake();", "}"],
                &[2, 3],
            ),
            (
                "a foreign item",
                &[
                    "unsafe extern \"C\" {",
                    "    #[cfg(test)]",
                    "    fn probe();",
                    "}",
                ],
                &[2, 3],
            ),
            (
                "an item in a function body",
                &[
                    "fn real() {",
                    "    #[cfg(test)]",
                    "    fn helper() {}",
                    "    work();",
                    "}",
                ],
                &[2, 3],
            ),
            (
                "a use, a const and a macro item",
                &[
                    "#[cfg(test)]",
                    "use std::fs;",
                    "#[cfg(test)]",
                    "const FIXTURE: u8 = 1;",
                    "#[cfg(test)]",
                    "thread_local! { static X: u8 = 1; }",
                ],
                &[1, 2, 3, 4, 5, 6],
            ),
            (
                "an attribute path written as a raw identifier",
                &["#[r#cfg(test)]", "mod r#tests {", "    fn works() {}", "}"],
                &[1, 2, 3, 4],
            ),
            (
                "test code inside test code",
                &[
                    "#[cfg(test)]",
                    "mod tests {",
                    "    #[cfg(test)]",
                    "    fn helper() {}",
                    "}",
                ],
                &[1, 2, 3, 4, 5],
            ),
        ] {
            assert_eq!(test_lines(source), expected, "{shape}");
        }
    }

    #[test]
    fn a_line_that_a_test_item_shares_with_production_code_is_kept() {
        let source = [
            "pub fn before() {} #[cfg(test)] fn helper() {",
            "    work();",
            "} pub fn after() {}",
        ];
        assert_eq!(test_lines(&source), [2]);
    }

    #[test]
    fn test_support_and_an_out_of_line_module_named_tests_are_fine() {
        let source = [
            "#[cfg(any(test, feature = \"testing\"))]",
            "mod fixed {",
            "    pub fn set() {}",
            "}",
            "#[cfg(test)]",
            "mod tests;",
            "mod outer {",
            "    #[cfg(test)]",
            "    mod tests;",
            "}",
        ];
        assert_eq!(test_lines(&source), [5, 6, 8, 9]);
    }

    #[test]
    fn each_shape_that_would_hide_code_is_refused_at_its_line() {
        for (source, expected) in [
            (&["#[cfg(test)]", "mod unit;"][..], (2, OutOfLineTestModule)),
            (&["mod ab_tests;"], (1, UnmarkedTestsModule)),
            (&["pub mod tests;"], (1, UnmarkedTestsModule)),
            (
                &["mod tests {", "    fn works() {}", "}"],
                (1, UnmarkedTestsModule),
            ),
            (
                &["#[cfg(test)]", "mod fakes {", "    mod helpers;", "}"],
                (3, StrayTestModuleFile),
            ),
            (
                &["#[cfg(not(coverage))]", "pub fn f() {}"],
                (1, NamesCoverage),
            ),
            (
                &["#[coverage(off)]", "pub fn f() {}"],
                (1, CoverageAttribute),
            ),
            (
                &[
                    "#[cfg_attr(coverage_nightly, coverage(off))]",
                    "pub fn f() {}",
                ],
                (1, NamesCoverage),
            ),
            (
                &["#[cfg(not(test))]", "pub fn f() {}"],
                (1, TestInOtherForm),
            ),
            (
                &["#[cfg(all(test, unix))]", "mod tests {}"],
                (1, TestInOtherForm),
            ),
            (
                &["pub fn f() {", "    #[cfg(test)]", "    let x = 1;", "}"],
                (2, CfgTestNotOnItem),
            ),
            (
                &["pub struct S {", "    #[cfg(test)]", "    field: u8,", "}"],
                (2, CfgTestNotOnItem),
            ),
            (
                &["pub enum E {", "    #[cfg(test)]", "    Fake,", "}"],
                (2, CfgTestNotOnItem),
            ),
            (&["#![cfg(test)]", "pub fn f() {}"], (1, CfgTestNotOnItem)),
            (
                &["#[path = \"../../shared/logic.rs\"]", "mod logic;"],
                (1, PathAttribute),
            ),
            (
                &[
                    "#[cfg(test)]",
                    "mod tests {",
                    "    #[path = \"x.rs\"]",
                    "    mod x;",
                    "}",
                ],
                (3, PathAttribute),
            ),
            (
                &["#[cfg_attr(test, cfg(unix))]", "pub fn f() {}"],
                (1, CfgInCfgAttr),
            ),
            (&["#[test]", "fn works() {}"], (1, TestOutsideTestCode)),
            (
                &[
                    "passthrough! {",
                    "    #[cfg(test)]",
                    "    mod tests {}",
                    "}",
                ],
                (2, CfgTestNotOnItem),
            ),
        ] {
            assert_eq!(finding(source), expected, "{source:?}");
        }
    }

    #[test]
    fn include_is_refused_when_called_or_imported_unless_from_out_dir() {
        for (source, line) in [
            (&["pub fn f() {}", "include!(\"tests.rs\");"][..], 2),
            (&["include!(concat!(\"../tests/\", \"helpers.rs\"));"], 1),
            (
                &[
                    "#[cfg(test)]",
                    "mod tests {",
                    "    macro_rules! inc {",
                    "        ($f:expr) => { include!($f); };",
                    "    }",
                    "}",
                ],
                4,
            ),
            (
                &["use std::include as inc;", "inc!(\"../tests/helpers.rs\");"],
                1,
            ),
            (&["use std::{fs, include as inc};"], 1),
            (&["pub use core::include;"], 1),
            (
                &["include!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/src/tests.rs\"));"],
                1,
            ),
            (
                &["include!(concat!(env!(\"OUT_DIR\"), \"/../../src/tests.rs\"));"],
                1,
            ),
            (
                &["include!(concat!(env!(\"OUT_DIR\"), \"/gen.rs\", \"x\"));"],
                1,
            ),
            (&["const X: u8 = r#include! { \"x.rs\" };"], 1),
            (
                &[
                    "#[cfg(test)]",
                    "mod tests {",
                    "    mod again {",
                    "        include!(\"money.rs\");",
                    "    }",
                    "}",
                ],
                4,
            ),
        ] {
            assert_eq!(finding(source), (line, Include), "{source:?}");
        }
        for source in [
            &["include!(concat!(env!(\"OUT_DIR\"), \"/generated.rs\"));"][..],
            &[
                "mod generated {",
                "    std::include!(concat!(env!(\"OUT_DIR\"), \"/g.rs\"));",
                "}",
            ],
            &["const TEXT: &str = include_str!(\"../README.md\");"],
            &["const BYTES: &[u8] = include_bytes!(\"../logo.png\");"],
            &[
                "pub struct Query {",
                "    pub include: Vec<String>,",
                "}",
                "impl Query {",
                "    pub fn include(&self) -> bool {",
                "        let include = !self.include.is_empty();",
                "        include",
                "    }",
                "}",
            ],
            &[
                "pub fn wants(query: Query) -> Query {",
                "    Query { include: vec![] }",
                "}",
            ],
            &[
                "pub fn sized() -> impl Sized + use<> {",
                "    include_nothing()",
                "}",
            ],
            &[
                "use std::fs;",
                "pub fn f(include: bool) -> bool {",
                "    include",
                "}",
            ],
        ] {
            assert!(read(source).is_ok(), "{source:?}");
        }
    }

    #[test]
    fn a_macro_rules_body_may_not_write_mod_or_tests_outside_test_code() {
        for (source, expected) in [
            (
                &[
                    "macro_rules! module {",
                    "    ($name:ident) => { pub mod $name; };",
                    "}",
                ][..],
                (2, MacroWritesModule),
            ),
            (
                &[
                    "macro_rules! padding {",
                    "    () => {",
                    "        #[test]",
                    "        fn padding() {}",
                    "    };",
                    "}",
                ],
                (3, MacroWritesTests),
            ),
            (
                &[
                    "macro_rules! fakes {",
                    "    () => {",
                    "        #[cfg(test)]",
                    "        mod fakes {}",
                    "    };",
                    "}",
                ],
                (3, MacroWritesTests),
            ),
            (
                &[
                    "macro_rules! async_test {",
                    "    () => { #[tokio::test] async fn works() {} };",
                    "}",
                ],
                (2, MacroWritesTests),
            ),
            (
                &[
                    "macro_rules! gated {",
                    "    ($p:meta, $i:item) => { #[cfg($p)] $i };",
                    "}",
                ],
                (2, CfgOutsideAllowlist),
            ),
            (
                &[
                    "macro_rules! shipped {",
                    "    () => { #[cfg(not(debug_assertions))] pub fn f() {} };",
                    "}",
                ],
                (2, CfgOutsideAllowlist),
            ),
            (
                &[
                    "macro_rules! unmeasured {",
                    "    () => { #[coverage(off)] pub fn f() {} };",
                    "}",
                ],
                (2, CoverageAttribute),
            ),
            (
                &[
                    "macro_rules! inc {",
                    "    ($f:expr) => { include!($f) };",
                    "}",
                ],
                (2, Include),
            ),
        ] {
            assert_eq!(finding(source), expected, "{source:?}");
        }
        for source in [
            &[
                "macro_rules! define_id {",
                "    ($(#[$attribute:meta])* $name:ident) => {",
                "        $(#[$attribute])*",
                "        #[derive(Debug, Clone, Copy)]",
                "        pub struct $name(u128);",
                "    };",
                "}",
                "define_id!(",
                "    /// Identifies a user.",
                "    UserId",
                ");",
            ][..],
            &[
                "macro_rules! check {",
                "    ($test:expr) => {",
                "        assert!($test)",
                "    };",
                "}",
            ],
            &[
                "macro_rules! run {",
                "    () => { crate::r#mod::run() };",
                "}",
            ],
            &[
                "macro_rules! with_attribute {",
                "    ($a:meta, $i:item) => { #[$a] $i };",
                "}",
            ],
            &[
                "pub fn port(cfg: &Config) -> String {",
                "    format!(\"port {}\", cfg.port)",
                "}",
            ],
            &[
                "impl fmt::Display for Case {",
                "    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {",
                "        write!(f, \"{}\", self.test)",
                "    }",
                "}",
            ],
            &[
                "pub fn f(test: &str) -> u8 {",
                "    debug_assert!(cfg!(target_pointer_width = \"64\"));",
                "    if cfg!(unix) && cfg!(test) { 1 } else { 2 }",
                "}",
            ],
            &[
                "thread_local! {",
                "    #[cfg_attr(unix, allow(unused))]",
                "    static X: u8 = 1;",
                "}",
            ],
            &[
                "#[cfg(test)]",
                "mod tests {",
                "    macro_rules! case {",
                "        ($name:ident) => {",
                "            #[test]",
                "            fn $name() {}",
                "        };",
                "    }",
                "}",
            ],
        ] {
            assert!(read(source).is_ok(), "{source:?}");
        }
    }

    #[test]
    fn a_cfg_outside_the_allowlist_is_refused_at_its_line() {
        for source in [
            &[
                "pub fn f() {}",
                "#[cfg(not(debug_assertions))]",
                "pub fn g() {}",
            ][..],
            &["pub fn f() {}", "#[cfg(shipping)]", "pub fn g() {}"],
            &[
                "pub fn f() {}",
                "#[cfg(not(feature = \"cheap\"))]",
                "pub fn g() {}",
            ],
        ] {
            assert_eq!(finding(source), (2, CfgOutsideAllowlist), "{source:?}");
        }
        let allowed = [
            "#[cfg(unix)]",
            "mod shutdown;",
            "#[cfg(all(feature = \"testing\", not(windows)))]",
            "pub fn g() {}",
            "#[cfg(debug_assertions)]",
            "pub fn dev_only() {}",
        ];
        assert!(read(&allowed).is_ok());
    }

    #[test]
    fn a_function_body_outside_test_code_and_every_cfg_is_noted() {
        for source in [
            &["pub fn f() {}"][..],
            &["impl S {", "    pub fn f(&self) {}", "}"],
            &["pub trait T {", "    fn f(&self) {}", "}"],
            &["pub static F: fn() -> u8 = || 1;"],
            &[
                "#[cfg(windows)]",
                "pub fn win() {}",
                "pub fn everywhere() {}",
            ],
        ] {
            assert!(read(source).unwrap().has_function_body, "{source:?}");
        }
        for source in [
            &["//! Docs only."][..],
            &[
                "mod parser;",
                "pub use parser::parse;",
                "pub struct S;",
                "pub enum E { A }",
            ],
            &["pub trait T {", "    fn f(&self);", "}"],
            &["#[cfg(test)]", "mod tests {", "    fn f() {}", "}"],
            &["impl S {", "    #[cfg(test)]", "    fn f() {}", "}"],
            &["unsafe extern \"C\" {", "    fn probe();", "}"],
            &["#[cfg(feature = \"extra\")]", "pub fn extra() {}"],
            &["#[cfg(windows)]", "impl S {", "    pub fn f(&self) {}", "}"],
            &["#[cfg(unix)]", "mod platform {", "    pub fn f() {}", "}"],
            &[
                "#[cfg(any(test, feature = \"testing\"))]",
                "pub fn helper() {}",
            ],
            &["#![cfg(windows)]", "pub fn only_windows() {}"],
        ] {
            assert!(!read(source).unwrap().has_function_body, "{source:?}");
        }
    }

    #[test]
    fn each_out_of_line_mod_line_is_kept_with_whether_a_cfg_gates_it() {
        let source = read(&[
            "#[cfg(windows)]",
            "mod win;",
            "pub mod plain;",
            "mod outer {",
            "    pub mod inner;",
            "}",
            "#[cfg(unix)]",
            "mod platform {",
            "    pub mod r#linux;",
            "}",
            "#[cfg(test)]",
            "mod tests;",
            "mod inline {}",
        ])
        .unwrap();
        let line = |path: &[&str], gated| ModuleLine {
            path: path.iter().map(|name| (*name).to_owned()).collect(),
            gated,
        };
        assert_eq!(
            source.modules,
            [
                line(&["win"], true),
                line(&["plain"], false),
                line(&["outer", "inner"], false),
                line(&["platform", "linux"], true),
            ]
        );
    }

    /// `depth` nested calls inside a function body.
    fn nested_calls(depth: usize) -> String {
        format!(
            "fn id(n: u8) -> u8 {{\n    n\n}}\n\npub fn deep() -> u8 {{\n    {}1{}\n}}\n",
            "id(".repeat(depth),
            ")".repeat(depth)
        )
    }

    #[test]
    fn a_source_nested_deeper_than_256_groups_is_refused_before_the_parse() {
        // The body's braces are one group already, so 256 calls nest 257 deep.
        let error = Source::read(&nested_calls(256)).unwrap_err();
        assert_eq!((error.line, error.problem), (6, TooDeep));
        let error = Source::read(&nested_calls(200_000)).unwrap_err();
        assert_eq!((error.line, error.problem), (6, TooDeep));
        let error = Source::check_test_module(&nested_calls(1000)).unwrap_err();
        assert_eq!((error.line, error.problem), (6, TooDeep));
    }

    #[test]
    fn a_source_nested_256_groups_deep_parses_on_a_large_stack() {
        let read = std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| Source::read(&nested_calls(255)).map(|source| source.line_count))
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(read, Ok(7));
    }

    #[test]
    fn a_test_module_file_is_checked_for_include_and_path_only() {
        let refused = [
            (
                &["fn helper() {}", "include!(\"../lib.rs\");"][..],
                (2, Include),
            ),
            (
                &["#[path = \"../lib.rs\"]", "mod copy;"],
                (1, PathAttribute),
            ),
            (
                &[
                    "fn f() {",
                    "    #[cfg_attr(unix, path = \"x.rs\")]",
                    "    mod x {}",
                    "}",
                ],
                (2, PathAttribute),
            ),
            (&["fn broken( {"], (1, Unparsable)),
        ];
        for (source, expected) in refused {
            let error = Source::check_test_module(&(source.join("\n") + "\n")).unwrap_err();
            assert_eq!((error.line, error.problem), expected, "{source:?}");
        }
        let allowed = "#[cfg(not(coverage))]\n#[test]\nfn works() {\n    let test = cfg!(test);\n}\nmod helpers;\n";
        assert_eq!(Source::check_test_module(allowed), Ok(()));
    }

    #[test]
    fn modules_whose_files_land_where_the_measure_expects_them_are_fine() {
        for source in [
            &["#[cfg(test)]", "mod tests {", "    mod helpers;", "}"][..],
            &["#[cfg(test)]", "mod fakes {", "    mod tests;", "}"],
            &["mod parser;", "pub mod ab_testing;"],
            &["mod ab_tests {", "    mod variants;", "}"],
        ] {
            assert!(read(source).is_ok(), "{source:?}");
        }
    }

    #[test]
    fn the_first_finding_by_line_is_the_error() {
        let source = [
            "#[cfg(all(test, unix))]",
            "mod tests {}",
            "#[coverage(off)]",
            "fn f() {}",
        ];
        assert_eq!(finding(&source), (1, TestInOtherForm));
    }

    #[test]
    fn a_file_that_does_not_parse_is_refused_at_its_line() {
        assert_eq!(
            finding(&["pub fn ok() {}", "pub fn broken( {}"]),
            (2, Unparsable)
        );
        assert_eq!(
            finding(&["pub fn ok() {}", "", "fn f() { \"open"]),
            (3, Unparsable)
        );
        assert_eq!(finding(&["pub fn ok() {}", "}"]).1, Unparsable);
    }

    #[test]
    fn windows_line_endings_change_nothing() {
        let source = "pub fn production() {}\r\n#[cfg(test)]\r\nmod tests {\r\n}\r\n";
        let read = Source::read(source).unwrap();
        assert_eq!(read.line_count, 4);
        assert_eq!(read.test_lines.into_iter().collect::<Vec<_>>(), [2, 3, 4]);
    }

    #[test]
    fn the_lines_are_counted_as_rustc_counts_them() {
        assert_eq!(Source::read("").unwrap().line_count, 0);
        assert_eq!(Source::read("fn f() {}").unwrap().line_count, 1);
        assert_eq!(Source::read("fn f() {}\n\n").unwrap().line_count, 2);
        assert_eq!(Source::read("\u{feff}fn f() {}\n").unwrap().line_count, 1);
    }
}
