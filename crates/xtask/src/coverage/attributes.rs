//! How check-coverage reads an attribute: whether it marks test code, and whether it could
//! hide code from the measure. Predicates are read as token trees, never as text, so no
//! spelling, spacing or comment can disguise one.

use proc_macro2::{Delimiter, TokenStream, TokenTree};
use syn::ext::IdentExt;
use syn::{Meta, Path};

use crate::error::SourceProblem;

/// What a `cfg` predicate selects, as far as the measure is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Predicate {
    /// Exactly `test`.
    Test,
    /// `any(test, feature = "…")`, either way round: test support that other crates' tests
    /// use, built in every test run, so it is production code to the measure.
    TestSupport,
    /// `test` in any other form, such as `not(test)` or `all(test, unix)`.
    NamesTest,
    /// `coverage` anywhere, `coverage_nightly` included.
    NamesCoverage,
    /// Built from `feature = "…"`, `debug_assertions`, `unix`, `windows` and the `target_*`
    /// keys with `all`, `any` and `not`, and no `not` around a feature or around
    /// `debug_assertions`: a predicate that holds in the build the measure runs whenever it
    /// holds in a build that ships.
    Allowed,
    /// Anything else: `not(debug_assertions)`, a custom cfg or `not(feature = "…")`
    /// compiles code that only a build the measure never runs holds.
    OutsideAllowlist,
}

pub fn classify(predicate: TokenStream) -> Predicate {
    let tokens: Vec<TokenTree> = predicate.into_iter().collect();
    // A feature name counts too: a build for the measure could switch one on.
    if mentions(&tokens, &|word| word.contains("coverage"), true) {
        Predicate::NamesCoverage
    } else if is_word(&tokens, "test") {
        Predicate::Test
    } else if is_test_support(&tokens) {
        Predicate::TestSupport
    } else if mentions(&tokens, &|word| word == "test", false) {
        Predicate::NamesTest
    } else if is_allowed(&tokens, false) {
        Predicate::Allowed
    } else {
        Predicate::OutsideAllowlist
    }
}

/// Whether `tokens` is one predicate of the allowlist. `under_not` is set inside a `not`,
/// where a feature and `debug_assertions` are refused: the measured build is a debug build
/// and may turn a feature on, so the code they leave out of it is never measured.
fn is_allowed(tokens: &[TokenTree], under_not: bool) -> bool {
    match tokens {
        [TokenTree::Ident(word)] => {
            word == "unix" || word == "windows" || (word == "debug_assertions" && !under_not)
        }
        [
            TokenTree::Ident(key),
            TokenTree::Punct(equals),
            TokenTree::Literal(value),
        ] if equals.as_char() == '=' && value.to_string().starts_with('"') => {
            let key = key.to_string();
            (key == "feature" && !under_not) || key.starts_with("target_")
        }
        [TokenTree::Ident(combinator), TokenTree::Group(arguments)]
            if arguments.delimiter() == Delimiter::Parenthesis =>
        {
            let parts = split_at_commas(arguments.stream());
            if combinator == "all" || combinator == "any" {
                parts.iter().all(|part| is_allowed(part, under_not))
            } else if combinator == "not" {
                matches!(parts.as_slice(), [part] if is_allowed(part, true))
            } else {
                false
            }
        }
        _ => false,
    }
}

/// `#[cfg(test)]`, exactly: the mark of a test item.
pub fn is_cfg_test(meta: &Meta) -> bool {
    matches!(meta, Meta::List(list)
        if is_named(&list.path, "cfg") && classify(list.tokens.clone()) == Predicate::Test)
}

/// A `cfg` other than the mark of a test item: what it stands on may be compiled out of the
/// measured build, a platform's code on another platform or a feature left off.
pub fn is_cfg_gate(meta: &Meta) -> bool {
    matches!(meta, Meta::List(list) if is_named(&list.path, "cfg")) && !is_cfg_test(meta)
}

/// What is wrong with the attribute `meta`, if anything. In test code only a `#[path]`
/// matters: test code is left out of the measure anyway, but a module it points elsewhere
/// is not.
pub fn problem_of(meta: &Meta, in_test_code: bool) -> Option<SourceProblem> {
    let path = meta.path();
    let last = path.segments.last().map(|segment| segment.ident.unraw());
    match meta {
        Meta::NameValue(_) if is_named(path, "path") => Some(SourceProblem::PathAttribute),
        Meta::List(list) if is_named(path, "cfg_attr") => {
            cfg_attr_problem(list.tokens.clone(), in_test_code)
        }
        _ if in_test_code => None,
        Meta::List(list) if is_named(path, "cfg") => match classify(list.tokens.clone()) {
            Predicate::Test => Some(SourceProblem::CfgTestNotOnItem),
            Predicate::NamesTest => Some(SourceProblem::TestInOtherForm),
            Predicate::NamesCoverage => Some(SourceProblem::NamesCoverage),
            Predicate::OutsideAllowlist => Some(SourceProblem::CfgOutsideAllowlist),
            Predicate::TestSupport | Predicate::Allowed => None,
        },
        _ if last.as_ref().is_some_and(|word| word == "coverage") => {
            Some(SourceProblem::CoverageAttribute)
        }
        _ if last.as_ref().is_some_and(|word| word == "test") => {
            Some(SourceProblem::TestOutsideTestCode)
        }
        _ => None,
    }
}

/// `path` is the one word `name`. rustc reads `r#cfg` as `cfg`, so a raw identifier is
/// read the same way.
pub fn is_named(path: &Path, name: &str) -> bool {
    let mut segments = path.segments.iter();
    match (segments.next(), segments.next()) {
        (Some(segment), None) => segment.arguments.is_none() && segment.ident.unraw() == name,
        _ => false,
    }
}

/// `cfg_attr(predicate, attribute, …)`: the predicate is judged like a `cfg` one, and each
/// attribute it applies like any other.
fn cfg_attr_problem(arguments: TokenStream, in_test_code: bool) -> Option<SourceProblem> {
    let mut parts = split_at_commas(arguments).into_iter();
    let predicate = parts.next()?;
    let predicate_problem = match classify(predicate.into_iter().collect()) {
        _ if in_test_code => None,
        Predicate::NamesTest => Some(SourceProblem::TestInOtherForm),
        Predicate::NamesCoverage => Some(SourceProblem::NamesCoverage),
        Predicate::OutsideAllowlist => Some(SourceProblem::CfgOutsideAllowlist),
        Predicate::Test | Predicate::TestSupport | Predicate::Allowed => None,
    };
    predicate_problem.or_else(|| {
        parts.find_map(|attribute| {
            let meta = syn::parse2::<Meta>(attribute.into_iter().collect()).ok()?;
            if is_named(meta.path(), "cfg") && !in_test_code {
                return Some(SourceProblem::CfgInCfgAttr);
            }
            problem_of(&meta, in_test_code)
        })
    })
}

fn is_word(tokens: &[TokenTree], word: &str) -> bool {
    matches!(tokens, [TokenTree::Ident(ident)] if ident == word)
}

fn is_test_support(tokens: &[TokenTree]) -> bool {
    let [TokenTree::Ident(any), TokenTree::Group(arguments)] = tokens else {
        return false;
    };
    if any != "any" || arguments.delimiter() != Delimiter::Parenthesis {
        return false;
    }
    match split_at_commas(arguments.stream()).as_slice() {
        [first, second] => {
            (is_word(first, "test") && is_feature(second))
                || (is_feature(first) && is_word(second, "test"))
        }
        _ => false,
    }
}

/// `feature = "…"`.
fn is_feature(tokens: &[TokenTree]) -> bool {
    matches!(tokens,
        [TokenTree::Ident(feature), TokenTree::Punct(equals), TokenTree::Literal(name)]
            if feature == "feature" && equals.as_char() == '=' && name.to_string().starts_with('"'))
}

/// Whether a word of `tokens`, at any depth, matches: every identifier, raw or not, and,
/// when `in_literals`, the text of every literal.
fn mentions(tokens: &[TokenTree], matches: &dyn Fn(&str) -> bool, in_literals: bool) -> bool {
    tokens.iter().any(|token| match token {
        TokenTree::Ident(ident) => {
            let word = ident.to_string();
            matches(word.strip_prefix("r#").unwrap_or(&word))
        }
        TokenTree::Literal(literal) => in_literals && matches(&literal.to_string()),
        TokenTree::Group(group) => {
            let inner: Vec<TokenTree> = group.stream().into_iter().collect();
            mentions(&inner, matches, in_literals)
        }
        TokenTree::Punct(_) => false,
    })
}

/// The parts of `tokens` between commas at its top level; a trailing comma leaves an
/// empty last part.
fn split_at_commas(tokens: TokenStream) -> Vec<Vec<TokenTree>> {
    let mut parts = vec![Vec::new()];
    for token in tokens {
        match &token {
            TokenTree::Punct(comma) if comma.as_char() == ',' => parts.push(Vec::new()),
            _ => {
                if let Some(part) = parts.last_mut() {
                    part.push(token);
                }
            }
        }
    }
    if parts.len() == 1 && parts.first().is_some_and(Vec::is_empty) {
        parts.clear();
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn predicate(text: &str) -> Predicate {
        classify(text.parse().unwrap())
    }

    fn attribute(text: &str) -> Meta {
        syn::parse_str(text).unwrap()
    }

    #[test]
    fn a_cfg_gates_unless_it_is_the_test_mark() {
        for gate in [
            "cfg(unix)",
            "cfg(feature = \"extra\")",
            "cfg(any(test, feature = \"testing\"))",
            "r#cfg(windows)",
        ] {
            assert!(is_cfg_gate(&attribute(gate)), "{gate}");
        }
        for not_a_gate in [
            "cfg(test)",
            "cfg_attr(unix, allow(dead_code))",
            "x::cfg(unix)",
            "doc(cfg(unix))",
            "allow(unused)",
        ] {
            assert!(!is_cfg_gate(&attribute(not_a_gate)), "{not_a_gate}");
        }
    }

    #[test]
    fn exactly_test_marks_test_code() {
        assert_eq!(predicate("test"), Predicate::Test);
        assert_eq!(predicate(" test "), Predicate::Test);
        assert!(is_cfg_test(&attribute("cfg(test)")));
        assert!(is_cfg_test(&attribute("cfg( test /* why */ )")));
        for not_the_mark in [
            "x::cfg(test)",
            "cfg::x(test)",
            "cfg_attr(test)",
            "cfg = test",
        ] {
            assert!(!is_cfg_test(&attribute(not_the_mark)), "{not_the_mark}");
        }
    }

    #[test]
    fn any_test_or_a_feature_is_test_support_either_way_round() {
        for text in [
            "any(test, feature = \"testing\")",
            "any(feature = \"testing\", test)",
            "any( test ,feature=\"testing\" )",
        ] {
            assert_eq!(predicate(text), Predicate::TestSupport, "{text}");
            assert!(!is_cfg_test(&attribute(&format!("cfg({text})"))), "{text}");
        }
    }

    #[test]
    fn test_in_any_other_form_is_named_and_not_a_mark() {
        for text in [
            "not(test)",
            "all(test, unix)",
            "any(test)",
            "all(test)",
            "any(test, unix)",
            "any(test, feature = \"testing\", unix)",
            "any(test, feature = \"testing\",)",
            "any(test, feature = testing)",
            "any(test, test)",
            "r#test",
            "test, unix",
            "all(unix, any(windows, not(test)))",
        ] {
            assert_eq!(predicate(text), Predicate::NamesTest, "{text}");
            assert!(!is_cfg_test(&attribute(&format!("cfg({text})"))), "{text}");
        }
    }

    #[test]
    fn coverage_is_named_wherever_it_appears() {
        for text in [
            "coverage",
            "not(coverage)",
            "coverage_nightly",
            "all(test, not(coverage))",
            "any(test, feature = \"coverage\")",
            "r#coverage",
        ] {
            assert_eq!(predicate(text), Predicate::NamesCoverage, "{text}");
        }
    }

    #[test]
    fn features_unix_windows_and_the_target_keys_are_allowed_with_all_any_and_not() {
        for text in [
            "unix",
            "windows",
            "not(windows)",
            "feature = \"testing\"",
            "feature = \"test-utils\"",
            "target_os = \"linux\"",
            "target_pointer_width = \"64\"",
            "not(target_os = \"linux\")",
            "all(unix, feature = \"testing\")",
            "any(feature = \"a\", all(windows, target_arch = \"x86_64\"))",
            "not(any(windows, target_os = \"macos\"))",
            "all()",
            "any()",
            "debug_assertions",
            "all(unix, debug_assertions)",
            "any(debug_assertions, feature = \"x\")",
        ] {
            assert_eq!(predicate(text), Predicate::Allowed, "{text}");
        }
    }

    #[test]
    fn anything_else_is_outside_the_allowlist() {
        for text in [
            "not(debug_assertions)",
            "not(any(debug_assertions, unix))",
            "shipping",
            "tests",
            "testing",
            "doc",
            "docsrs",
            "miri",
            "panic = \"abort\"",
            "not(feature = \"cheap\")",
            "all(unix, not(feature = \"cheap\"))",
            "not(any(feature = \"a\", unix))",
            "not(unix, windows)",
            "not()",
            "feature = cheap",
            "target_os",
            "r#unix",
            "any(unix,)",
            "if(unix)",
        ] {
            assert_eq!(predicate(text), Predicate::OutsideAllowlist, "{text}");
        }
    }

    fn problem(text: &str) -> Option<SourceProblem> {
        problem_of(&attribute(text), false)
    }

    #[test]
    fn each_attribute_that_would_hide_code_is_a_problem() {
        use SourceProblem::*;
        for (text, expected) in [
            ("cfg(test)", CfgTestNotOnItem),
            ("cfg(not(test))", TestInOtherForm),
            ("cfg(all(test, unix))", TestInOtherForm),
            ("cfg(not(coverage))", NamesCoverage),
            ("coverage(off)", CoverageAttribute),
            ("core::coverage(off)", CoverageAttribute),
            ("cfg_attr(coverage_nightly, coverage(off))", NamesCoverage),
            ("cfg_attr(unix, coverage(off))", CoverageAttribute),
            ("cfg_attr(test, cfg(unix))", CfgInCfgAttr),
            ("cfg_attr(unix, cfg_attr(test, cfg(windows)))", CfgInCfgAttr),
            ("cfg_attr(not(test), allow(dead_code))", TestInOtherForm),
            ("path = \"../../shared/logic.rs\"", PathAttribute),
            ("cfg_attr(unix, path = \"x.rs\")", PathAttribute),
            ("test", TestOutsideTestCode),
            ("tokio::test", TestOutsideTestCode),
            ("cfg_attr(test, test)", TestOutsideTestCode),
            ("cfg(not(debug_assertions))", CfgOutsideAllowlist),
            ("cfg(shipping)", CfgOutsideAllowlist),
            ("cfg(not(feature = \"cheap\"))", CfgOutsideAllowlist),
            (
                "cfg_attr(docsrs, doc(cfg(feature = \"x\")))",
                CfgOutsideAllowlist,
            ),
            (
                "cfg_attr(unix, cfg_attr(shipping, allow(dead_code)))",
                CfgOutsideAllowlist,
            ),
        ] {
            assert_eq!(problem(text), Some(expected), "{text}");
        }
    }

    #[test]
    fn a_raw_identifier_in_an_attribute_path_is_read_as_rustc_reads_it() {
        use SourceProblem::*;
        assert!(is_cfg_test(&attribute("r#cfg(test)")));
        for (text, expected) in [
            ("r#cfg(test)", CfgTestNotOnItem),
            ("r#cfg(not(coverage))", NamesCoverage),
            ("r#cfg_attr(test, cfg(unix))", CfgInCfgAttr),
            ("cfg_attr(unix, r#cfg(test))", CfgInCfgAttr),
            ("r#path = \"x.rs\"", PathAttribute),
            ("r#coverage(off)", CoverageAttribute),
            ("r#test", TestOutsideTestCode),
            ("tokio::r#test", TestOutsideTestCode),
        ] {
            assert_eq!(problem(text), Some(expected), "{text}");
        }
    }

    #[test]
    fn ordinary_attributes_are_no_problem() {
        for text in [
            "cfg(unix)",
            "cfg(any(test, feature = \"testing\"))",
            "cfg_attr(test, derive(Debug))",
            "cfg_attr(any(test, feature = \"testing\"), allow(dead_code))",
            "cfg_attr(feature = \"x\", doc = \"x\")",
            "doc = \"cfg(test)\"",
            "doc(cfg(test))",
            "derive(Debug)",
            "allow(clippy::too_many_lines)",
            "expect(clippy::too_many_lines, reason = \"one long table\")",
            "utoipa::path(get, path = \"/health\")",
            "path(get)",
            "test_case(1)",
            "should_panic",
        ] {
            assert_eq!(problem(text), None, "{text}");
        }
    }

    #[test]
    fn in_test_code_only_a_path_is_a_problem() {
        for text in [
            "cfg(test)",
            "cfg(not(coverage))",
            "coverage(off)",
            "test",
            "cfg_attr(test, cfg(unix))",
            "cfg_attr(not(test), allow(dead_code))",
        ] {
            assert_eq!(problem_of(&attribute(text), true), None, "{text}");
        }
        for text in ["path = \"x.rs\"", "cfg_attr(unix, path = \"x.rs\")"] {
            assert_eq!(
                problem_of(&attribute(text), true),
                Some(SourceProblem::PathAttribute),
                "{text}"
            );
        }
    }
}
