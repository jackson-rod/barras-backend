//! The layout rules for docs, module crates and migrations.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use regex::Regex;

use crate::error::XtaskError;

const README: &str = "README.md";

fn pattern(source: &str) -> Result<Regex, XtaskError> {
    Regex::new(source).map_err(|e| XtaskError::Pattern {
        pattern: source.to_owned(),
        message: e.to_string(),
    })
}

/// Sorted (name, is_directory) pairs, hidden entries left out, directly inside `dir`, so messages come out in a stable order.
pub(crate) fn entries(dir: &Path) -> Result<Vec<(String, bool)>, XtaskError> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir).map_err(XtaskError::io(dir))? {
        let entry = entry.map_err(XtaskError::io(dir))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        // Finder and editors drop hidden files (.DS_Store) into any folder on their own.
        if name.starts_with('.') {
            continue;
        }
        let is_dir = entry
            .file_type()
            .map_err(XtaskError::io(&entry.path()))?
            .is_dir();
        found.push((name, is_dir));
    }
    found.sort();
    Ok(found)
}

fn read(path: &Path) -> Result<String, XtaskError> {
    fs::read_to_string(path).map_err(XtaskError::io(path))
}

pub fn check_adrs(adr_dir: &Path) -> Result<Vec<String>, XtaskError> {
    let name_rule = pattern(r"^(\d{4})-[a-z0-9]+(-[a-z0-9]+)*\.md$")?;
    let status_rule = pattern(r"^Status: (Proposed|Accepted|Superseded by ADR-\d{4})$")?;
    let link_rule = pattern(r"\((\d{4}-[^()\s]+\.md)\)")?;
    let index = read(&adr_dir.join(README))?;
    let mut problems = Vec::new();
    let mut files = Vec::new();
    let mut numbers = BTreeSet::new();
    let mut duplicates = BTreeSet::new();

    for (file, is_dir) in entries(adr_dir)? {
        if is_dir || file == README {
            continue;
        }
        let Some(captures) = name_rule.captures(&file) else {
            problems.push(format!("{file}: name must match NNNN-kebab-case-title.md"));
            continue;
        };
        let number = captures[1].to_owned();
        let text = read(&adr_dir.join(&file))?;
        // A CRLF checkout must not turn every ADR into a violation.
        let lines: Vec<&str> = text.lines().map(|l| l.trim_end_matches('\r')).collect();
        if !lines
            .first()
            .is_some_and(|l| l.starts_with(&format!("# ADR-{number}: ")))
        {
            problems.push(format!(
                "{file}: title line must start with \"# ADR-{number}: \""
            ));
        }
        if !lines.iter().any(|l| status_rule.is_match(l)) {
            problems.push(format!(
                "{file}: status line must be Proposed, Accepted or Superseded by ADR-NNNN"
            ));
        }
        if !index.contains(&format!("({file})")) {
            problems.push(format!("{file} is not listed in {README}"));
        }
        match number.parse::<u32>() {
            Ok(0) => problems.push(format!("{file}: number must start at 0001")),
            Ok(n) => {
                if !numbers.insert(n) {
                    duplicates.insert(n);
                }
            }
            Err(e) => problems.push(format!("{file}: number is not valid: {e}")),
        }
        files.push(file);
    }

    if let Some(&highest) = numbers.last() {
        for missing in (1..highest).filter(|n| !numbers.contains(n)) {
            problems.push(format!("ADR {missing:04} is missing"));
        }
    }
    for number in duplicates {
        problems.push(format!("ADR {number:04} is used by more than one file"));
    }
    for linked in link_rule.captures_iter(&index) {
        if !files.contains(&linked[1].to_owned()) {
            problems.push(format!(
                "{} does not exist, but {README} links it",
                &linked[1]
            ));
        }
    }
    Ok(problems)
}

pub fn check_modules(modules_dir: &Path) -> Result<Vec<String>, XtaskError> {
    let name_rule = pattern(r"^[a-z][a-z0-9]*(_[a-z0-9]+)*$")?;
    let mut problems = Vec::new();
    for (module, is_dir) in entries(modules_dir)? {
        if !is_dir {
            continue;
        }
        let root = modules_dir.join(&module);
        if !name_rule.is_match(&module) {
            problems.push(format!("{module}: name must be snake_case"));
        }
        for required in ["README.md", "src/lib.rs", "src/domain/errors.rs"] {
            if !root.join(required).is_file() {
                problems.push(format!("{module}: {required} is missing"));
            }
        }
        for required in ["src/ports", "src/application"] {
            let dir = root.join(required);
            if !dir.is_dir() {
                problems.push(format!("{module}: {required} is missing"));
            } else if !entries(&dir)?
                .iter()
                .any(|(file, is_dir)| !is_dir && file.ends_with(".rs"))
            {
                // git does not track an empty folder: it would pass here and fail in CI.
                problems.push(format!("{module}: {required} has no .rs file"));
            }
        }
        for (folder, exempt) in [
            ("src/application", &["mod.rs"][..]),
            ("src/domain", &["mod.rs", "errors.rs"][..]),
        ] {
            let dir = root.join(folder);
            if !dir.is_dir() {
                continue;
            }
            for (file, is_dir) in entries(&dir)? {
                if is_dir || !file.ends_with(".rs") || exempt.contains(&file.as_str()) {
                    continue;
                }
                if !read(&dir.join(&file))?.contains("#[cfg(test)]") {
                    problems.push(format!("{module}: {folder}/{file} has no tests"));
                }
            }
        }
    }
    Ok(problems)
}

pub fn check_migrations(migrations_dir: &Path) -> Result<Vec<String>, XtaskError> {
    let name_rule = pattern(r"^\d{14}_[a-z0-9_]+\.sql$")?;
    Ok(entries(migrations_dir)?
        .into_iter()
        .filter(|(name, _)| !name_rule.is_match(name))
        .map(|(name, _)| {
            format!("migrations/{name}: name must match YYYYMMDDHHMMSS_snake_case.sql")
        })
        .collect())
}

pub fn check_structure(repo_root: &Path) -> Result<Vec<String>, XtaskError> {
    let mut problems = Vec::new();
    let adr_dir = repo_root.join("docs/adr");
    if adr_dir.is_dir() {
        problems.extend(check_adrs(&adr_dir)?);
    } else {
        problems.push("docs/adr is missing".to_owned());
    }
    let modules = repo_root.join("crates/modules");
    if modules.is_dir() {
        problems.extend(check_modules(&modules)?);
    }
    let migrations = repo_root.join("migrations");
    if migrations.is_dir() {
        problems.extend(check_migrations(&migrations)?);
    }
    Ok(problems)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;

    fn adr(n: u32, slug: &str) -> String {
        format!("# ADR-{n:04}: {slug}\n\nStatus: Accepted\nDate: 2026-11-15\n")
    }

    fn write(root: &Path, path: &str, text: &str) {
        let full = root.join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, text).unwrap();
    }

    #[test]
    fn a_valid_adr_folder_is_clean() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "0001-a.md", &adr(1, "A"));
        write(dir.path(), "0002-b.md", &adr(2, "B"));
        write(
            dir.path(),
            "README.md",
            "[0001](0001-a.md)\n[0002](0002-b.md)\n",
        );
        assert!(check_adrs(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn adr_violations_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "README.md",
            "[0001](0001-a.md)\n[0009](0009-ghost.md)\n",
        );
        write(dir.path(), "0001-a.md", &adr(1, "A").replace('\n', "\r\n"));
        write(dir.path(), "0003-c.md", &adr(3, "C"));
        write(
            dir.path(),
            "0004-d.md",
            &adr(4, "D").replace("Status: Accepted", "Status: Draft"),
        );
        write(dir.path(), "0005-e.md", "# Something\n\nStatus: Accepted\n");
        write(dir.path(), "Notes.md", "notes\n");
        let found = check_adrs(dir.path()).unwrap().join("\n");
        for expected in [
            "0003-c.md is not listed",
            "0009-ghost.md does not exist",
            "0002 is missing",
            "0004-d.md: status",
            "0005-e.md: title",
            "Notes.md: name",
        ] {
            assert!(found.contains(expected), "{expected}\n{found}");
        }
        assert!(!found.contains("0001-a.md"));
    }

    #[test]
    fn module_violations_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "identity/README.md", "");
        write(root, "identity/src/lib.rs", "");
        write(root, "identity/src/domain/errors.rs", "");
        write(
            root,
            "identity/src/domain/user.rs",
            "#[cfg(test)]\nmod tests {}\n",
        );
        write(root, "identity/src/ports/mod.rs", "");
        write(
            root,
            "identity/src/application/register_user.rs",
            "#[cfg(test)]\nmod tests {}\n",
        );
        write(root, "capital/src/lib.rs", "");
        write(root, "capital/src/ports/mod.rs", "");
        write(
            root,
            "capital/src/application/add_account.rs",
            "fn f() {}\n",
        );
        write(root, "capital/src/domain/mod.rs", "");
        fs::create_dir_all(root.join("BadName")).unwrap();
        let found = check_modules(root).unwrap().join("\n");
        for expected in [
            "capital: README.md",
            "capital: src/domain/errors.rs",
            "capital: src/application/add_account.rs has no tests",
            "BadName: name",
        ] {
            assert!(found.contains(expected), "{expected}\n{found}");
        }
        assert!(!found.contains("identity"));
    }

    #[test]
    fn empty_ports_and_application_folders_are_reported() {
        // git does not track an empty folder, so such a module passes here and fails in CI.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "wallets/README.md", "");
        write(root, "wallets/src/lib.rs", "");
        write(root, "wallets/src/domain/errors.rs", "");
        fs::create_dir_all(root.join("wallets/src/ports")).unwrap();
        fs::create_dir_all(root.join("wallets/src/application")).unwrap();
        let found = check_modules(root).unwrap().join("\n");
        for expected in [
            "wallets: src/ports has no .rs file",
            "wallets: src/application has no .rs file",
        ] {
            assert!(found.contains(expected), "{expected}\n{found}");
        }
    }

    #[test]
    fn migration_names() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "20261115120000_create_users.sql", "");
        write(dir.path(), "create_users.sql", "");
        write(dir.path(), "20261115120000_CreateUsers.sql", "");
        let found = check_migrations(dir.path()).unwrap();
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn missing_optional_folders_are_fine_but_adrs_are_required() {
        let dir = tempfile::tempdir().unwrap();
        let found = check_structure(dir.path()).unwrap().join("\n");
        assert!(found.contains("docs/adr is missing"));
        assert!(!found.contains("modules"));
        assert!(!found.contains("migrations"));
    }

    #[test]
    fn hidden_files_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "docs/adr/README.md", "[0001](0001-a.md)\n");
        write(root, "docs/adr/0001-a.md", &adr(1, "A"));
        write(root, "docs/adr/.DS_Store", "");
        write(root, "crates/modules/identity/README.md", "");
        write(root, "crates/modules/identity/src/lib.rs", "");
        write(root, "crates/modules/identity/src/domain/errors.rs", "");
        write(
            root,
            "crates/modules/identity/src/domain/user.rs",
            "#[cfg(test)]\nmod tests {}\n",
        );
        write(root, "crates/modules/identity/src/ports/mod.rs", "");
        write(
            root,
            "crates/modules/identity/src/application/register_user.rs",
            "#[cfg(test)]\nmod tests {}\n",
        );
        write(root, "crates/modules/.DS_Store", "");
        write(root, "migrations/20261115120000_create_users.sql", "");
        write(root, "migrations/.DS_Store", "");
        assert_eq!(check_structure(dir.path()).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn an_io_error_names_the_path_it_was_reading() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "0001-a.md", &adr(1, "A"));
        let error = check_adrs(dir.path()).unwrap_err().to_string();
        assert!(error.contains("README.md"), "{error}");
    }

    #[test]
    fn adr_numbers_are_unique_and_start_at_one() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "README.md",
            "[0](0000-zero.md)\n[1](0001-a.md)\n[2](0002-b.md)\n[2](0002-c.md)\n",
        );
        write(dir.path(), "0000-zero.md", &adr(0, "Zero"));
        write(dir.path(), "0001-a.md", &adr(1, "A"));
        write(dir.path(), "0002-b.md", &adr(2, "B"));
        write(dir.path(), "0002-c.md", &adr(2, "C"));
        let found = check_adrs(dir.path()).unwrap().join("\n");
        for expected in ["0000-zero.md: number", "0002 is used by more than one file"] {
            assert!(found.contains(expected), "{expected}\n{found}");
        }
    }
}
