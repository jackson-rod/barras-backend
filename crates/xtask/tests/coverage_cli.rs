#![allow(clippy::unwrap_used, clippy::expect_used)] // integration test crate: every item is test code

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// How many lines of every fixture file sit in its test module, all of them run. Counted
/// as production lines, they would lift a kernel at 94% to its bar.
const TEST_LINES: usize = 20;

/// A line of a fixture source and what a test run over it reports.
#[derive(Clone, Copy)]
enum Line<'a> {
    /// Production code a test ran.
    Run(&'a str),
    /// Production code no test ran.
    Missed(&'a str),
    /// Test code, which a test run always runs.
    Test(&'a str),
    /// A line without code, which the report does not name.
    Bare(&'a str),
}

use Line::{Bare, Missed, Run, Test};

/// `run` production lines a test ran, then `missed` ones no test ran.
fn production(run: usize, missed: usize) -> Vec<Line<'static>> {
    let mut lines = vec![Run("pub fn run() {}"); run];
    lines.extend(vec![Missed("pub fn missed() {}"); missed]);
    lines
}

/// The body of a test module: `TEST_LINES` lines, every one run.
fn test_body() -> Vec<Line<'static>> {
    vec![Test("    fn test() {}"); TEST_LINES]
}

/// A temporary workspace and the LCOV report of a test run over it.
struct Workspace {
    dir: tempfile::TempDir,
    lcov: String,
}

impl Workspace {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
            lcov: String::new(),
        }
    }

    /// The real path of a file of the workspace, as cargo-llvm-cov writes it.
    fn path(&self, relative: &str) -> PathBuf {
        self.dir.path().canonicalize().unwrap().join(relative)
    }

    /// Writes a file, and the manifest of the crate around it when it is in a crate folder.
    fn write(&self, relative: &str, text: &str) -> PathBuf {
        let path = self.path(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        if let Some((folder, _)) = relative.split_once("/src/") {
            let manifest = self.path(folder).join("Cargo.toml");
            if !manifest.exists() {
                fs::write(manifest, "[package]\nname = \"fixture\"\n").unwrap();
            }
        }
        path
    }

    /// Adds a source file of these lines, and the record a test run over it writes.
    fn lines(self, relative: &str, lines: &[Line]) -> Self {
        let text: Vec<&str> = lines
            .iter()
            .map(|line| match line {
                Run(text) | Missed(text) | Test(text) | Bare(text) => *text,
            })
            .collect();
        let path = self.write(relative, &(text.join("\n") + "\n"));
        let hits: Vec<(usize, u8)> = lines
            .iter()
            .enumerate()
            .filter_map(|(index, line)| match line {
                Run(_) | Test(_) => Some((index + 1, 1)),
                Missed(_) => Some((index + 1, 0)),
                Bare(_) => None,
            })
            .collect();
        self.record(&path, &hits)
    }

    /// Adds a source file with `found` production lines, the first `hit` of them run, and
    /// an inline test module after them.
    fn file(self, relative: &str, hit: usize, found: usize) -> Self {
        let mut lines = production(hit, found - hit);
        lines.extend([Bare("#[cfg(test)]"), Bare("mod tests {")]);
        lines.extend(test_body());
        lines.push(Bare("}"));
        self.lines(relative, &lines)
    }

    /// Adds one record. The sums are wrong on purpose: the lines are counted, never taken
    /// from them.
    fn record(mut self, file: &Path, hits: &[(usize, u8)]) -> Self {
        self.lcov.push_str(&format!("SF:{}\n", file.display()));
        for (line, hit) in hits {
            self.lcov.push_str(&format!("DA:{line},{hit}\n"));
        }
        self.lcov.push_str("LF:0\nLH:0\nend_of_record\n");
        self
    }

    fn report(&self) -> PathBuf {
        let path = self.dir.path().join("lcov.info");
        fs::write(&path, &self.lcov).unwrap();
        path
    }

    fn check(&self) -> Checked {
        check(&[&self.report(), self.dir.path()])
    }
}

struct Checked {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn check(args: &[&Path]) -> Checked {
    checked(
        Command::new(env!("CARGO_BIN_EXE_xtask"))
            .arg("check-coverage")
            .args(args)
            .output()
            .unwrap(),
    )
}

fn checked(output: Output) -> Checked {
    Checked {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// A kernel at 90% of its production lines, the rest of the workspace at 100%.
fn kernel_at_90_percent_with(shape: &[Line]) -> Checked {
    let mut kernel = production(90, 10);
    kernel.push(Bare(""));
    kernel.extend_from_slice(shape);
    Workspace::new()
        .lines("crates/kernel/src/lib.rs", &kernel)
        .file("apps/api/src/main.rs", 100, 100)
        .check()
}

fn assert_the_kernel_reads_90_percent_and_fails(run: &Checked, shape: &str) {
    assert_eq!(run.code, Some(1), "{shape}: {}", run.stdout);
    assert!(
        run.stdout.contains("crates/kernel: 90.0% (90/100)\n"),
        "{shape}: {}{}",
        run.stdout,
        run.stderr
    );
}

#[test]
fn a_report_at_exactly_both_bars_passes_and_prints_every_figure() {
    let run = Workspace::new()
        .file("crates/kernel/src/money.rs", 95, 100)
        .file("apps/api/src/main.rs", 75, 100)
        .check();
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(
        run.stdout,
        "apps/api: 75.0% (75/100)\ncrates/kernel: 95.0% (95/100)\ntotal: 85.0% (170/200)\n"
    );
    assert_eq!(run.stderr, "");
}

#[test]
fn a_kernel_at_94_percent_fails_and_is_named() {
    let run = Workspace::new()
        .file("crates/kernel/src/money.rs", 94, 100)
        .file("apps/api/src/main.rs", 100, 100)
        .check();
    assert_eq!(run.code, Some(1), "{}", run.stdout);
    assert!(
        run.stderr.starts_with("crates/kernel: 94.0% (94/100)"),
        "{}",
        run.stderr
    );
    assert!(!run.stderr.contains("total"), "{}", run.stderr);
    assert!(
        run.stdout.contains("total: 97.0% (194/200)"),
        "{}",
        run.stdout
    );
}

#[test]
fn a_module_under_its_bar_fails_and_is_named() {
    let run = Workspace::new()
        .file("crates/kernel/src/money.rs", 100, 100)
        .file("crates/modules/goals/src/domain/goal.rs", 94, 100)
        .check();
    assert_eq!(run.code, Some(1), "{}", run.stdout);
    assert!(
        run.stderr
            .starts_with("crates/modules/goals: 94.0% (94/100)"),
        "{}",
        run.stderr
    );
}

#[test]
fn a_total_at_84_percent_fails() {
    let run = Workspace::new()
        .file("crates/kernel/src/money.rs", 100, 100)
        .file("apps/api/src/main.rs", 68, 100)
        .check();
    assert_eq!(run.code, Some(1), "{}", run.stdout);
    assert!(
        run.stderr.starts_with("total: 84.0% (168/200)"),
        "{}",
        run.stderr
    );
    assert!(!run.stderr.contains("crates/kernel"), "{}", run.stderr);
}

#[test]
fn a_test_module_is_left_out_whatever_sits_between_its_attribute_and_its_name() {
    for (shape, lines) in [
        (
            "a wrapped attribute",
            &[
                Bare("#[cfg(test)]"),
                Bare("#[expect("),
                Bare("    clippy::too_many_lines,"),
                Bare("    reason = \"one long table\""),
                Bare(")]"),
                Bare("mod tests {"),
            ][..],
        ),
        (
            "a plain comment",
            &[
                Bare("#[cfg(test)]"),
                Bare("// the fixtures are long"),
                Bare("mod tests {"),
            ],
        ),
        (
            "doc comments",
            &[
                Bare("/// Tests."),
                Bare("#[cfg(test)]"),
                Bare("/// More."),
                Bare("mod tests {"),
            ],
        ),
        (
            "a module of another name and visibility",
            &[Bare("#[cfg(test)]"), Bare("pub(crate) mod fakes {")],
        ),
    ] {
        let mut module = lines.to_vec();
        module.extend(test_body());
        module.push(Bare("}"));
        let run = kernel_at_90_percent_with(&module);
        assert_the_kernel_reads_90_percent_and_fails(&run, shape);
    }
}

#[test]
fn a_brace_at_column_0_in_a_raw_string_does_not_end_a_test_module() {
    let mut module = vec![
        Bare("#[cfg(test)]"),
        Bare("mod tests {"),
        Test("    const BODY: &str = r#\""),
        Test("{"),
        Test("  \"amount\": 1"),
        Test("}"),
        Test("\"#;"),
    ];
    module.extend(test_body());
    module.push(Bare("}"));
    let run = kernel_at_90_percent_with(&module);
    assert_the_kernel_reads_90_percent_and_fails(&run, "a raw string");
}

#[test]
fn production_code_after_a_test_module_or_under_a_commented_attribute_counts() {
    let mut after_a_test_module = vec![Bare("#[cfg(test)]"), Bare("mod tests {")];
    after_a_test_module.extend(test_body());
    after_a_test_module.extend([Bare("} // tests"), Bare(""), Bare("mod ledger {")]);
    after_a_test_module.extend(vec![Missed("    pub fn missed() {}"); 10]);
    after_a_test_module.push(Bare("}"));
    let mut under_a_comment = vec![
        Bare("/* Pricing rules. Kept apart from the rest:"),
        Bare("#[cfg(test)]"),
        Bare("#[doc(hidden)] */"),
        Bare("mod pricing {"),
    ];
    under_a_comment.extend(vec![Missed("    pub fn missed() {}"); 10]);
    under_a_comment.push(Bare("}"));
    for (shape, lines) in [
        ("a module after the tests", after_a_test_module),
        ("a module under a comment", under_a_comment),
    ] {
        let mut kernel = production(90, 0);
        kernel.push(Bare(""));
        kernel.extend(lines);
        let run = Workspace::new()
            .lines("crates/kernel/src/lib.rs", &kernel)
            .file("apps/api/src/main.rs", 100, 100)
            .check();
        assert_the_kernel_reads_90_percent_and_fails(&run, shape);
    }
}

#[test]
fn any_item_that_carries_cfg_test_is_left_out_at_any_depth() {
    let wrap = |opening: &[Line<'static>], closing: &[Line<'static>]| {
        let mut lines = opening.to_vec();
        lines.extend(test_body());
        lines.extend_from_slice(closing);
        lines
    };
    for (shape, lines) in [
        (
            "a function",
            wrap(&[Bare("#[cfg(test)]"), Bare("fn helper() {")], &[Bare("}")]),
        ),
        (
            "an impl",
            wrap(&[Bare("#[cfg(test)]"), Bare("impl Probe {")], &[Bare("}")]),
        ),
        (
            "a method",
            wrap(
                &[
                    Bare("impl Probe {"),
                    Bare("    #[cfg(test)]"),
                    Bare("    fn fake() {"),
                ],
                &[Bare("    }"), Bare("}")],
            ),
        ),
        (
            "a nested module",
            wrap(
                &[
                    Bare("mod outer {"),
                    Bare("    #[cfg(test)]"),
                    Bare("    mod tests {"),
                ],
                &[Bare("    }"), Bare("}")],
            ),
        ),
    ] {
        let run = kernel_at_90_percent_with(&lines);
        assert_the_kernel_reads_90_percent_and_fails(&run, shape);
    }
}

#[test]
fn every_shape_that_would_hide_code_from_the_measure_is_an_error_that_names_file_and_line() {
    for (shape, line) in [
        ("#[cfg(test)]\nmod unit;", 2),
        ("mod ab_tests;", 1),
        ("#[cfg(not(coverage))]\npub fn hidden() {}", 1),
        ("#[coverage(off)]\npub fn hidden() {}", 1),
        (
            "#[cfg_attr(coverage_nightly, coverage(off))]\npub fn hidden() {}",
            1,
        ),
        ("#[cfg(not(test))]\npub fn real() {}", 1),
        ("#[cfg(all(test, unix))]\nmod tests {}", 1),
        ("pub fn f() {\n    #[cfg(test)]\n    let x = 1;\n}", 2),
        ("pub struct S {\n    #[cfg(test)]\n    field: u8,\n}", 2),
        ("#[path = \"../../shared/logic.rs\"]\nmod logic;", 1),
        ("#[cfg_attr(test, cfg(unix))]\npub fn f() {}", 1),
    ] {
        let workspace = Workspace::new().file("apps/api/src/main.rs", 100, 100);
        let path = workspace.write("crates/kernel/src/lib.rs", &format!("{shape}\n"));
        let run = workspace.record(&path, &[(1, 1)]).check();
        assert_eq!(run.code, Some(1), "{shape}: {}", run.stdout);
        assert!(
            run.stderr
                .contains(&format!("crates/kernel/src/lib.rs:{line}: ")),
            "{shape}: {}",
            run.stderr
        );
        assert_eq!(run.stdout, "", "{shape}");
    }
}

#[test]
fn a_file_or_a_folder_that_cargo_llvm_cov_leaves_out_is_an_error() {
    for (file, named) in [
        (
            "crates/kernel/src/ab_tests.rs",
            "crates/kernel/src/ab_tests.rs",
        ),
        (
            "crates/kernel/src/x-tests.rs",
            "crates/kernel/src/x-tests.rs",
        ),
        (
            "crates/kernel/src/examples/mod.rs",
            "crates/kernel/src/examples",
        ),
        (
            "crates/kernel/src/benches/mod.rs",
            "crates/kernel/src/benches",
        ),
    ] {
        let workspace = Workspace::new()
            .file("crates/kernel/src/lib.rs", 100, 100)
            .file("apps/api/src/main.rs", 100, 100);
        workspace.write(file, "pub fn hidden() {}\n");
        let run = workspace.check();
        assert_eq!(run.code, Some(1), "{file}: {}", run.stdout);
        assert!(run.stderr.contains(named), "{file}: {}", run.stderr);
    }
}

#[test]
fn an_out_of_line_test_module_named_tests_and_test_support_are_fine() {
    let kernel = [
        production(95, 5),
        vec![
            Bare("#[cfg(test)]"),
            Bare("mod tests;"),
            Bare("#[cfg(any(test, feature = \"testing\"))]"),
            Bare("mod fixed {"),
            Missed("    pub fn set() {}"),
            Bare("}"),
        ],
    ]
    .concat();
    let workspace = Workspace::new()
        .lines("crates/kernel/src/lib.rs", &kernel)
        .file("apps/api/src/main.rs", 100, 100);
    // cargo-llvm-cov leaves `tests.rs` out of the report, and check-coverage reads it only
    // for `include` and `#[path]`.
    workspace.write(
        "crates/kernel/src/tests.rs",
        "#[cfg(not(coverage))]\nfn helper() {}\n",
    );
    let run = workspace.check();
    assert_eq!(run.code, Some(1), "{}", run.stderr);
    assert!(
        run.stdout.contains("crates/kernel: 94.0% (95/101)\n"),
        "{}{}",
        run.stdout,
        run.stderr
    );
}

#[test]
fn a_file_inside_the_workspace_but_outside_every_crate_is_an_error() {
    let workspace = Workspace::new()
        .file("crates/kernel/src/lib.rs", 100, 100)
        .file("apps/api/src/main.rs", 100, 100);
    let shared = workspace.write("shared/logic.rs", "pub fn never_tested() {}\n");
    let run = workspace.record(&shared, &[(1, 0)]).check();
    assert_eq!(run.code, Some(1), "{}", run.stdout);
    assert!(run.stderr.contains("shared/logic.rs"), "{}", run.stderr);
}

#[test]
fn the_kernel_and_every_module_must_be_in_the_report() {
    let absent = Workspace::new().file("apps/api/src/main.rs", 100, 100);
    absent.write("crates/kernel/src/lib.rs", "pub fn untested() {}\n");
    let run = absent.check();
    assert_eq!(run.code, Some(1), "{}", run.stdout);
    assert!(run.stderr.contains("crates/kernel"), "{}", run.stderr);

    let without_production = Workspace::new()
        .lines(
            "crates/kernel/src/lib.rs",
            &[
                Bare("#[cfg(test)]"),
                Bare("mod tests {"),
                Test("    fn t() {}"),
                Bare("}"),
            ],
        )
        .file("apps/api/src/main.rs", 100, 100);
    let run = without_production.check();
    assert_eq!(run.code, Some(1), "{}", run.stdout);
    assert!(run.stderr.contains("crates/kernel"), "{}", run.stderr);

    let module_absent = Workspace::new()
        .file("crates/kernel/src/lib.rs", 100, 100)
        .file("apps/api/src/main.rs", 100, 100);
    module_absent.write("crates/modules/goals/src/lib.rs", "pub fn untested() {}\n");
    let run = module_absent.check();
    assert_eq!(run.code, Some(1), "{}", run.stdout);
    assert!(
        run.stderr.contains("crates/modules/goals"),
        "{}",
        run.stderr
    );
}

#[test]
fn tests_and_benches_of_a_crate_are_not_measured() {
    let mut workspace = Workspace::new()
        .lines("crates/kernel/src/lib.rs", &production(90, 10))
        .file("apps/api/src/main.rs", 100, 100);
    for file in ["crates/kernel/tests/x.rs", "crates/kernel/benches/x.rs"] {
        let lines = vec!["fn run() {}"; 100].join("\n") + "\n";
        let path = workspace.write(file, &lines);
        let hits: Vec<(usize, u8)> = (1..=100).map(|line| (line, 1)).collect();
        workspace = workspace.record(&path, &hits);
    }
    let run = workspace.check();
    assert_the_kernel_reads_90_percent_and_fails(&run, "tests and benches");
}

#[cfg(unix)]
#[test]
fn a_missing_source_file_is_an_error_under_either_spelling_of_the_workspace() {
    let real = Workspace::new()
        .file("crates/kernel/src/lib.rs", 100, 100)
        .file("apps/api/src/main.rs", 100, 100);
    let links = tempfile::tempdir().unwrap();
    let link = links.path().join("workspace");
    std::os::unix::fs::symlink(real.dir.path(), &link).unwrap();
    for gone in [
        real.path("crates/kernel/src/gone.rs"),
        link.join("crates/kernel/src/gone.rs"),
    ] {
        let report = Workspace {
            dir: tempfile::tempdir().unwrap(),
            lcov: real.lcov.clone(),
        }
        .record(&gone, &[(1, 1)]);
        let run = check(&[&report.report(), real.dir.path()]);
        assert_eq!(run.code, Some(1), "{}: {}", gone.display(), run.stdout);
        assert!(run.stderr.contains("gone.rs"), "{}", run.stderr);
    }
}

#[cfg(unix)]
#[test]
fn a_fifo_or_a_symlink_named_by_the_report_is_an_error_and_never_a_wait() {
    let workspace = Workspace::new()
        .file("crates/kernel/src/lib.rs", 100, 100)
        .file("apps/api/src/main.rs", 100, 100);
    let fifo = workspace.path("crates/kernel/tests/fifo.rs");
    fs::create_dir_all(fifo.parent().unwrap()).unwrap();
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let link = workspace.path("crates/kernel/tests/link.rs");
    std::os::unix::fs::symlink("/etc/hosts", &link).unwrap();
    for named in [fifo, link] {
        let run = Workspace {
            dir: tempfile::tempdir().unwrap(),
            lcov: workspace.lcov.clone(),
        }
        .record(&named, &[(1, 1)]);
        let run = check(&[&run.report(), workspace.dir.path()]);
        assert_eq!(run.code, Some(1), "{}: {}", named.display(), run.stdout);
    }
}

#[cfg(unix)]
#[test]
fn a_fifo_or_a_symlink_under_a_measured_src_is_an_error() {
    for kind in ["fifo", "symlink"] {
        let workspace = Workspace::new()
            .file("crates/kernel/src/lib.rs", 100, 100)
            .file("apps/api/src/main.rs", 100, 100);
        let path = workspace.path("crates/kernel/src/odd.rs");
        if kind == "fifo" {
            assert!(
                Command::new("mkfifo")
                    .arg(&path)
                    .status()
                    .unwrap()
                    .success()
            );
        } else {
            std::os::unix::fs::symlink("/etc/hosts", &path).unwrap();
        }
        let run = workspace.check();
        assert_eq!(run.code, Some(1), "{kind}: {}", run.stdout);
        assert!(run.stderr.contains("odd.rs"), "{kind}: {}", run.stderr);
    }
}

#[test]
fn a_source_over_1_mib_or_a_report_over_64_mib_is_an_error() {
    let workspace = Workspace::new()
        .file("crates/kernel/src/lib.rs", 100, 100)
        .file("apps/api/src/main.rs", 100, 100);
    let big = workspace.write("crates/kernel/src/big.rs", "");
    fs::File::options()
        .write(true)
        .open(&big)
        .unwrap()
        .set_len(1024 * 1024 + 1)
        .unwrap();
    let run = workspace.check();
    assert_eq!(run.code, Some(1), "{}", run.stdout);
    assert!(run.stderr.contains("big.rs"), "{}", run.stderr);

    let workspace = Workspace::new()
        .file("crates/kernel/src/lib.rs", 100, 100)
        .file("apps/api/src/main.rs", 100, 100);
    let report = workspace.report();
    fs::File::options()
        .write(true)
        .open(&report)
        .unwrap()
        .set_len(64 * 1024 * 1024 + 1)
        .unwrap();
    let run = check(&[&report, workspace.dir.path()]);
    assert_eq!(run.code, Some(1), "{}", run.stdout);
    assert!(run.stderr.contains("lcov.info"), "{}", run.stderr);
}

#[test]
fn control_characters_in_a_path_from_the_report_are_printed_escaped() {
    let workspace = Workspace::new()
        .file("crates/kernel/src/lib.rs", 100, 100)
        .file("apps/api/src/main.rs", 100, 100);
    for hostile in [
        "crates/kernel/src/\u{1b}]0;owned\u{7}\u{1b}[2Jgone.rs",
        "shared/\u{1b}[31mlogic.rs",
    ] {
        let run = Workspace {
            dir: tempfile::tempdir().unwrap(),
            lcov: workspace.lcov.clone(),
        }
        .record(&workspace.path(hostile), &[(1, 1)]);
        let run = check(&[&run.report(), workspace.dir.path()]);
        assert_eq!(run.code, Some(1), "{}", run.stdout);
        assert!(!run.stderr.contains('\u{1b}'), "{:?}", run.stderr);
        assert!(run.stderr.contains("\\u{1b}"), "{:?}", run.stderr);
    }
}

#[test]
fn a_missing_report_fails() {
    let workspace = Workspace::new();
    let missing = workspace.dir.path().join("lcov.info");
    let run = check(&[&missing, workspace.dir.path()]);
    assert_eq!(run.code, Some(1), "{}", run.stdout);
    assert!(run.stderr.contains("lcov.info"), "{}", run.stderr);
    assert_eq!(run.stdout, "");
}

#[test]
fn a_report_cut_short_fails_and_names_the_report() {
    let mut workspace = Workspace::new()
        .file("crates/kernel/src/money.rs", 100, 100)
        .file("apps/api/src/main.rs", 100, 100);
    let whole = workspace.lcov.len();
    workspace.lcov.truncate(whole - "end_of_record\n".len());
    let run = workspace.check();
    assert_eq!(run.code, Some(1), "{}", run.stdout);
    assert!(run.stderr.contains("lcov.info"), "{}", run.stderr);
    assert_eq!(run.stdout, "");
}

#[test]
fn a_report_that_measures_nothing_fails() {
    let run = Workspace::new().check();
    assert_eq!(run.code, Some(1), "{}", run.stdout);
    assert!(
        run.stdout.ends_with("total: no line was measured\n"),
        "{}",
        run.stdout
    );
}

#[test]
fn without_a_root_the_workspace_is_this_repository_wherever_the_command_runs() {
    // A report about another workspace measures nothing here, even from inside it: this
    // repository's files are then named by no record.
    let other = Workspace::new()
        .file("crates/kernel/src/money.rs", 100, 100)
        .file("apps/api/src/main.rs", 100, 100);
    let run = checked(
        Command::new(env!("CARGO_BIN_EXE_xtask"))
            .arg("check-coverage")
            .arg(other.report())
            .current_dir(other.dir.path())
            .output()
            .unwrap(),
    );
    assert_eq!(run.code, Some(1), "{}", run.stdout);
    assert_eq!(run.stdout, "");
    assert!(run.stderr.contains("does not name it"), "{}", run.stderr);

    // A report that runs the first line of every source of this repository passes: each
    // file with a function body must be named.
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let mut own = String::new();
    for file in measured_sources_of(&repository) {
        own.push_str(&format!("SF:{}\nDA:1,1\nend_of_record\n", file.display()));
    }
    fs::write(other.dir.path().join("own.info"), own).unwrap();
    let run = checked(
        Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(["check-coverage", "own.info"])
            .current_dir(other.dir.path())
            .output()
            .unwrap(),
    );
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    for figure in [
        "crates/kernel: 100.0% (",
        "crates/xtask: 100.0% (",
        "total: 100.0% (",
    ] {
        assert!(run.stdout.contains(figure), "{figure}: {}", run.stdout);
    }
}

/// Every source check-coverage measures in the workspace at `root`: the `.rs` files under
/// the `src/` of each crate it measures, test modules and hidden entries aside.
fn measured_sources_of(root: &Path) -> Vec<PathBuf> {
    let mut pending: Vec<PathBuf> = ["crates/kernel", "crates/bootstrap", "crates/xtask"]
        .iter()
        .map(|folder| root.join(folder).join("src"))
        .collect();
    for parent in ["apps", "crates/modules", "crates/adapters"] {
        let Ok(entries) = fs::read_dir(root.join(parent)) else {
            continue;
        };
        for entry in entries {
            pending.push(entry.unwrap().path().join("src"));
        }
    }
    let mut found = Vec::new();
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            if name.starts_with('.') || name == "tests" || name == "tests.rs" {
                continue;
            }
            if path.is_dir() {
                pending.push(path);
            } else if name.ends_with(".rs") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

#[test]
fn the_report_is_a_required_argument() {
    let run = check(&[]);
    assert_eq!(run.code, Some(2), "{}", run.stdout);
    assert!(
        run.stderr.contains("check-coverage <lcov-file>"),
        "{}",
        run.stderr
    );
}

/// A workspace of a kernel at 100% and an app at 100%, and the report of a run over it.
fn a_clean_workspace() -> Workspace {
    Workspace::new()
        .file("crates/kernel/src/lib.rs", 100, 100)
        .file("apps/api/src/main.rs", 100, 100)
}

#[test]
fn a_record_line_the_report_does_not_know_is_an_error_naming_the_report() {
    let workspace = Workspace::new()
        .lines("crates/kernel/src/lib.rs", &production(90, 10))
        .file("apps/api/src/main.rs", 100, 100);
    for (missed, garbled) in [(" DA:91,0", "indented"), ("da:91,0", "lower case")] {
        let lcov = workspace.lcov.replacen("DA:91,0", missed, 1);
        let line = lcov.lines().position(|text| text == missed).unwrap() + 1;
        let garbled_report = Workspace {
            dir: tempfile::tempdir().unwrap(),
            lcov,
        };
        let run = check(&[&garbled_report.report(), workspace.dir.path()]);
        assert_eq!(run.code, Some(1), "{garbled}: {}", run.stdout);
        assert!(
            run.stderr
                .contains(&format!("lcov.info is malformed at line {line}")),
            "{garbled}: {}",
            run.stderr
        );
    }
}

#[test]
fn an_error_about_a_file_of_the_report_names_the_report_and_the_line() {
    let workspace = a_clean_workspace();
    let line = workspace.lcov.lines().count() + 1;
    let shared = workspace.write("shared/logic.rs", "pub fn never_tested() {}\n");
    let run = workspace.record(&shared, &[(1, 0)]).check();
    assert_eq!(run.code, Some(1), "{}", run.stdout);
    assert!(
        run.stderr.contains(&format!("lcov.info:{line}: ")),
        "{}",
        run.stderr
    );
}

#[test]
fn a_source_with_code_that_the_report_does_not_name_is_an_error() {
    // cargo-llvm-cov drops any path with `/rustc/<hex>/`, and `facade` is hex.
    let workspace = a_clean_workspace().lines(
        "crates/modules/pricing/src/lib.rs",
        &[Run("pub mod rustc;"), Run("pub fn tested() {}")],
    );
    workspace.write("crates/modules/pricing/src/rustc.rs", "pub mod facade;\n");
    workspace.write(
        "crates/modules/pricing/src/rustc/facade.rs",
        "pub fn never_tested() -> u64 {\n    1\n}\n",
    );
    let run = workspace.check();
    assert_eq!(run.code, Some(1), "{}", run.stdout);
    assert!(run.stderr.contains("rustc/facade.rs"), "{}", run.stderr);

    // An adapter whose tests never run is absent from the report.
    let workspace = a_clean_workspace();
    workspace.write(
        "crates/adapters/ghost/src/lib.rs",
        "pub fn never_tested() -> u64 {\n    1\n}\n",
    );
    let run = workspace.check();
    assert_eq!(run.code, Some(1), "{}", run.stdout);
    assert!(
        run.stderr.contains("crates/adapters/ghost/src/lib.rs"),
        "{}",
        run.stderr
    );
}

#[test]
fn brackets_nested_too_deep_are_an_error_that_names_the_file() {
    let workspace = Workspace::new().file("apps/api/src/main.rs", 100, 100);
    let calls = "id(".repeat(800) + "1" + &")".repeat(800);
    let path = workspace.write(
        "crates/kernel/src/lib.rs",
        &format!(
            "fn id(n: u64) -> u64 {{\n    n\n}}\n\npub fn deep() -> u64 {{\n    {calls}\n}}\n"
        ),
    );
    let run = workspace
        .record(&path, &[(1, 1), (2, 1), (5, 1), (6, 1)])
        .check();
    assert_eq!(run.code, Some(1), "{}{}", run.stdout, run.stderr);
    assert!(
        run.stderr.contains("crates/kernel/src/lib.rs:6: "),
        "{}",
        run.stderr
    );
}

#[test]
fn every_shape_check_coverage_cannot_measure_is_an_error_that_names_file_and_line() {
    for (shape, line) in [
        ("#[cfg(not(debug_assertions))]\npub fn shipped() {}", 1),
        ("#[cfg(shipping)]\npub fn shipped() {}", 1),
        ("#[cfg(not(feature = \"cheap\"))]\npub fn shipped() {}", 1),
        (
            "macro_rules! gated {\n    ($p:meta, $i:item) => { #[cfg($p)] $i };\n}",
            2,
        ),
        (
            "macro_rules! padding {\n    () => { #[test] fn padding() {} };\n}",
            2,
        ),
        (
            "macro_rules! module {\n    ($name:ident) => { pub mod $name; };\n}",
            2,
        ),
        ("include!(concat!(\"../tests/\", \"helpers.rs\"));", 1),
        ("use std::include as inc;", 1),
        (
            "include!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/src/tests.rs\"));",
            1,
        ),
        (
            "pub fn f() {}\n#[cfg(test)]\nmod tests {\n    include!(\"money.rs\");\n}",
            4,
        ),
    ] {
        let workspace = Workspace::new().file("apps/api/src/main.rs", 100, 100);
        let path = workspace.write("crates/kernel/src/lib.rs", &format!("{shape}\n"));
        let run = workspace.record(&path, &[(1, 1)]).check();
        assert_eq!(run.code, Some(1), "{shape}: {}", run.stdout);
        assert!(
            run.stderr
                .contains(&format!("crates/kernel/src/lib.rs:{line}: ")),
            "{shape}: {}",
            run.stderr
        );
    }
}

#[test]
fn a_test_module_file_is_read_for_include_and_path() {
    for (file, text) in [
        ("crates/kernel/src/tests.rs", "include!(\"lib.rs\");\n"),
        (
            "crates/kernel/src/tests/helpers.rs",
            "#[path = \"../lib.rs\"]\nmod copy;\n",
        ),
    ] {
        let workspace = a_clean_workspace();
        workspace.write(file, text);
        let run = workspace.check();
        assert_eq!(run.code, Some(1), "{file}: {}", run.stdout);
        assert!(
            run.stderr.contains(&format!("{file}:1: ")),
            "{file}: {}",
            run.stderr
        );
    }
}

#[test]
fn generated_code_may_be_included_from_out_dir() {
    let workspace = Workspace::new()
        .lines(
            "crates/kernel/src/lib.rs",
            &[
                Run("pub fn tested() {}"),
                Bare("include!(concat!(env!(\"OUT_DIR\"), \"/generated.rs\"));"),
            ],
        )
        .file("apps/api/src/main.rs", 100, 100);
    let run = workspace.check();
    assert_eq!(run.code, Some(0), "{}", run.stderr);
}

/// A kernel whose `lib.rs` holds `lines`, every one of them run, and an app at 100%.
fn a_kernel_of(lines: &[&str]) -> Workspace {
    let lines: Vec<Line> = lines.iter().map(|line| Run(line)).collect();
    Workspace::new()
        .lines("crates/kernel/src/lib.rs", &lines)
        .file("apps/api/src/main.rs", 100, 100)
}

#[test]
fn a_file_compiled_out_by_a_cfg_need_not_be_named_by_the_report() {
    let workspace = a_kernel_of(&[
        "pub fn f() -> u8 { 1 }",
        "#[cfg(windows)]",
        "mod win;",
        "#[cfg(feature = \"extra\")]",
        "pub mod extra;",
        "pub mod gated;",
    ]);
    workspace.write(
        "crates/kernel/src/win.rs",
        "pub fn only_windows() -> u8 {\n    1\n}\n",
    );
    workspace.write(
        "crates/kernel/src/extra.rs",
        "pub fn extra() -> u8 {\n    1\n}\n",
    );
    workspace.write(
        "crates/kernel/src/gated.rs",
        "#[cfg(feature = \"extra\")]\npub fn extra() -> u8 {\n    1\n}\n",
    );
    let run = workspace.check();
    assert_eq!(run.code, Some(0), "{}", run.stderr);
}

#[test]
fn honest_code_that_names_include_test_or_cfg_passes() {
    for shape in [
        &[
            "pub struct Query {",
            "    pub include: Vec<String>,",
            "}",
            "pub fn wants(query: &Query) -> bool {",
            "    !query.include.is_empty()",
            "}",
        ][..],
        &[
            "pub fn label(test: &str) -> String {",
            "    format!(\"case {test} {}\", test)",
            "}",
        ],
        &[
            "pub fn f() -> u8 {",
            "    debug_assert!(cfg!(target_pointer_width = \"64\"));",
            "    if cfg!(test) { 1 } else { 2 }",
            "}",
        ],
        &[
            "macro_rules! check {",
            "    ($test:expr) => {",
            "        assert!($test)",
            "    };",
            "}",
            "pub fn f() -> u8 {",
            "    check!(true);",
            "    1",
            "}",
        ],
        &[
            "pub fn f() -> u8 { 1 }",
            "#[cfg(debug_assertions)]",
            "pub fn dev_only() -> u8 { 2 }",
        ],
    ] {
        let run = a_kernel_of(shape).check();
        assert_eq!(run.code, Some(0), "{shape:?}: {}", run.stderr);
    }
}
