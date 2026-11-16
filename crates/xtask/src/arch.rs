//! The layer rule: who may depend on whom, checked against `cargo metadata`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use cargo_metadata::{DependencyKind, MetadataCommand, TargetKind};
use proc_macro2::{Delimiter, Literal, TokenStream, TokenTree};

use crate::error::{XtaskError, escaped};
use crate::regular_file::read_regular_file;
use crate::structure::entries;

/// From crates.io, for the normal and build dependencies of the kernel.
const KERNEL_ALLOWED: &[&str] = &["thiserror", "uuid", "time", "rust_decimal"];
/// From crates.io, for the normal and build dependencies of a module.
const MODULE_ALLOWED: &[&str] = &["thiserror", "uuid", "time", "rust_decimal", "async-trait"];
/// From crates.io, for dev dependencies, on top of the crate's own list.
const DEV_EXTRA: &[&str] = &["tokio", "mockall", "insta"];
/// The members the kernel may depend on, by folder, each only as a path dependency there.
const KERNEL_MEMBERS: &[&str] = &[HACK_DIR];
/// The members a module may depend on, by folder, each only as a path dependency there.
const MODULE_MEMBERS: &[&str] = &[KERNEL_DIR, HACK_DIR];
const KERNEL_DIR: &str = "crates/kernel";
/// The crate cargo-hakari generates. Every member but `xtask` depends on it.
const HACK_DIR: &str = "crates/workspace-hack";
/// The hack's files are read only to refuse them when they hold code: a stub is tiny.
const HACK_FILE_LIMIT: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Kernel,
    Module,
    Adapter,
    App,
    Bootstrap,
    Testkit,
    Tool,
    WorkspaceHack,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepKind {
    Normal,
    Dev,
    Build,
}

/// Where a dependency comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// A path dependency.
    Path,
    CratesIo,
    /// A git repository or a registry other than crates.io.
    Elsewhere,
}

impl Origin {
    /// Classifies the `source` that `cargo metadata` reports, which is absent for a path dependency.
    pub fn of(source: Option<&cargo_metadata::Source>) -> Self {
        match source {
            None => Self::Path,
            Some(source) if source.is_crates_io() => Self::CratesIo,
            Some(_) => Self::Elsewhere,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Dep {
    /// The real package name, never the local rename.
    pub name: String,
    pub kind: DepKind,
    pub origin: Origin,
    /// The folder of a path dependency: relative to the workspace root when it is inside
    /// it, as cargo reports it otherwise.
    pub path: Option<PathBuf>,
}

/// What a target of a package is, as far as `hack-is-a-stub` is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetRole {
    /// A library of the plain `lib` crate type.
    Library,
    BuildScript,
    /// A binary, an example, a test, a bench, or a library of another crate type.
    Other,
}

impl TargetRole {
    pub fn of(kinds: &[TargetKind]) -> Self {
        match kinds {
            [TargetKind::Lib] => Self::Library,
            [TargetKind::CustomBuild] => Self::BuildScript,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Target {
    pub role: TargetRole,
    /// The target's main file: relative to the workspace root when it is inside it.
    pub src_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct CrateInfo {
    pub name: String,
    /// Relative to the workspace root.
    pub dir: PathBuf,
    pub deps: Vec<Dep>,
    pub targets: Vec<Target>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// The package at fault, or the folder for a rule about a folder.
    pub krate: String,
    /// The dependency at fault; for a rule about the crate itself, what in it is at fault.
    pub dependency: String,
    pub rule: &'static str,
}

pub fn layer_of(dir: &Path) -> Option<Layer> {
    let parts: Vec<&str> = dir.iter().filter_map(|p| p.to_str()).collect();
    match parts.as_slice() {
        ["crates", "kernel"] => Some(Layer::Kernel),
        ["crates", "modules", _] => Some(Layer::Module),
        ["crates", "adapters", _] => Some(Layer::Adapter),
        ["apps", _] => Some(Layer::App),
        ["crates", "bootstrap"] => Some(Layer::Bootstrap),
        ["crates", "testkit"] => Some(Layer::Testkit),
        ["crates", "xtask"] => Some(Layer::Tool),
        ["crates", "workspace-hack"] => Some(Layer::WorkspaceHack),
        _ => None,
    }
}

/// The package name the crate in the known folder `dir` must have. The cargo-deny bans
/// name crates, so a name that drifted from its folder would slip them.
pub fn expected_name(dir: &Path) -> Option<String> {
    let last = dir.file_name()?.to_str()?;
    Some(match layer_of(dir)? {
        Layer::Kernel => "barras-kernel".to_owned(),
        Layer::Module | Layer::App => format!("barras-{last}"),
        Layer::Adapter => format!("barras-adapter-{last}"),
        Layer::Bootstrap => "barras-bootstrap".to_owned(),
        Layer::Testkit => "barras-testkit".to_owned(),
        Layer::Tool => "xtask".to_owned(),
        Layer::WorkspaceHack => "workspace-hack".to_owned(),
    })
}

/// Violations come in input order of the crates, then in order of their dependencies.
pub fn check_arch(crates: &[CrateInfo]) -> Vec<Violation> {
    let mut violations = Vec::new();
    for krate in crates {
        match expected_name(&krate.dir) {
            None => violations.push(violation(krate, "", "unknown-location")),
            Some(expected) if expected != krate.name => violations.push(Violation {
                krate: escaped(&krate.dir),
                dependency: krate.name.clone(),
                rule: "package-name",
            }),
            Some(_) => {}
        }
        let layer = layer_of(&krate.dir);
        for dep in &krate.deps {
            for rule in broken_rules(crates, layer, dep) {
                violations.push(violation(krate, &dep.name, rule));
            }
        }
    }
    violations
}

/// The rules that `dep`, a dependency of a crate in `layer`, breaks. A dependency on a
/// member is known by the member it resolves to, found by package name and judged by its
/// folder, so renaming a package cannot slip a rule.
fn broken_rules(crates: &[CrateInfo], layer: Option<Layer>, dep: &Dep) -> Vec<&'static str> {
    let member = crates.iter().find(|c| c.name == dep.name);
    let member_layer = member.and_then(|m| layer_of(&m.dir));
    let mut broken = Vec::new();
    broken.extend(layer_rule(layer, dep, member));
    // A crate outside the members is never checked, so it could pass any member on.
    if dep.origin == Origin::Path
        && !crates
            .iter()
            .any(|c| dep.path.as_deref() == Some(c.dir.as_path()))
    {
        broken.push("path-outside-members");
    }
    if member_layer == Some(Layer::App) {
        broken.push("apps-are-roots");
    }
    if member_layer == Some(Layer::Testkit) && dep.kind != DepKind::Dev {
        broken.push("testkit-dev-only");
    }
    if member_layer == Some(Layer::Bootstrap) && layer != Some(Layer::App) {
        broken.push("bootstrap-apps-only");
    }
    broken
}

/// The rule of the crate's own layer that `dep` breaks, if any. `member` is the member
/// `dep` resolves to.
fn layer_rule(layer: Option<Layer>, dep: &Dep, member: Option<&CrateInfo>) -> Option<&'static str> {
    let dev = dep.kind == DepKind::Dev;
    let (rule, fits) = match layer? {
        Layer::Kernel if dev => (
            "kernel-dev-allowlist",
            allowed(&[KERNEL_ALLOWED, DEV_EXTRA], KERNEL_MEMBERS, dep, member),
        ),
        Layer::Kernel => (
            "kernel-allowlist",
            allowed(&[KERNEL_ALLOWED], KERNEL_MEMBERS, dep, member),
        ),
        Layer::Module if dev => (
            "module-dev-allowlist",
            allowed(&[MODULE_ALLOWED, DEV_EXTRA], MODULE_MEMBERS, dep, member),
        ),
        Layer::Module => (
            "module-allowlist",
            allowed(&[MODULE_ALLOWED], MODULE_MEMBERS, dep, member),
        ),
        Layer::Adapter => (
            "adapter-isolation",
            member.and_then(|m| layer_of(&m.dir)) != Some(Layer::Adapter),
        ),
        Layer::Bootstrap => (
            "bootstrap-isolation",
            member.is_none_or(|m| is_path_at(dep, m, HACK_DIR)),
        ),
        Layer::Tool => ("tool-isolation", member.is_none()),
        Layer::WorkspaceHack => ("hack-isolation", dep.origin == Origin::CratesIo),
        Layer::App | Layer::Testkit => return None,
    };
    (!fits).then_some(rule)
}

/// A dependency on a member counts only when that member's folder is in `members` and the
/// dependency takes it by path from there: a copy kept in any other folder, or taken from
/// git or a registry, has the same name and is other code. Any other name counts only from
/// crates.io, or a `path` or `git` entry, or another registry, could pass a foreign crate
/// off under an allowed name. `[patch]` and `[replace]` are stopped by `check_patches`.
fn allowed(lists: &[&[&str]], members: &[&str], dep: &Dep, member: Option<&CrateInfo>) -> bool {
    match member {
        Some(member) => members.iter().any(|folder| is_path_at(dep, member, folder)),
        None => {
            dep.origin == Origin::CratesIo
                && lists.iter().any(|list| list.contains(&dep.name.as_str()))
        }
    }
}

/// `member`, which `dep` resolves to, sits at `folder`, and `dep` takes it by path from there.
fn is_path_at(dep: &Dep, member: &CrateInfo, folder: &str) -> bool {
    let folder = Path::new(folder);
    member.dir == folder && dep.origin == Origin::Path && dep.path.as_deref() == Some(folder)
}

fn violation(krate: &CrateInfo, dependency: &str, rule: &'static str) -> Violation {
    Violation {
        krate: krate.name.clone(),
        dependency: dependency.to_owned(),
        rule,
    }
}

/// Every rule of `check-arch` over the workspace at `root`.
pub fn check_workspace(root: &Path) -> Result<Vec<Violation>, XtaskError> {
    let crates = load_workspace(root)?;
    let mut violations = check_arch(&crates);
    violations.extend(check_hack(root, &crates)?);
    violations.extend(check_members(root, &crates)?);
    violations.extend(check_patches(root)?);
    Ok(violations)
}

/// `hack-is-a-stub`. The kernel's and the modules' lists accept `workspace-hack` only
/// because it cannot carry code: one library, at `src/lib.rs`, that holds doc comments
/// at most; at most one build script, at `build.rs`, that holds `fn main() {}` only; and no
/// other `.rs` file in its folder.
fn check_hack(root: &Path, crates: &[CrateInfo]) -> Result<Vec<Violation>, XtaskError> {
    let Some(hack) = crates
        .iter()
        .find(|c| layer_of(&c.dir) == Some(Layer::WorkspaceHack))
    else {
        return Ok(Vec::new());
    };
    let mut offenders: BTreeSet<PathBuf> = misplaced_targets(hack).into_iter().collect();
    let folder = root.join(&hack.dir);
    for file in rust_files(&folder)? {
        let read = || read_regular_file(&folder.join(&file), HACK_FILE_LIMIT);
        let fits = if file == Path::new("src/lib.rs") {
            holds_only_doc_comments(&read()?)
        } else if file == Path::new("build.rs") {
            is_an_empty_main(&read()?)
        } else {
            false
        };
        if !fits {
            offenders.insert(hack.dir.join(file));
        }
    }
    Ok(offenders
        .iter()
        .map(|offender| violation(hack, &escaped(offender), "hack-is-a-stub"))
        .collect())
}

/// The main file of each target of `hack` that is neither its library at `src/lib.rs` nor
/// its build script at `build.rs`, and `src/lib.rs` itself when it has no library.
fn misplaced_targets(hack: &CrateInfo) -> Vec<PathBuf> {
    let library = hack.dir.join("src/lib.rs");
    let build_script = hack.dir.join("build.rs");
    let mut misplaced: Vec<PathBuf> = hack
        .targets
        .iter()
        .filter(|target| match target.role {
            TargetRole::Library => target.src_path != library,
            TargetRole::BuildScript => target.src_path != build_script,
            TargetRole::Other => true,
        })
        .map(|target| target.src_path.clone())
        .collect();
    if !hack
        .targets
        .iter()
        .any(|target| target.role == TargetRole::Library)
    {
        misplaced.push(library);
    }
    misplaced
}

/// Every entry under `folder` whose name ends in `.rs`, relative to it. A symlink is listed
/// and never followed, so it cannot lead the walk out of the folder.
fn rust_files(folder: &Path) -> Result<Vec<PathBuf>, XtaskError> {
    let mut found = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(relative) = pending.pop() {
        let dir = folder.join(&relative);
        for entry in fs::read_dir(&dir).map_err(XtaskError::io(&dir))? {
            let entry = entry.map_err(XtaskError::io(&dir))?;
            let path = relative.join(entry.file_name());
            if entry
                .file_type()
                .map_err(XtaskError::io(&entry.path()))?
                .is_dir()
            {
                pending.push(path);
            } else if entry.file_name().to_string_lossy().ends_with(".rs") {
                found.push(path);
            }
        }
    }
    Ok(found)
}

/// Doc comments at most. proc-macro2 hands each one over as `#[doc = "…"]` or
/// `#![doc = "…"]`; a plain comment is no token at all.
fn holds_only_doc_comments(source: &str) -> bool {
    let Ok(tokens) = source.parse::<TokenStream>() else {
        return false;
    };
    let mut tokens = tokens.into_iter().peekable();
    while let Some(token) = tokens.next() {
        if !matches!(&token, TokenTree::Punct(pound) if pound.as_char() == '#') {
            return false;
        }
        if matches!(tokens.peek(), Some(TokenTree::Punct(bang)) if bang.as_char() == '!') {
            tokens.next();
        }
        let Some(TokenTree::Group(attribute)) = tokens.next() else {
            return false;
        };
        let inside: Vec<TokenTree> = attribute.stream().into_iter().collect();
        let is_doc = matches!(
            inside.as_slice(),
            [TokenTree::Ident(doc), TokenTree::Punct(equals), TokenTree::Literal(text)]
                if doc == "doc" && equals.as_char() == '=' && is_a_string(text)
        );
        if attribute.delimiter() != Delimiter::Bracket || !is_doc {
            return false;
        }
    }
    true
}

fn is_a_string(literal: &Literal) -> bool {
    let text = literal.to_string();
    text.starts_with('"') || text.starts_with("r\"") || text.starts_with("r#")
}

/// Exactly `fn main() {}`, comments aside.
fn is_an_empty_main(source: &str) -> bool {
    let Ok(tokens) = source.parse::<TokenStream>() else {
        return false;
    };
    let tokens: Vec<TokenTree> = tokens.into_iter().collect();
    matches!(
        tokens.as_slice(),
        [TokenTree::Ident(keyword), TokenTree::Ident(name), TokenTree::Group(parameters), TokenTree::Group(body)]
            if keyword == "fn"
                && name == "main"
                && parameters.delimiter() == Delimiter::Parenthesis
                && parameters.stream().is_empty()
                && body.delimiter() == Delimiter::Brace
                && body.stream().is_empty()
    )
}

/// A crate folder that `members` does not list is never built and never checked.
fn check_members(root: &Path, crates: &[CrateInfo]) -> Result<Vec<Violation>, XtaskError> {
    let mut folders = Vec::new();
    for parent in ["apps", "crates/modules", "crates/adapters"] {
        let dir = root.join(parent);
        if !dir.is_dir() {
            continue;
        }
        for (name, is_dir) in entries(&dir)? {
            if is_dir {
                folders.push(format!("{parent}/{name}"));
            }
        }
    }
    folders.extend(
        [
            "crates/bootstrap",
            "crates/kernel",
            "crates/testkit",
            "crates/workspace-hack",
            "crates/xtask",
        ]
        .map(String::from),
    );
    Ok(folders
        .into_iter()
        .filter(|folder| root.join(folder).join("Cargo.toml").is_file())
        .filter(|folder| !crates.iter().any(|c| c.dir.as_path() == Path::new(folder)))
        .map(|folder| Violation {
            krate: escaped(Path::new(&folder)),
            dependency: String::new(),
            rule: "not-a-member",
        })
        .collect())
}

/// `[patch]` and `[replace]` swap a crate for other code without touching any dependency
/// line, so the allowlists cannot see them. Needing one is a decision for an ADR.
fn check_patches(root: &Path) -> Result<Vec<Violation>, XtaskError> {
    let mut violations = Vec::new();
    for (file, tables) in [
        ("Cargo.toml", &["patch", "replace"][..]),
        (".cargo/config.toml", &["patch"][..]),
    ] {
        let path = root.join(file);
        if !path.is_file() {
            continue;
        }
        let text = fs::read_to_string(&path).map_err(XtaskError::io(&path))?;
        for found in top_level_tables(&text, tables) {
            violations.push(Violation {
                krate: file.to_owned(),
                dependency: found,
                rule: "no-patch",
            });
        }
    }
    Ok(violations)
}

/// The tables among `names` that a TOML text defines at its top level, as written: a
/// `[name]` or `[name.x]` header, or a `name` or `name.x` key before the first header.
/// A line scan rather than a TOML parser, so the check needs no new dependency.
fn top_level_tables(toml: &str, names: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    let mut before_first_header = true;
    for line in toml.lines().map(str::trim) {
        let (key, shown) = if let Some(header) = line.strip_prefix('[') {
            before_first_header = false;
            let end = line.find(']').map_or(line.len(), |i| i + 1);
            (
                header.trim_start_matches('['),
                line.get(..end).unwrap_or(line),
            )
        } else if before_first_header {
            (line, line.split('=').next().unwrap_or(line).trim_end())
        } else {
            continue;
        };
        if names.contains(&first_key(key)) {
            found.push(shown.to_owned());
        }
    }
    found
}

/// The first part of a dotted TOML key, unquoted: `patch` for `patch.crates-io`,
/// `"patch".crates-io` or `patch . crates-io`.
fn first_key(key: &str) -> &str {
    let key = key.trim_start();
    for quote in ['"', '\''] {
        if let Some(rest) = key.strip_prefix(quote) {
            return rest.split(quote).next().unwrap_or(rest);
        }
    }
    key.split(['.', ']', '=', ' ', '\t']).next().unwrap_or(key)
}

pub fn load_workspace(root: &Path) -> Result<Vec<CrateInfo>, XtaskError> {
    let metadata = MetadataCommand::new()
        .current_dir(root)
        .manifest_path(root.join("Cargo.toml"))
        .no_deps()
        .exec()
        .map_err(|e| XtaskError::Metadata(e.to_string()))?;
    // Taken from the metadata, not from `root`: cargo may report a canonicalised path.
    let workspace_root = metadata.workspace_root.as_std_path();
    // Cargo spells a path like the root and normalises it, so a plain prefix strip is
    // enough. A path outside the root keeps its whole spelling and so equals no member's.
    let relative = |path: &Path| {
        path.strip_prefix(workspace_root)
            .unwrap_or(path)
            .to_path_buf()
    };
    let mut crates = Vec::new();
    for package in &metadata.packages {
        let manifest_dir = package
            .manifest_path
            .as_std_path()
            .parent()
            .ok_or_else(|| {
                XtaskError::Metadata(format!("manifest of {} has no directory", package.name))
            })?;
        let dir = manifest_dir
            .strip_prefix(workspace_root)
            .map_err(|e| XtaskError::Metadata(format!("{}: {e}", package.name)))?
            .to_path_buf();
        let deps = package
            .dependencies
            .iter()
            .map(|d| Dep {
                name: d.name.clone(),
                kind: match d.kind {
                    DependencyKind::Development => DepKind::Dev,
                    DependencyKind::Build => DepKind::Build,
                    _ => DepKind::Normal,
                },
                origin: Origin::of(d.source.as_ref()),
                path: d.path.as_ref().map(|path| relative(path.as_std_path())),
            })
            .collect();
        let targets = package
            .targets
            .iter()
            .map(|target| Target {
                role: TargetRole::of(&target.kind),
                src_path: relative(target.src_path.as_std_path()),
            })
            .collect();
        crates.push(CrateInfo {
            name: package.name.to_string(),
            dir,
            deps,
            targets,
        });
    }
    Ok(crates)
}

pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[cfg(test)]
mod tests {
    use super::DepKind::{Build, Dev, Normal};
    use super::*;

    const CRATES_IO: &str = "registry+https://github.com/rust-lang/crates.io-index";
    const GIT: &str = "git+https://github.com/example/fork?branch=main#0123456789abcdef";
    const OTHER_REGISTRY: &str = "sparse+https://registry.example.com/index/";
    /// Stands for the folder of the member of that name until `workspace` resolves it.
    const MEMBER_FOLDER: &str = "<the member's folder>";

    /// `true` marks a path dependency at the folder of the member of that name, `false` one
    /// from crates.io.
    fn krate(name: &str, dir: &str, deps: &[(&str, DepKind, bool)]) -> CrateInfo {
        let sources: Vec<(&str, DepKind, Option<&str>)> = deps
            .iter()
            .map(|(name, kind, is_path)| (*name, *kind, (!is_path).then_some(CRATES_IO)))
            .collect();
        krate_from(name, dir, &sources)
    }

    /// Each dependency carries the `source` that `cargo metadata` would report: `None` for
    /// a path, which is then the folder of the member of that name.
    fn krate_from(name: &str, dir: &str, deps: &[(&str, DepKind, Option<&str>)]) -> CrateInfo {
        CrateInfo {
            name: name.to_owned(),
            dir: PathBuf::from(dir),
            deps: deps
                .iter()
                .map(|(name, kind, source)| Dep {
                    name: (*name).to_owned(),
                    kind: *kind,
                    origin: Origin::of(
                        source
                            .map(|repr| cargo_metadata::Source {
                                repr: repr.to_owned(),
                            })
                            .as_ref(),
                    ),
                    path: source.is_none().then(|| PathBuf::from(MEMBER_FOLDER)),
                })
                .collect(),
            targets: Vec::new(),
        }
    }

    /// Path dependencies only, each from the folder given.
    fn krate_at(name: &str, dir: &str, deps: &[(&str, DepKind, &str)]) -> CrateInfo {
        CrateInfo {
            name: name.to_owned(),
            dir: PathBuf::from(dir),
            deps: deps
                .iter()
                .map(|(name, kind, path)| Dep {
                    name: (*name).to_owned(),
                    kind: *kind,
                    origin: Origin::Path,
                    path: Some(PathBuf::from(path)),
                })
                .collect(),
            targets: Vec::new(),
        }
    }

    /// The crates as `load_workspace` reads them: a path dependency on a member's name
    /// points at that member's folder, and one on any other name at a folder outside.
    fn workspace<const N: usize>(crates: [CrateInfo; N]) -> Vec<CrateInfo> {
        let folders: Vec<(String, PathBuf)> = crates
            .iter()
            .map(|c| (c.name.clone(), c.dir.clone()))
            .collect();
        crates
            .into_iter()
            .map(|mut krate| {
                for dep in &mut krate.deps {
                    if dep.path.as_deref() == Some(Path::new(MEMBER_FOLDER)) {
                        dep.path = Some(
                            folders
                                .iter()
                                .find(|(name, _)| *name == dep.name)
                                .map_or_else(
                                    || Path::new("/outside").join(&dep.name),
                                    |(_, dir)| dir.clone(),
                                ),
                        );
                    }
                }
                krate
            })
            .collect()
    }

    fn rules(violations: &[Violation]) -> Vec<(&str, &str, &str)> {
        violations
            .iter()
            .map(|v| (v.krate.as_str(), v.dependency.as_str(), v.rule))
            .collect()
    }

    #[test]
    fn the_empty_skeleton_is_clean() {
        let crates = workspace([
            krate(
                "barras-kernel",
                "crates/kernel",
                &[("thiserror", Normal, false)],
            ),
            krate(
                "barras-identity",
                "crates/modules/identity",
                &[
                    ("barras-kernel", Normal, true),
                    ("async-trait", Normal, false),
                    ("tokio", Dev, false),
                ],
            ),
            krate(
                "barras-adapter-postgres",
                "crates/adapters/postgres",
                &[("barras-identity", Normal, true), ("sqlx", Normal, false)],
            ),
            krate(
                "barras-api",
                "apps/api",
                &[
                    ("barras-adapter-postgres", Normal, true),
                    ("barras-testkit", Dev, true),
                ],
            ),
            krate(
                "barras-testkit",
                "crates/testkit",
                &[("sqlx", Normal, false)],
            ),
            krate("xtask", "crates/xtask", &[("regex", Normal, false)]),
        ]);
        assert_eq!(check_arch(&crates), []);
    }

    #[test]
    fn a_module_cannot_reach_a_framework_or_another_module() {
        let crates = workspace([
            krate("barras-capital", "crates/modules/capital", &[]),
            krate(
                "barras-identity",
                "crates/modules/identity",
                &[
                    ("sqlx", Normal, false),
                    ("axum", Normal, false),
                    ("barras-capital", Normal, true),
                    ("sqlx", Dev, false),
                ],
            ),
        ]);
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("barras-identity", "sqlx", "module-allowlist"),
                ("barras-identity", "axum", "module-allowlist"),
                ("barras-identity", "barras-capital", "module-allowlist"),
                ("barras-identity", "sqlx", "module-dev-allowlist"),
            ]
        );
    }

    #[test]
    fn the_other_rules() {
        let crates = workspace([
            krate(
                "barras-kernel",
                "crates/kernel",
                &[("serde", Normal, false)],
            ),
            krate("barras-adapter-postgres", "crates/adapters/postgres", &[]),
            krate(
                "barras-adapter-http",
                "crates/adapters/http",
                &[("barras-adapter-postgres", Normal, true)],
            ),
            krate(
                "barras-api",
                "apps/api",
                &[("barras-testkit", Normal, true)],
            ),
            krate("barras-testkit", "crates/testkit", &[]),
            krate("xtask", "crates/xtask", &[("barras-kernel", Normal, true)]),
            krate("barras-misc", "crates/misc", &[]),
        ]);
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("barras-kernel", "serde", "kernel-allowlist"),
                (
                    "barras-adapter-http",
                    "barras-adapter-postgres",
                    "adapter-isolation"
                ),
                ("barras-api", "barras-testkit", "testkit-dev-only"),
                ("xtask", "barras-kernel", "tool-isolation"),
                ("barras-misc", "", "unknown-location"),
            ]
        );
    }

    #[test]
    fn each_known_folder_holds_the_package_of_its_name() {
        let named = [
            ("barras-kernel", "crates/kernel"),
            ("barras-identity", "crates/modules/identity"),
            ("barras-adapter-http", "crates/adapters/http"),
            ("barras-api", "apps/api"),
            ("barras-bootstrap", "crates/bootstrap"),
            ("barras-testkit", "crates/testkit"),
            ("workspace-hack", "crates/workspace-hack"),
            ("xtask", "crates/xtask"),
        ];
        let crates = named.map(|(name, dir)| krate(name, dir, &[]));
        assert_eq!(check_arch(&crates), []);
        let misnamed = [
            ("barras-core", "crates/kernel"),
            ("identity", "crates/modules/identity"),
            ("barras-http", "crates/adapters/http"),
            ("api", "apps/api"),
            ("barras-startup", "crates/bootstrap"),
            ("barras-fixtures", "crates/testkit"),
            ("hack", "crates/workspace-hack"),
            ("barras-xtask", "crates/xtask"),
        ];
        let crates = misnamed.map(|(name, dir)| krate(name, dir, &[]));
        assert_eq!(
            rules(&check_arch(&crates)),
            misnamed.map(|(name, dir)| (dir, name, "package-name"))
        );
    }

    #[test]
    fn a_path_dependency_outside_the_members_is_a_violation_in_any_crate_and_kind() {
        let mut crates = workspace([
            krate("barras-kernel", "crates/kernel", &[]),
            krate_at(
                "barras-api",
                "apps/api",
                &[
                    ("shim", Normal, "vendor/shim"),
                    ("shim", Dev, "/outside/vendor/shim"),
                    ("barras-kernel", Build, "crates/kernel"),
                ],
            ),
            krate_at(
                "barras-testkit",
                "crates/testkit",
                &[("shim", Build, "vendor/shim")],
            ),
        ]);
        // A path dependency whose folder cargo did not report is outside every member too.
        crates[1].deps.push(Dep {
            name: "shim".to_owned(),
            kind: Normal,
            origin: Origin::Path,
            path: None,
        });
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("barras-api", "shim", "path-outside-members"),
                ("barras-api", "shim", "path-outside-members"),
                ("barras-api", "shim", "path-outside-members"),
                ("barras-testkit", "shim", "path-outside-members"),
            ]
        );
    }

    #[test]
    fn no_crate_may_depend_on_an_app_in_any_kind() {
        let crates = workspace([
            krate("barras-api", "apps/api", &[("barras-ingest", Dev, true)]),
            krate("barras-ingest", "apps/ingest", &[]),
            krate(
                "barras-adapter-http",
                "crates/adapters/http",
                &[("barras-api", Normal, true)],
            ),
            krate(
                "barras-testkit",
                "crates/testkit",
                &[("barras-ingest", Build, true)],
            ),
            krate_from(
                "barras-bootstrap",
                "crates/bootstrap",
                &[("barras-api", Normal, Some(GIT))],
            ),
        ]);
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("barras-api", "barras-ingest", "apps-are-roots"),
                ("barras-adapter-http", "barras-api", "apps-are-roots"),
                ("barras-testkit", "barras-ingest", "apps-are-roots"),
                ("barras-bootstrap", "barras-api", "bootstrap-isolation"),
                ("barras-bootstrap", "barras-api", "apps-are-roots"),
            ]
        );
    }

    #[test]
    fn a_renamed_member_is_judged_by_its_folder() {
        let crates = workspace([
            krate("barras-core", "crates/kernel", &[]),
            krate("barras-startup", "crates/bootstrap", &[]),
            krate("barras-fixtures", "crates/testkit", &[]),
            krate("barras-db", "crates/adapters/postgres", &[]),
            krate(
                "barras-identity",
                "crates/modules/identity",
                &[("barras-core", Normal, true)],
            ),
            krate(
                "barras-adapter-http",
                "crates/adapters/http",
                &[
                    ("barras-startup", Normal, true),
                    ("barras-db", Normal, true),
                ],
            ),
            krate(
                "barras-api",
                "apps/api",
                &[
                    ("barras-startup", Normal, true),
                    ("barras-fixtures", Normal, true),
                ],
            ),
        ]);
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("crates/kernel", "barras-core", "package-name"),
                ("crates/bootstrap", "barras-startup", "package-name"),
                ("crates/testkit", "barras-fixtures", "package-name"),
                ("crates/adapters/postgres", "barras-db", "package-name"),
                (
                    "barras-adapter-http",
                    "barras-startup",
                    "bootstrap-apps-only"
                ),
                ("barras-adapter-http", "barras-db", "adapter-isolation"),
                ("barras-api", "barras-fixtures", "testkit-dev-only"),
            ]
        );
    }

    #[test]
    fn an_allowed_name_from_the_wrong_source_is_a_violation() {
        let crates = workspace([
            krate("barras-kernel", "crates/kernel", &[("uuid", Normal, true)]),
            krate(
                "barras-identity",
                "crates/modules/identity",
                &[
                    ("barras-kernel", Normal, false),
                    ("time", Normal, true),
                    ("tokio", Dev, true),
                ],
            ),
        ]);
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("barras-kernel", "uuid", "kernel-allowlist"),
                ("barras-kernel", "uuid", "path-outside-members"),
                ("barras-identity", "barras-kernel", "module-allowlist"),
                ("barras-identity", "time", "module-allowlist"),
                ("barras-identity", "time", "path-outside-members"),
                ("barras-identity", "tokio", "module-dev-allowlist"),
                ("barras-identity", "tokio", "path-outside-members"),
            ]
        );
    }

    #[test]
    fn an_allowed_name_counts_only_from_crates_io() {
        let crates = workspace([
            krate_from(
                "barras-kernel",
                "crates/kernel",
                &[("thiserror", Normal, Some(GIT))],
            ),
            krate_from(
                "barras-identity",
                "crates/modules/identity",
                &[
                    ("uuid", Normal, Some(GIT)),
                    ("time", Normal, Some(OTHER_REGISTRY)),
                    ("workspace-hack", Normal, Some(CRATES_IO)),
                ],
            ),
        ]);
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("barras-kernel", "thiserror", "kernel-allowlist"),
                ("barras-identity", "uuid", "module-allowlist"),
                ("barras-identity", "time", "module-allowlist"),
                ("barras-identity", "workspace-hack", "module-allowlist"),
            ]
        );
    }

    #[test]
    fn a_workspace_crate_is_known_by_its_name_whatever_its_source() {
        let crates = workspace([
            krate_from("barras-kernel", "crates/kernel", &[]),
            krate_from("barras-adapter-postgres", "crates/adapters/postgres", &[]),
            krate_from(
                "barras-adapter-http",
                "crates/adapters/http",
                &[("barras-adapter-postgres", Normal, Some(GIT))],
            ),
            krate_from(
                "xtask",
                "crates/xtask",
                &[("barras-kernel", Normal, Some(GIT))],
            ),
        ]);
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                (
                    "barras-adapter-http",
                    "barras-adapter-postgres",
                    "adapter-isolation"
                ),
                ("xtask", "barras-kernel", "tool-isolation"),
            ]
        );
    }

    #[test]
    fn the_kernel_counts_only_from_its_own_folder() {
        let crates = workspace([
            krate("barras-kernel", "crates/kernel", &[]),
            krate_at(
                "barras-identity",
                "crates/modules/identity",
                &[("barras-kernel", Normal, "crates/kernel")],
            ),
            krate_at(
                "barras-goals",
                "crates/modules/goals",
                &[
                    ("barras-kernel", Normal, "vendor/kernel"),
                    ("barras-kernel", Build, "crates/kernel/copy"),
                    ("barras-kernel", Dev, "/outside/crates/kernel"),
                ],
            ),
        ]);
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("barras-goals", "barras-kernel", "module-allowlist"),
                ("barras-goals", "barras-kernel", "path-outside-members"),
                ("barras-goals", "barras-kernel", "module-allowlist"),
                ("barras-goals", "barras-kernel", "path-outside-members"),
                ("barras-goals", "barras-kernel", "module-dev-allowlist"),
                ("barras-goals", "barras-kernel", "path-outside-members"),
            ]
        );
    }

    #[test]
    fn a_path_dependency_without_its_folder_never_counts() {
        let mut crates = workspace([
            krate("barras-kernel", "crates/kernel", &[]),
            krate_at(
                "barras-identity",
                "crates/modules/identity",
                &[("barras-kernel", Normal, "crates/kernel")],
            ),
        ]);
        crates[1].deps[0].path = None;
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("barras-identity", "barras-kernel", "module-allowlist"),
                ("barras-identity", "barras-kernel", "path-outside-members"),
            ]
        );
    }

    #[test]
    fn the_hack_is_allowed_in_the_kernel_and_the_modules_from_its_own_folder() {
        let crates = workspace([
            krate("workspace-hack", "crates/workspace-hack", &[]),
            krate_at(
                "barras-kernel",
                "crates/kernel",
                &[
                    ("workspace-hack", Normal, "crates/workspace-hack"),
                    ("workspace-hack", Dev, "crates/workspace-hack"),
                ],
            ),
            krate_at(
                "barras-identity",
                "crates/modules/identity",
                &[
                    ("workspace-hack", Normal, "crates/workspace-hack"),
                    ("workspace-hack", Build, "crates/workspace-hack"),
                    ("workspace-hack", Dev, "crates/workspace-hack"),
                ],
            ),
        ]);
        assert_eq!(check_arch(&crates), []);
    }

    #[test]
    fn the_hack_counts_only_from_its_own_folder() {
        let crates = workspace([
            krate("workspace-hack", "crates/workspace-hack", &[]),
            krate_from(
                "barras-kernel",
                "crates/kernel",
                &[
                    ("workspace-hack", Normal, Some(CRATES_IO)),
                    ("workspace-hack", Dev, Some(GIT)),
                ],
            ),
            krate_at(
                "barras-identity",
                "crates/modules/identity",
                &[("workspace-hack", Normal, "vendor/workspace-hack")],
            ),
        ]);
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("barras-kernel", "workspace-hack", "kernel-allowlist"),
                ("barras-kernel", "workspace-hack", "kernel-dev-allowlist"),
                ("barras-identity", "workspace-hack", "module-allowlist"),
                ("barras-identity", "workspace-hack", "path-outside-members"),
            ]
        );
    }

    #[test]
    fn bootstrap_and_the_hack_are_known_folders() {
        assert_eq!(
            layer_of(Path::new("crates/bootstrap")),
            Some(Layer::Bootstrap)
        );
        assert_eq!(
            layer_of(Path::new("crates/workspace-hack")),
            Some(Layer::WorkspaceHack)
        );
        let crates = workspace([
            krate(
                "barras-bootstrap",
                "crates/bootstrap",
                &[("tokio", Normal, false), ("workspace-hack", Normal, true)],
            ),
            krate(
                "workspace-hack",
                "crates/workspace-hack",
                &[("tokio", Normal, false), ("axum", Normal, false)],
            ),
            krate("barras-nested", "crates/bootstrap/nested", &[]),
            krate("barras-hacks", "crates/workspace-hacks", &[]),
        ]);
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("barras-nested", "", "unknown-location"),
                ("barras-hacks", "", "unknown-location"),
            ]
        );
    }

    #[test]
    fn bootstrap_depends_on_no_workspace_crate_but_the_hack() {
        let crates = workspace([
            krate("barras-kernel", "crates/kernel", &[]),
            krate("barras-adapter-http", "crates/adapters/http", &[]),
            krate("workspace-hack", "crates/workspace-hack", &[]),
            krate_from(
                "barras-bootstrap",
                "crates/bootstrap",
                &[
                    ("tracing", Normal, Some(CRATES_IO)),
                    ("workspace-hack", Normal, None),
                    ("barras-kernel", Normal, None),
                    ("barras-adapter-http", Dev, None),
                    ("barras-kernel", Build, Some(GIT)),
                ],
            ),
        ]);
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("barras-bootstrap", "barras-kernel", "bootstrap-isolation"),
                (
                    "barras-bootstrap",
                    "barras-adapter-http",
                    "bootstrap-isolation"
                ),
                ("barras-bootstrap", "barras-kernel", "bootstrap-isolation"),
            ]
        );
    }

    #[test]
    fn bootstrap_takes_the_hack_only_from_its_own_folder() {
        let hack = krate("workspace-hack", "crates/workspace-hack", &[]);
        let from_another_folder = krate_at(
            "barras-bootstrap",
            "crates/bootstrap",
            &[("workspace-hack", Normal, "vendor/workspace-hack")],
        );
        assert_eq!(
            rules(&check_arch(&workspace([hack.clone(), from_another_folder]))),
            [
                ("barras-bootstrap", "workspace-hack", "bootstrap-isolation"),
                ("barras-bootstrap", "workspace-hack", "path-outside-members"),
            ]
        );
        let from_git = krate_from(
            "barras-bootstrap",
            "crates/bootstrap",
            &[("workspace-hack", Normal, Some(GIT))],
        );
        assert_eq!(
            rules(&check_arch(&workspace([hack, from_git]))),
            [("barras-bootstrap", "workspace-hack", "bootstrap-isolation")]
        );
    }

    #[test]
    fn only_an_app_may_depend_on_bootstrap() {
        let crates = workspace([
            krate("barras-bootstrap", "crates/bootstrap", &[]),
            krate(
                "barras-api",
                "apps/api",
                &[("barras-bootstrap", Normal, true)],
            ),
            krate(
                "barras-ingest",
                "apps/ingest",
                &[("barras-bootstrap", Dev, true)],
            ),
            krate(
                "barras-adapter-http",
                "crates/adapters/http",
                &[("barras-bootstrap", Normal, true)],
            ),
            krate(
                "barras-testkit",
                "crates/testkit",
                &[("barras-bootstrap", Dev, true)],
            ),
            krate_from(
                "workspace-hack",
                "crates/workspace-hack",
                &[("barras-bootstrap", Build, Some(GIT))],
            ),
        ]);
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                (
                    "barras-adapter-http",
                    "barras-bootstrap",
                    "bootstrap-apps-only"
                ),
                ("barras-testkit", "barras-bootstrap", "bootstrap-apps-only"),
                ("workspace-hack", "barras-bootstrap", "hack-isolation"),
                ("workspace-hack", "barras-bootstrap", "bootstrap-apps-only"),
            ]
        );
    }

    #[test]
    fn bootstrap_in_a_crate_with_its_own_rule_breaks_both() {
        let crates = workspace([
            krate("barras-bootstrap", "crates/bootstrap", &[]),
            krate(
                "barras-kernel",
                "crates/kernel",
                &[("barras-bootstrap", Dev, true)],
            ),
            krate(
                "barras-identity",
                "crates/modules/identity",
                &[("barras-bootstrap", Normal, true)],
            ),
            krate(
                "xtask",
                "crates/xtask",
                &[("barras-bootstrap", Normal, true)],
            ),
        ]);
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("barras-kernel", "barras-bootstrap", "kernel-dev-allowlist"),
                ("barras-kernel", "barras-bootstrap", "bootstrap-apps-only"),
                ("barras-identity", "barras-bootstrap", "module-allowlist"),
                ("barras-identity", "barras-bootstrap", "bootstrap-apps-only"),
                ("xtask", "barras-bootstrap", "tool-isolation"),
                ("xtask", "barras-bootstrap", "bootstrap-apps-only"),
            ]
        );
    }

    #[test]
    fn the_tool_still_may_not_depend_on_the_hack() {
        let crates = workspace([
            krate("workspace-hack", "crates/workspace-hack", &[]),
            krate_at(
                "xtask",
                "crates/xtask",
                &[("workspace-hack", Normal, "crates/workspace-hack")],
            ),
        ]);
        assert_eq!(
            rules(&check_arch(&crates)),
            [("xtask", "workspace-hack", "tool-isolation")]
        );
    }

    #[test]
    fn every_dependency_of_the_hack_comes_from_crates_io() {
        let crates = workspace([
            krate("barras-adapter-http", "crates/adapters/http", &[]),
            krate_from(
                "workspace-hack",
                "crates/workspace-hack",
                &[
                    ("tokio", Normal, Some(CRATES_IO)),
                    ("cc", Build, Some(CRATES_IO)),
                    ("barras-adapter-http", Normal, None),
                    ("regex", Build, Some(GIT)),
                    ("serde", Dev, Some(OTHER_REGISTRY)),
                ],
            ),
        ]);
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("workspace-hack", "barras-adapter-http", "hack-isolation"),
                ("workspace-hack", "regex", "hack-isolation"),
                ("workspace-hack", "serde", "hack-isolation"),
            ]
        );
    }

    fn hack_with(targets: &[(TargetRole, &str)]) -> CrateInfo {
        let mut hack = krate("workspace-hack", "crates/workspace-hack", &[]);
        hack.targets = targets
            .iter()
            .map(|(role, src_path)| Target {
                role: *role,
                src_path: PathBuf::from(src_path),
            })
            .collect();
        hack
    }

    #[test]
    fn the_hack_has_one_library_at_src_lib_rs_and_at_most_a_build_script_at_build_rs() {
        use TargetRole::{BuildScript, Library, Other};
        let library = (Library, "crates/workspace-hack/src/lib.rs");
        let build_script = (BuildScript, "crates/workspace-hack/build.rs");
        assert_eq!(
            misplaced_targets(&hack_with(&[library])),
            [] as [PathBuf; 0]
        );
        assert_eq!(
            misplaced_targets(&hack_with(&[library, build_script])),
            [] as [PathBuf; 0]
        );
        for (targets, misplaced) in [
            (
                &[(Library, "crates/workspace-hack/src/other.rs")][..],
                &["crates/workspace-hack/src/other.rs"][..],
            ),
            (
                &[library, (Other, "crates/workspace-hack/src/main.rs")],
                &["crates/workspace-hack/src/main.rs"],
            ),
            (
                &[library, (BuildScript, "crates/workspace-hack/gen.rs")],
                &["crates/workspace-hack/gen.rs"],
            ),
            (
                &[(Other, "crates/workspace-hack/src/lib.rs")],
                &[
                    "crates/workspace-hack/src/lib.rs",
                    "crates/workspace-hack/src/lib.rs",
                ],
            ),
            (&[], &["crates/workspace-hack/src/lib.rs"]),
        ] {
            assert_eq!(
                misplaced_targets(&hack_with(targets)),
                misplaced.iter().map(PathBuf::from).collect::<Vec<_>>(),
                "{targets:?}"
            );
        }
    }

    #[test]
    fn a_target_is_a_library_only_when_its_one_kind_is_lib() {
        assert_eq!(TargetRole::of(&[TargetKind::Lib]), TargetRole::Library);
        assert_eq!(
            TargetRole::of(&[TargetKind::CustomBuild]),
            TargetRole::BuildScript
        );
        for kinds in [
            &[TargetKind::Bin][..],
            &[TargetKind::ProcMacro],
            &[TargetKind::RLib, TargetKind::CDyLib],
            &[TargetKind::Lib, TargetKind::CDyLib],
            &[],
        ] {
            assert_eq!(TargetRole::of(kinds), TargetRole::Other, "{kinds:?}");
        }
    }

    #[test]
    fn the_hack_library_may_hold_doc_comments_and_nothing_else() {
        for stub in [
            "",
            "// This is a stub lib.rs.\n",
            "//! The crate.\n/// More.\n/** A block. */\n/*! Inner. */\n#![doc = \"x\"]\n#[doc = r\"y\"]\n",
        ] {
            assert!(holds_only_doc_comments(stub), "{stub}");
        }
        for code in [
            "pub use regex;\n",
            "// This is a stub lib.rs.\npub fn business_rule() {}\n",
            "#![no_std]\n",
            "#![allow(unused)]\n",
            "#![doc = include_str!(\"../README.md\")]\n",
            "/// A doc comment on an item.\nfn f() {}\n",
            "macro_rules! m { () => {} }\n",
            "#",
            "#!",
            "#![doc = b\"bytes\"]\n",
            "{",
        ] {
            assert!(!holds_only_doc_comments(code), "{code}");
        }
    }

    #[test]
    fn the_hack_build_script_holds_an_empty_main_and_nothing_else() {
        for empty in [
            "fn main() {}",
            "// A build script is required for cargo to consider build dependencies.\nfn main() {}\n",
            "fn main ( ) { /* nothing */ }",
        ] {
            assert!(is_an_empty_main(empty), "{empty}");
        }
        for code in [
            "",
            "fn main() { println!(\"cargo::rustc-cfg=hacked\"); }",
            "fn main() {}\nfn other() {}",
            "pub fn main() {}",
            "fn main(arguments: u8) {}",
            "#[allow(unused)]\nfn main() {}",
            "//! A doc comment.\nfn main() {}",
            "fn main() -> () {}",
            "fn main[] {}",
            "{",
        ] {
            assert!(!is_an_empty_main(code), "{code}");
        }
    }

    #[test]
    fn every_rs_file_of_the_hack_folder_is_found_and_a_symlink_is_not_followed() {
        let folder = tempfile::tempdir().unwrap();
        for file in [
            "build.rs",
            "src/lib.rs",
            "src/deep/er/extra.rs",
            ".hidden.rs",
        ] {
            let path = folder.path().join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "").unwrap();
        }
        fs::write(folder.path().join("Cargo.toml"), "").unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        fs::write(elsewhere.path().join("never.rs"), "").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(elsewhere.path(), folder.path().join("src/linked")).unwrap();
            std::os::unix::fs::symlink(
                elsewhere.path().join("never.rs"),
                folder.path().join("src/link.rs"),
            )
            .unwrap();
        }
        let mut found = rust_files(folder.path()).unwrap();
        found.sort();
        let mut expected = vec![
            ".hidden.rs",
            "build.rs",
            "src/deep/er/extra.rs",
            "src/lib.rs",
        ];
        if cfg!(unix) {
            expected.push("src/link.rs");
        }
        expected.sort_unstable();
        assert_eq!(
            found,
            expected.iter().map(PathBuf::from).collect::<Vec<_>>()
        );
    }

    #[test]
    fn patch_and_replace_tables_are_found_in_each_toml_form() {
        let manifest = "patch.crates-io.uuid = { path = \"x\" }\n\n[package]\nname = \"x\"\n[patches]\n[workspace.patch]\n[ patch . \"https://github.com/example/fork\" ] # a fork\n[replace]\n\"uuid:1.0.0\" = { path = \"y\" }\n";
        assert_eq!(
            top_level_tables(manifest, &["patch", "replace"]),
            [
                "patch.crates-io.uuid",
                "[ patch . \"https://github.com/example/fork\" ]",
                "[replace]",
            ]
        );
    }

    #[test]
    fn build_and_dev_dependencies_follow_the_rules_too() {
        let crates = workspace([
            krate(
                "barras-kernel",
                "crates/kernel",
                &[
                    ("cc", Build, false),
                    ("sqlx", Dev, false),
                    ("insta", Dev, false),
                ],
            ),
            krate(
                "barras-identity",
                "crates/modules/identity",
                &[("cc", Build, false)],
            ),
            krate("barras-api", "apps/api", &[("barras-testkit", Build, true)]),
            krate("barras-testkit", "crates/testkit", &[]),
        ]);
        assert_eq!(
            rules(&check_arch(&crates)),
            [
                ("barras-kernel", "cc", "kernel-allowlist"),
                ("barras-kernel", "sqlx", "kernel-dev-allowlist"),
                ("barras-identity", "cc", "module-allowlist"),
                ("barras-api", "barras-testkit", "testkit-dev-only"),
            ]
        );
    }
}
