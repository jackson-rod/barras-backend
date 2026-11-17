//! The layout rules for docs, module crates, migrations and seeds.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use regex::Regex;

use crate::error::XtaskError;
use crate::walk::{EntryKind, kind_at};

const README: &str = "README.md";

fn pattern(source: &str) -> Result<Regex, XtaskError> {
    Regex::new(source).map_err(|e| XtaskError::Pattern {
        pattern: source.to_owned(),
        message: e.to_string(),
    })
}

/// Sorted (name, kind) pairs, hidden entries left out, directly inside `dir`, so messages come
/// out in a stable order. A symlink is never followed: its kind is `Symlink`, wherever it points.
pub(crate) fn entries(dir: &Path) -> Result<Vec<(String, EntryKind)>, XtaskError> {
    let mut found = Vec::new();
    for entry in fs::read_dir(dir).map_err(XtaskError::io(dir))? {
        let entry = entry.map_err(XtaskError::io(dir))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        // Finder and editors drop hidden files (.DS_Store) into any folder on their own.
        if name.starts_with('.') {
            continue;
        }
        let kind = EntryKind::of(entry.file_type().map_err(XtaskError::io(&entry.path()))?);
        found.push((name, kind));
    }
    found.sort();
    Ok(found)
}

/// What an entry of `kind` is, in a message.
fn described(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::File => "a file",
        EntryKind::Folder => "a folder",
        EntryKind::Symlink => "a symlink",
        EntryKind::Special => "a FIFO, a socket or a device",
    }
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

    for (file, kind) in entries(adr_dir)? {
        if kind == EntryKind::Folder || file == README {
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
    for (module, kind) in entries(modules_dir)? {
        if kind != EntryKind::Folder {
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
                .any(|(file, kind)| *kind != EntryKind::Folder && file.ends_with(".rs"))
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
            for (file, kind) in entries(&dir)? {
                if kind == EntryKind::Folder
                    || !file.ends_with(".rs")
                    || exempt.contains(&file.as_str())
                {
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

/// sqlx keys its history by version alone, so of two migrations that share one, only one is
/// ever applied.
pub fn check_migrations(migrations_dir: &Path) -> Result<Vec<String>, XtaskError> {
    check_versioned_sql(migrations_dir, "migrations")
}

/// Seeds are named like migrations and run in name order, so in the order of their versions:
/// two that share one would run in an order no version decides.
pub fn check_seeds(seeds_dir: &Path) -> Result<Vec<String>, XtaskError> {
    check_versioned_sql(seeds_dir, "seeds")
}

/// `dir` holds SQL files that run in the order of the version their names start with. It must be
/// a folder, each entry of it a regular file named `YYYYMMDDHHMMSS_snake_case.sql`, and no two
/// entries may share a version. An entry is reported for the first rule it breaks, and for that
/// alone: it is no such file, so it shares no version.
fn check_versioned_sql(dir: &Path, folder: &str) -> Result<Vec<String>, XtaskError> {
    // Not followed: sqlx and the seed runner follow a symlink in the folder's place, so the files
    // that run would be another folder's, outside the repository too.
    let kind = EntryKind::of(
        fs::symlink_metadata(dir)
            .map_err(XtaskError::io(dir))?
            .file_type(),
    );
    if kind != EntryKind::Folder {
        return Ok(vec![format!(
            "{folder}: must be a folder, not {}",
            described(kind)
        )]);
    }
    // ASCII digits: `\d` takes a digit of any script, and sqlx reads a version as an i64.
    let name_rule = pattern(r"^([0-9]{14})_[a-z0-9_]+\.sql$")?;
    let mut problems = Vec::new();
    let mut by_version: BTreeMap<String, Vec<String>> = BTreeMap::new();
    // Every name is printed as `{:?}` prints it: a newline or an escape sequence in one would
    // otherwise forge a line of the output or rewrite the terminal.
    for (name, kind) in entries(dir)? {
        let path = format!("{folder}/{name}");
        // sqlx and the seed runner follow a symlink, so the SQL that runs could be any file,
        // outside the repository too; and they skip whatever else is not a file, so the
        // migration or the seed it stands for would never run.
        if kind != EntryKind::File {
            problems.push(format!(
                "{path:?}: must be a regular file, not {}",
                described(kind)
            ));
            continue;
        }
        let Some(version) = name_rule.captures(&name).map(|c| c[1].to_owned()) else {
            problems.push(format!(
                "{path:?}: name must match YYYYMMDDHHMMSS_snake_case.sql"
            ));
            continue;
        };
        by_version.entry(version).or_default().push(name);
    }
    for (version, names) in by_version {
        if names.len() > 1 {
            let names: Vec<String> = names.iter().map(|name| format!("{name:?}")).collect();
            problems.push(format!(
                "{folder}: version {version} is used by more than one file: {}",
                names.join(", ")
            ));
        }
    }
    Ok(problems)
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
    // Whatever stands at either name is checked, not only a folder: `is_dir` would follow a
    // symlink, and pass over a file or a FIFO.
    if kind_at(repo_root, Path::new("migrations"))?.is_some() {
        problems.extend(check_migrations(&repo_root.join("migrations"))?);
    }
    // A missing folder holds no seed, as the seed runner reads it: the first seed comes with the
    // first module that needs local data.
    if kind_at(repo_root, Path::new("seeds"))?.is_some() {
        problems.extend(check_seeds(&repo_root.join("seeds"))?);
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
    fn two_migrations_with_the_same_version_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "20261115120000_create_users.sql", "");
        write(dir.path(), "20261115120000_create_wallets.sql", "");
        write(dir.path(), "20261116090000_create_goals.sql", "");
        // Refused for its name alone: it is no migration, so it shares no version.
        write(dir.path(), "20261115120000_create_users.sql.bak", "");
        assert_eq!(
            check_migrations(dir.path()).unwrap(),
            [
                "\"migrations/20261115120000_create_users.sql.bak\": name must match \
                 YYYYMMDDHHMMSS_snake_case.sql",
                "migrations: version 20261115120000 is used by more than one file: \
                 \"20261115120000_create_users.sql\", \"20261115120000_create_wallets.sql\"",
            ]
        );
    }

    #[test]
    fn seed_names() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "20261116000001_demo_user.sql", "");
        write(dir.path(), "demo_user.sql", "");
        write(dir.path(), "20261116000002_DemoWallets.sql", "");
        write(dir.path(), "README.md", "");
        assert_eq!(
            check_seeds(dir.path()).unwrap(),
            [
                "\"seeds/20261116000002_DemoWallets.sql\": name must match \
                 YYYYMMDDHHMMSS_snake_case.sql",
                "\"seeds/README.md\": name must match YYYYMMDDHHMMSS_snake_case.sql",
                "\"seeds/demo_user.sql\": name must match YYYYMMDDHHMMSS_snake_case.sql",
            ]
        );
    }

    #[test]
    fn two_seeds_with_the_same_version_are_reported() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "20261116000001_demo_user.sql", "");
        write(dir.path(), "20261116000001_demo_wallets.sql", "");
        write(dir.path(), "20261116000001_demo_goals.sql", "");
        write(dir.path(), "20261116000002_demo_budgets.sql", "");
        assert_eq!(
            check_seeds(dir.path()).unwrap(),
            [
                "seeds: version 20261116000001 is used by more than one file: \
                 \"20261116000001_demo_goals.sql\", \"20261116000001_demo_user.sql\", \
                 \"20261116000001_demo_wallets.sql\""
            ]
        );
    }

    #[test]
    fn the_seeds_folder_is_checked_when_it_exists() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "docs/adr/README.md", "[0001](0001-a.md)\n");
        write(root, "docs/adr/0001-a.md", &adr(1, "A"));
        write(root, "seeds/demo_user.sql", "");
        assert_eq!(
            check_structure(root).unwrap(),
            ["\"seeds/demo_user.sql\": name must match YYYYMMDDHHMMSS_snake_case.sql"]
        );
    }

    #[test]
    fn a_version_is_written_in_ascii_digits() {
        // `\d` takes a digit of any script, as these full-width and Arabic-Indic ones; sqlx
        // reads a version as an i64, from ASCII digits alone.
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "２０２６１１１５１２００００_create_users.sql",
            "",
        );
        write(dir.path(), "٢٠٢٦١١١٥١٢٠٠٠٠_create_goals.sql", "");
        write(dir.path(), "20261115120001_create_wallets.sql", "");
        assert_eq!(
            check_migrations(dir.path()).unwrap(),
            [
                "\"migrations/٢٠٢٦١١١٥١٢٠٠٠٠_create_goals.sql\": name must match \
                 YYYYMMDDHHMMSS_snake_case.sql",
                "\"migrations/２０２６１１１５１２００００_create_users.sql\": name must match \
                 YYYYMMDDHHMMSS_snake_case.sql",
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_name_is_printed_escaped_so_that_it_cannot_shape_the_output() {
        // Printed as it is, the newline would start a line of its own, a forged problem, and
        // the escape sequence would turn the terminal red.
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "20261116000001_demo_user.sql\n\u{1b}[31mseeds: all good",
            "",
        );
        let found = check_seeds(dir.path()).unwrap();
        assert_eq!(
            found,
            [
                "\"seeds/20261116000001_demo_user.sql\\n\\u{1b}[31mseeds: all good\": name must \
                 match YYYYMMDDHHMMSS_snake_case.sql"
            ]
        );
        assert!(
            found.iter().all(|line| !line.contains(['\n', '\u{1b}'])),
            "{found:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_entry_that_is_not_a_regular_file_is_refused_for_that_alone() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        write(elsewhere.path(), "outside.sql", "");
        write(dir.path(), "20261115120000_create_users.sql", "");
        // It is no migration, so it shares no version: the regular file above stands alone.
        symlink(
            elsewhere.path().join("outside.sql"),
            dir.path().join("20261115120000_create_users_again.sql"),
        )
        .unwrap();
        symlink(
            elsewhere.path(),
            dir.path().join("20261115120001_linked.sql"),
        )
        .unwrap();
        fs::create_dir(dir.path().join("20261115120002_folder.sql")).unwrap();
        let made = std::process::Command::new("mkfifo")
            .arg(dir.path().join("20261115120003_pipe.sql"))
            .status()
            .unwrap();
        assert!(made.success());
        // Hidden entries stay out of the check, whatever they are.
        symlink(
            elsewhere.path(),
            dir.path().join(".20261115120004_hidden.sql"),
        )
        .unwrap();
        assert_eq!(
            check_migrations(dir.path()).unwrap(),
            [
                "\"migrations/20261115120000_create_users_again.sql\": must be a regular file, \
                 not a symlink",
                "\"migrations/20261115120001_linked.sql\": must be a regular file, not a symlink",
                "\"migrations/20261115120002_folder.sql\": must be a regular file, not a folder",
                "\"migrations/20261115120003_pipe.sql\": must be a regular file, not a FIFO, a \
                 socket or a device",
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_migrations_or_seeds_that_is_not_a_folder_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let elsewhere = tempfile::tempdir().unwrap();
        write(elsewhere.path(), "20261115120000_create_users.sql", "");
        write(root, "docs/adr/README.md", "[0001](0001-a.md)\n");
        write(root, "docs/adr/0001-a.md", &adr(1, "A"));
        std::os::unix::fs::symlink(elsewhere.path(), root.join("migrations")).unwrap();
        write(root, "seeds", "");
        assert_eq!(
            check_structure(root).unwrap(),
            [
                "migrations: must be a folder, not a symlink",
                "seeds: must be a folder, not a file",
            ]
        );
        // Through the link, every entry would pass: the check must not follow it.
        assert_eq!(
            check_migrations(elsewhere.path()).unwrap(),
            Vec::<String>::new()
        );
        fs::remove_file(root.join("seeds")).unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), root.join("seeds")).unwrap();
        assert_eq!(
            check_seeds(&root.join("seeds")).unwrap(),
            ["seeds: must be a folder, not a symlink"]
        );
    }

    #[test]
    fn missing_optional_folders_are_fine_but_adrs_are_required() {
        let dir = tempfile::tempdir().unwrap();
        let found = check_structure(dir.path()).unwrap().join("\n");
        assert!(found.contains("docs/adr is missing"));
        assert!(!found.contains("modules"));
        assert!(!found.contains("migrations"));
        assert!(!found.contains("seeds"));
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
        write(root, "seeds/20261116000001_demo_user.sql", "");
        write(root, "seeds/.DS_Store", "");
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
