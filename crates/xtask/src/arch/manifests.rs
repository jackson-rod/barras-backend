//! The rules read from the files of the workspace rather than from `cargo metadata`: a
//! crate folder that `members` misses, and a `[patch]` or `[replace]` table.

use std::fs;
use std::path::Path;

use super::{CrateInfo, Violation};
use crate::error::{XtaskError, escaped};
use crate::layout::{crate_folders, is_fixed_folder};
use crate::walk::EntryKind;

/// A crate folder that `members` does not list is never built and never checked.
pub(super) fn check_members(
    root: &Path,
    crates: &[CrateInfo],
) -> Result<Vec<Violation>, XtaskError> {
    let mut violations = Vec::new();
    for (folder, kind) in crate_folders(root)? {
        if !(is_fixed_folder(&folder) || kind == EntryKind::Folder) || folder.to_str().is_none() {
            continue;
        }
        if root.join(&folder).join("Cargo.toml").is_file()
            && !crates.iter().any(|c| c.dir == folder)
        {
            violations.push(Violation {
                krate: escaped(&folder),
                dependency: String::new(),
                rule: "not-a-member",
            });
        }
    }
    Ok(violations)
}

/// line, so the allowlists cannot see them. Needing one is a decision for an ADR.
pub(super) fn check_patches(root: &Path) -> Result<Vec<Violation>, XtaskError> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
