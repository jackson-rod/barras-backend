//! `hack-is-a-stub`: the crate cargo-hakari generates may carry no code.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use proc_macro2::{Delimiter, Literal, TokenStream, TokenTree};

use super::{CrateInfo, TargetRole, Violation, violation};
use crate::error::{XtaskError, escaped};
use crate::layout::{Layer, layer_of};
use crate::regular_file::read_regular_file;
use crate::walk::{EntryKind, walk};

/// The hack's files are read only to refuse them when they hold code: a stub is tiny.
const HACK_FILE_LIMIT: u64 = 1024 * 1024;

/// `hack-is-a-stub`. The kernel's and the modules' lists accept `workspace-hack` only
/// because it cannot carry code: one library, at `src/lib.rs`, that holds doc comments
/// at most; at most one build script, at `build.rs`, that holds `fn main() {}` only; and no
/// other `.rs` file in its folder.
pub(super) fn check_hack(root: &Path, crates: &[CrateInfo]) -> Result<Vec<Violation>, XtaskError> {
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
    let entries = walk(folder, Path::new(""), |_| true)?;
    Ok(entries
        .into_iter()
        .filter(|entry| entry.kind != EntryKind::Folder)
        .filter(|entry| {
            entry
                .path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with(".rs"))
        })
        .map(|entry| entry.path)
        .collect())
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

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::arch::Target;

    fn hack_with(targets: &[(TargetRole, &str)]) -> CrateInfo {
        CrateInfo {
            name: "workspace-hack".to_owned(),
            dir: PathBuf::from("crates/workspace-hack"),
            deps: Vec::new(),
            targets: targets
                .iter()
                .map(|(role, src_path)| Target {
                    role: *role,
                    src_path: PathBuf::from(src_path),
                })
                .collect(),
        }
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
}
