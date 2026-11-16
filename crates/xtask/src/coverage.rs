//! The coverage bars: reads an LCOV report and holds each crate to its share of tested lines.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::ops::{AddAssign, RangeInclusive};
use std::path::{Component, Path, PathBuf};

use crate::arch::{Layer, layer_of};
use crate::error::XtaskError;

/// Percent of lines, for `crates/kernel` and for each crate under `crates/modules/`.
const CRATE_BAR: u64 = 95;
/// Percent of lines, for the measured crates together.
const TOTAL_BAR: u64 = 85;

/// Production lines the tests ran, out of those the report measured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Lines {
    pub hit: u64,
    pub found: u64,
}

impl Lines {
    /// In whole numbers, so that no rounding lifts a figure over its bar.
    fn meets(self, bar: u64) -> bool {
        u128::from(self.hit) * 100 >= u128::from(bar) * u128::from(self.found)
    }
}

impl AddAssign for Lines {
    fn add_assign(&mut self, other: Self) {
        self.hit += other.hit;
        self.found += other.found;
    }
}

impl fmt::Display for Lines {
    /// `97.3% (220/226)`. The figure is cut, not rounded: 94.99% under a bar of 95 must
    /// not read as 95.0%.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Self { hit, found } = *self;
        match (u128::from(hit) * 1000).checked_div(u128::from(found)) {
            Some(tenths) => write!(f, "{}.{}% ({hit}/{found})", tenths / 10, tenths % 10),
            None => f.write_str("no line was measured"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CoverageReport {
    /// Each measured crate that has a counted line, by its folder.
    pub crates: BTreeMap<PathBuf, Lines>,
}

impl CoverageReport {
    pub fn total(&self) -> Lines {
        let mut total = Lines::default();
        for lines in self.crates.values() {
            total += *lines;
        }
        total
    }

    /// One line per crate, then the total.
    pub fn summary(&self) -> Vec<String> {
        let mut summary: Vec<String> = self
            .crates
            .iter()
            .map(|(folder, lines)| format!("{}: {lines}", folder.display()))
            .collect();
        summary.push(format!("total: {}", self.total()));
        summary
    }
}

/// `check_coverage` over the report in `file`.
pub fn check_coverage_file(root: &Path, file: &Path) -> Result<CoverageReport, XtaskError> {
    let lcov = fs::read_to_string(file).map_err(XtaskError::io(file))?;
    check_coverage(root, &lcov)
}

/// Counts, crate by crate, the production lines that the LCOV text `lcov` measured in the
/// workspace at `root`.
pub fn check_coverage(root: &Path, lcov: &str) -> Result<CoverageReport, XtaskError> {
    // The report names files by their real path, and `root` may be written with `..` or
    // through a symlink.
    let root = fs::canonicalize(root).map_err(XtaskError::io(root))?;
    let mut files: BTreeMap<PathBuf, FileLines> = BTreeMap::new();
    for (file, lines) in parse_lcov(lcov)? {
        if let Some(relative) = inside(&root, &file) {
            let merged = files.entry(relative).or_default();
            for (line, hit) in lines {
                record(merged, line, hit);
            }
        }
    }
    let mut crates: BTreeMap<PathBuf, Lines> = BTreeMap::new();
    for (file, lines) in &files {
        let Some(folder) = measured_crate(file) else {
            continue;
        };
        let counted = production_lines(&root.join(file), lines)?;
        if counted.found > 0 {
            *crates.entry(folder.to_path_buf()).or_default() += counted;
        }
    }
    Ok(CoverageReport { crates })
}

/// What is under its bar, one message each; empty when the report passes.
pub fn under_the_bars(report: &CoverageReport) -> Vec<String> {
    let mut problems: Vec<String> = report
        .crates
        .iter()
        .filter(|(folder, lines)| has_its_own_bar(folder) && !lines.meets(CRATE_BAR))
        .map(|(folder, lines)| {
            format!(
                "{}: {lines} is under its bar of {CRATE_BAR}%",
                folder.display()
            )
        })
        .collect();
    let total = report.total();
    if total.found == 0 {
        // Nothing out of nothing meets any bar in whole numbers. A report that measured
        // no line is a run that went wrong, not a pass.
        problems.push(format!("total: {total}"));
    } else if !total.meets(TOTAL_BAR) {
        problems.push(format!("total: {total} is under its bar of {TOTAL_BAR}%"));
    }
    problems
}

fn has_its_own_bar(folder: &Path) -> bool {
    matches!(layer_of(folder), Some(Layer::Kernel | Layer::Module))
}

/// Line number to "a test ran it".
type FileLines = BTreeMap<usize, bool>;

/// A line counts once, and as run if any record ran it.
fn record(lines: &mut FileLines, line: usize, hit: bool) {
    *lines.entry(line).or_insert(false) |= hit;
}

/// The `DA` records of an LCOV text, by the source file their `SF` record names. Every
/// other record repeats or sums these, or describes functions and branches.
fn parse_lcov(lcov: &str) -> Result<BTreeMap<PathBuf, FileLines>, XtaskError> {
    let mut files: BTreeMap<PathBuf, FileLines> = BTreeMap::new();
    let mut open: Option<PathBuf> = None;
    let mut last = 0;
    for (index, text) in lcov.lines().enumerate() {
        last = index + 1;
        let malformed = |problem| XtaskError::Lcov {
            line: last,
            problem,
        };
        if let Some(path) = text.strip_prefix("SF:") {
            if open.is_some() {
                return Err(malformed("a record starts inside another"));
            }
            open = Some(source_path(path).ok_or_else(|| {
                malformed("the path of a source file must be absolute and without `..`")
            })?);
        } else if let Some(data) = text.strip_prefix("DA:") {
            let file = open
                .as_ref()
                .ok_or_else(|| malformed("a line record outside a file record"))?;
            let (line, hits) = line_record(data)
                .ok_or_else(|| malformed("a line record must be `DA:<line>,<hits>`"))?;
            record(files.entry(file.clone()).or_default(), line, hits > 0);
        } else if text == "end_of_record" && open.take().is_none() {
            return Err(malformed("a record ends that never started"));
        }
    }
    if open.is_some() {
        // A report cut short would otherwise pass with the lines it still has.
        return Err(XtaskError::Lcov {
            line: last,
            problem: "the last record never ends",
        });
    }
    Ok(files)
}

fn source_path(path: &str) -> Option<PathBuf> {
    let path = Path::new(path);
    let plain = path.is_absolute() && !path.components().any(|part| part == Component::ParentDir);
    plain.then(|| path.to_path_buf())
}

/// `<line>,<hits>` with an optional `,<checksum>`. Lines are numbered from 1.
fn line_record(data: &str) -> Option<(usize, u64)> {
    let mut fields = data.split(',');
    let line: usize = fields.next()?.parse().ok()?;
    let hits: u64 = fields.next()?.parse().ok()?;
    (line > 0).then_some((line, hits))
}

/// `file` relative to `root`, which is canonical, or `None` when it is outside. A report
/// may spell the workspace through a symlink (`/tmp` on macOS), so a path that does not
/// start with `root` is tried again by its real path. One that does not resolve is not
/// in the workspace either.
fn inside(root: &Path, file: &Path) -> Option<PathBuf> {
    if let Ok(relative) = file.strip_prefix(root) {
        return Some(relative.to_path_buf());
    }
    let real = fs::canonicalize(file).ok()?;
    real.strip_prefix(root).ok().map(Path::to_path_buf)
}

/// The folder of the crate that `file` is in, when that crate is measured:
/// `crates/testkit` is test code and `crates/workspace-hack` is generated.
fn measured_crate(file: &Path) -> Option<&Path> {
    file.ancestors().skip(1).find(|folder| {
        layer_of(folder)
            .is_some_and(|layer| !matches!(layer, Layer::Testkit | Layer::WorkspaceHack))
    })
}

/// The lines of the report for the source at `path`, without those of its test modules.
fn production_lines(path: &Path, lines: &FileLines) -> Result<Lines, XtaskError> {
    let source = fs::read_to_string(path).map_err(XtaskError::io(path))?;
    let length = source.lines().count();
    if let Some(line) = lines.keys().next_back().filter(|line| **line > length) {
        // The test modules are found in the file as it is now, so their lines would be
        // the wrong ones.
        return Err(XtaskError::StaleReport {
            path: path.to_path_buf(),
            line: *line,
        });
    }
    let tests = test_module_lines(&source);
    let mut counted = Lines::default();
    for (line, hit) in lines {
        if !tests.iter().any(|module| module.contains(line)) {
            counted += Lines {
                hit: u64::from(*hit),
                found: 1,
            };
        }
    }
    Ok(counted)
}

/// The lines, numbered from 1, of each inline test module of a source file. cargo-llvm-cov
/// counts them as covered lines, which would lift every figure.
fn test_module_lines(source: &str) -> Vec<RangeInclusive<usize>> {
    let lines: Vec<&str> = source.lines().collect();
    let mut modules = Vec::new();
    let mut at = 0;
    while let Some(rest) = lines.get(at..).filter(|rest| !rest.is_empty()) {
        match test_module_length(rest) {
            Some(length) => {
                modules.push(at + 1..=at + length);
                at += length;
            }
            None => at += 1,
        }
    }
    modules
}

/// How many lines the test module takes that starts on the first of `lines`, if one does.
/// It is the one shape rustfmt gives it, every line below at column 0:
///
/// ```text
/// #[cfg(test)]
/// #[allow(clippy::too_many_lines)] // any number of other attribute lines
/// mod tests {
///     …
/// }
/// ```
///
/// Anything else leaves nothing out, so no production line is dropped on a guess.
fn test_module_length(lines: &[&str]) -> Option<usize> {
    let (first, rest) = lines.split_first()?;
    if *first != "#[cfg(test)]" {
        return None;
    }
    let attributes = rest
        .iter()
        .take_while(|line| line.starts_with("#["))
        .count();
    let (opening, body) = rest.get(attributes..)?.split_first()?;
    if !(opening.starts_with("mod ") && opening.ends_with('{')) {
        return None;
    }
    let closing = body.iter().position(|line| *line == "}")?;
    // The `#[cfg(test)]` line, the attributes, the `mod` line, the body and its `}`.
    Some(1 + attributes + 1 + closing + 1)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    // A fixture is written as a list of lines, never as one literal that spans lines: a
    // `}` at column 0 inside this module would end it for `test_module_lines` itself.
    fn text(lines: &[&str]) -> String {
        let mut text = lines.join("\n");
        text.push('\n');
        text
    }

    /// The first and the last line of each test module.
    fn test_modules(source: &str) -> Vec<(usize, usize)> {
        test_module_lines(source)
            .iter()
            .map(|module| (*module.start(), *module.end()))
            .collect()
    }

    fn write(root: &Path, relative: &str, lines: &[&str]) -> PathBuf {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text(lines)).unwrap();
        path
    }

    /// One LCOV record: `lines` are (line number, hits).
    fn record(file: &Path, lines: &[(usize, u64)]) -> String {
        let mut record = format!("SF:{}\n", file.display());
        for (line, hits) in lines {
            record.push_str(&format!("DA:{line},{hits}\n"));
        }
        record.push_str("end_of_record\n");
        record
    }

    /// `hit` lines run and `found - hit` lines not run, numbered from 1.
    fn lines_run(hit: usize, found: usize) -> Vec<(usize, u64)> {
        (1..=found)
            .map(|line| (line, u64::from(line <= hit)))
            .collect()
    }

    /// A source file of `count` lines with no test module.
    fn plain_source(root: &Path, relative: &str, count: usize) -> PathBuf {
        let lines = vec!["// production"; count];
        write(root, relative, &lines)
    }

    fn report(crates: &[(&str, u64, u64)]) -> CoverageReport {
        CoverageReport {
            crates: crates
                .iter()
                .map(|(folder, hit, found)| {
                    (
                        PathBuf::from(folder),
                        Lines {
                            hit: *hit,
                            found: *found,
                        },
                    )
                })
                .collect(),
        }
    }

    const PRODUCTION_THEN_TESTS: &[&str] = &[
        "pub fn double(n: u32) -> u32 {",
        "    n * 2",
        "}",
        "",
        "#[cfg(test)]",
        "mod tests {",
        "    use super::*;",
        "",
        "    #[test]",
        "    fn doubles() {",
        "        assert_eq!(double(2), 4);",
        "    }",
        "}",
    ];

    // --- the test-module filter

    #[test]
    fn a_test_module_at_the_end_is_left_out() {
        assert_eq!(test_modules(&text(PRODUCTION_THEN_TESTS)), [(5, 13)]);
    }

    #[test]
    fn a_test_module_in_the_middle_is_left_out_and_the_code_after_it_is_kept() {
        let source = text(&[
            "fn before() {}",
            "",
            "#[cfg(test)]",
            "mod tests {",
            "    #[test]",
            "    fn works() {}",
            "}",
            "",
            "fn after() {",
            "    before();",
            "}",
        ]);
        assert_eq!(test_modules(&source), [(3, 7)]);
    }

    #[test]
    fn every_test_module_of_a_file_is_left_out() {
        let source = text(&[
            "#[cfg(test)]",
            "mod tests {",
            "    #[test]",
            "    fn one() {}",
            "}",
            "",
            "fn between() {}",
            "",
            "#[cfg(test)]",
            "mod more_tests {",
            "    #[test]",
            "    fn two() {}",
            "}",
        ]);
        assert_eq!(test_modules(&source), [(1, 5), (9, 13)]);
    }

    #[test]
    fn attribute_lines_may_sit_between_the_two_lines() {
        let source = text(&[
            "fn production() {}",
            "",
            "#[cfg(test)]",
            "#[allow(clippy::too_many_lines)] // one long table",
            "#[rustfmt::skip]",
            "mod tests {",
            "    #[test]",
            "    fn works() {}",
            "}",
        ]);
        assert_eq!(test_modules(&source), [(3, 9)]);
    }

    #[test]
    fn nested_braces_inside_the_module_do_not_end_it() {
        let source = text(&[
            "#[cfg(test)]",
            "mod tests {",
            "    struct Fixture {",
            "        id: u32,",
            "    }",
            "",
            "    mod inner {",
            "        #[test]",
            "        fn works() {",
            "            if true {",
            "            }",
            "        }",
            "    }",
            "}",
            "",
            "fn after() {}",
        ]);
        assert_eq!(test_modules(&source), [(1, 14)]);
    }

    #[test]
    fn cfg_test_on_anything_but_an_inline_module_leaves_nothing_out() {
        // Each is followed by production code that a wrong match would swallow.
        for item in [
            &["#[cfg(test)]", "use std::fs;"][..],
            &["#[cfg(test)]", "fn helper() {", "}"],
            &["#[cfg(test)]", "impl Clock {", "    fn set(&self) {}", "}"],
            &["#[cfg(test)]", "mod tests;"],
            &["#[cfg(test)]", "pub mod fixtures {", "}"],
            &["#[cfg(test)]", "pub(crate) mod fixtures {", "}"],
            &["#[cfg(test)]", "mod tests {}"],
            &["#[cfg(test)]", "mod tests { // why", "}"],
            &["#[cfg(test)] // why", "mod tests {", "}"],
            &[
                "#[cfg(any(test, feature = \"testing\"))]",
                "mod fixed {",
                "}",
            ],
            &["#[cfg(not(test))]", "mod real {", "}"],
            &["#![cfg(test)]", "mod tests {", "}"],
            &["    #[cfg(test)]", "mod tests {", "}"],
            &["mod tests {", "}"],
        ] {
            let source = [item, &["", "fn production() {", "    work();", "}"]].concat();
            assert_eq!(test_modules(&text(&source)), [], "{item:?}");
        }
    }

    #[test]
    fn only_attribute_lines_may_sit_between_so_production_code_is_never_swallowed() {
        let source = text(&[
            "#[cfg(test)]",
            "use std::fs;",
            "",
            "mod parser {",
            "    pub fn parse() {}",
            "}",
        ]);
        assert_eq!(test_modules(&source), []);
        let source = text(&[
            "#[cfg(test)]",
            "// a note about the module",
            "mod tests {",
            "}",
        ]);
        assert_eq!(test_modules(&source), []);
        let source = text(&["#[cfg(test)]", "", "mod tests {", "}"]);
        assert_eq!(test_modules(&source), []);
    }

    #[test]
    fn an_indented_module_is_not_a_test_module_of_the_file() {
        let source = text(&[
            "mod outer {",
            "    #[cfg(test)]",
            "    mod tests {",
            "        #[test]",
            "        fn works() {}",
            "    }",
            "}",
        ]);
        assert_eq!(test_modules(&source), []);
    }

    #[test]
    fn a_module_that_never_closes_leaves_nothing_out() {
        let source = text(&["fn f() {}", "#[cfg(test)]", "mod tests {", "    fn t() {}"]);
        assert_eq!(test_modules(&source), []);
        assert_eq!(test_modules("#[cfg(test)]\n#[allow(dead_code)]"), []);
        assert_eq!(test_modules("#[cfg(test)]"), []);
        assert_eq!(test_modules(""), []);
    }

    #[test]
    fn windows_line_endings_change_nothing() {
        let source = text(PRODUCTION_THEN_TESTS).replace('\n', "\r\n");
        assert_eq!(test_modules(&source), [(5, 13)]);
    }

    // --- parsing

    #[test]
    fn the_line_records_are_read_and_every_other_record_is_ignored() {
        let lcov = text(&[
            "TN:",
            "SF:/work/crates/kernel/src/money.rs",
            "FN:3,_RNvCs_6kernel5money",
            "FNDA:7,_RNvCs_6kernel5money",
            "FNF:1",
            "FNH:1",
            "DA:3,7",
            "DA:4,0",
            "DA:5,18446744073709551615",
            "BRDA:4,0,0,-",
            "BRF:1",
            "BRH:0",
            "LF:3",
            "LH:2",
            "end_of_record",
            "SF:/work/apps/api/src/main.rs",
            "DA:1,0",
            "end_of_record",
        ]);
        let files = parse_lcov(&lcov).unwrap();
        assert_eq!(
            files,
            BTreeMap::from([
                (
                    PathBuf::from("/work/apps/api/src/main.rs"),
                    BTreeMap::from([(1, false)])
                ),
                (
                    PathBuf::from("/work/crates/kernel/src/money.rs"),
                    BTreeMap::from([(3, true), (4, false), (5, true)])
                ),
            ])
        );
    }

    #[test]
    fn a_file_in_several_records_counts_each_line_once_and_as_run_if_any_record_ran_it() {
        let file = Path::new("/work/crates/kernel/src/id.rs");
        let lcov = [
            record(file, &[(1, 0), (2, 3), (3, 0)]),
            record(Path::new("/work/apps/api/src/main.rs"), &[(1, 1)]),
            record(file, &[(1, 2), (2, 0), (3, 0), (4, 0)]),
        ]
        .concat();
        assert_eq!(
            parse_lcov(&lcov).unwrap()[file],
            BTreeMap::from([(1, true), (2, true), (3, false), (4, false)])
        );
    }

    #[test]
    fn a_line_record_may_carry_a_checksum() {
        let lcov = text(&[
            "SF:/work/a.rs",
            "DA:9,1,PF4Rz2r7RTliO9u6bZ7h6g",
            "end_of_record",
        ]);
        assert_eq!(
            parse_lcov(&lcov).unwrap()[Path::new("/work/a.rs")],
            BTreeMap::from([(9, true)])
        );
    }

    #[test]
    fn a_malformed_record_is_an_error_that_names_its_line() {
        // Each report is whole but for one line, so nothing else can be what fails it.
        for (lcov, line) in [
            (&["DA:1,1", "SF:/work/a.rs", "end_of_record"][..], 1),
            (&["SF:/work/a.rs", "end_of_record", "DA:1,1", "TN:"], 3),
            (&["end_of_record", "SF:/work/a.rs", "end_of_record"], 1),
            (&["SF:/work/a.rs", "SF:/work/b.rs", "end_of_record"], 2),
            (&["SF:/work/a.rs", "DA:1", "end_of_record"], 2),
            (&["SF:/work/a.rs", "DA:,1", "end_of_record"], 2),
            (&["SF:/work/a.rs", "DA:one,1", "end_of_record"], 2),
            (&["SF:/work/a.rs", "DA:0,1", "end_of_record"], 2),
            (&["SF:/work/a.rs", "DA:-1,1", "end_of_record"], 2),
            (&["SF:/work/a.rs", "DA:1,-1", "end_of_record"], 2),
            (&["SF:/work/a.rs", "DA:1,1.5", "end_of_record"], 2),
            (&["SF:/work/a.rs", "DA:1,", "end_of_record"], 2),
            (&["SF:/work/a.rs", "DA:1, 1", "end_of_record"], 2),
            (&["SF:", "end_of_record"], 1),
            (&["SF:crates/kernel/src/money.rs", "end_of_record"], 1),
            (
                &[
                    "SF:/work/crates/kernel/../../../etc/passwd",
                    "end_of_record",
                ],
                1,
            ),
        ] {
            let error = parse_lcov(&text(lcov)).unwrap_err();
            assert!(
                matches!(error, XtaskError::Lcov { line: found, .. } if found == line),
                "{lcov:?}: {error}"
            );
        }
    }

    #[test]
    fn a_report_cut_short_is_an_error() {
        for (lcov, line) in [
            (&["SF:/work/a.rs"][..], 1),
            (&["SF:/work/a.rs", "DA:1,1"], 2),
            (
                &["SF:/work/a.rs", "DA:1,1", "end_of_record", "SF:/work/b.rs"],
                4,
            ),
            (&["SF:/work/a.rs", "DA:1,1", "end_of_recor"], 3),
        ] {
            let error = parse_lcov(&text(lcov)).unwrap_err();
            assert!(
                matches!(error, XtaskError::Lcov { line: found, .. } if found == line),
                "{lcov:?}: {error}"
            );
        }
    }

    #[test]
    fn an_empty_text_holds_no_record() {
        assert_eq!(parse_lcov("").unwrap(), BTreeMap::new());
        assert_eq!(parse_lcov("TN:\n\n").unwrap(), BTreeMap::new());
    }

    // --- grouping

    #[test]
    fn a_file_belongs_to_the_crate_folder_it_is_in() {
        for (file, folder) in [
            ("crates/kernel/src/money.rs", "crates/kernel"),
            (
                "crates/modules/identity/src/domain/user.rs",
                "crates/modules/identity",
            ),
            (
                "crates/adapters/http/src/envelope.rs",
                "crates/adapters/http",
            ),
            ("crates/bootstrap/src/logging.rs", "crates/bootstrap"),
            ("crates/xtask/src/main.rs", "crates/xtask"),
            ("apps/api/src/main.rs", "apps/api"),
            ("apps/ingest/build.rs", "apps/ingest"),
        ] {
            assert_eq!(
                measured_crate(Path::new(file)),
                Some(Path::new(folder)),
                "{file}"
            );
        }
    }

    #[test]
    fn the_testkit_the_hack_and_any_other_folder_are_not_measured() {
        for file in [
            "crates/testkit/src/process.rs",
            "crates/workspace-hack/src/lib.rs",
            "crates/misc/src/lib.rs",
            "vendor/uuid/src/lib.rs",
            "target/debug/build/out/generated.rs",
            "build.rs",
            "crates/kernel",
            "apps/api",
            "crates/modules/identity",
            "",
        ] {
            assert_eq!(measured_crate(Path::new(file)), None, "{file}");
        }
    }

    // --- counting

    #[test]
    fn only_production_lines_are_counted() {
        let root = tempfile::tempdir().unwrap();
        let file = write(
            root.path(),
            "crates/kernel/src/double.rs",
            PRODUCTION_THEN_TESTS,
        );
        let lcov = record(
            &file,
            &[(1, 4), (2, 0), (3, 4), (10, 1), (11, 1), (12, 1), (13, 1)],
        );
        assert_eq!(
            check_coverage(root.path(), &lcov).unwrap(),
            report(&[("crates/kernel", 2, 3)])
        );
    }

    #[test]
    fn each_crate_is_measured_on_its_own_and_the_total_adds_them_up() {
        let root = tempfile::tempdir().unwrap();
        let lcov = [
            record(
                &plain_source(root.path(), "crates/kernel/src/money.rs", 4),
                &lines_run(4, 4),
            ),
            record(
                &plain_source(root.path(), "crates/kernel/src/id.rs", 6),
                &lines_run(5, 6),
            ),
            record(
                &plain_source(root.path(), "crates/modules/goals/src/lib.rs", 2),
                &lines_run(1, 2),
            ),
            record(
                &plain_source(root.path(), "apps/api/src/main.rs", 3),
                &lines_run(0, 3),
            ),
        ]
        .concat();
        let found = check_coverage(root.path(), &lcov).unwrap();
        assert_eq!(
            found,
            report(&[
                ("apps/api", 0, 3),
                ("crates/kernel", 9, 10),
                ("crates/modules/goals", 1, 2),
            ])
        );
        assert_eq!(found.total(), Lines { hit: 10, found: 15 });
    }

    #[test]
    fn what_is_outside_the_workspace_or_outside_the_measured_crates_is_ignored() {
        let outside = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let lcov = [
            record(
                &plain_source(root.path(), "crates/kernel/src/id.rs", 2),
                &lines_run(2, 2),
            ),
            record(
                &plain_source(outside.path(), "crates/kernel/src/id.rs", 9),
                &lines_run(0, 9),
            ),
            record(
                Path::new("/rustc/0123abcd/library/core/src/option.rs"),
                &lines_run(0, 9),
            ),
            record(
                &plain_source(root.path(), "crates/testkit/src/process.rs", 9),
                &lines_run(9, 9),
            ),
            record(
                &plain_source(root.path(), "crates/workspace-hack/src/lib.rs", 9),
                &lines_run(9, 9),
            ),
            record(
                &plain_source(root.path(), "target/debug/build/out/generated.rs", 9),
                &lines_run(9, 9),
            ),
        ]
        .concat();
        assert_eq!(
            check_coverage(root.path(), &lcov).unwrap(),
            report(&[("crates/kernel", 2, 2)])
        );
    }

    #[test]
    fn a_crate_whose_every_line_is_a_test_has_no_figure() {
        let root = tempfile::tempdir().unwrap();
        let file = write(
            root.path(),
            "crates/kernel/src/lib.rs",
            &["#[cfg(test)]", "mod tests {", "    fn works() {}", "}"],
        );
        let lcov = [
            record(&file, &[(3, 1)]),
            record(
                &plain_source(root.path(), "apps/api/src/main.rs", 1),
                &lines_run(1, 1),
            ),
        ]
        .concat();
        assert_eq!(
            check_coverage(root.path(), &lcov).unwrap(),
            report(&[("apps/api", 1, 1)])
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_root_and_the_report_may_spell_the_workspace_through_a_symlink() {
        let real = tempfile::tempdir().unwrap();
        let links = tempfile::tempdir().unwrap();
        let link = links.path().join("workspace");
        std::os::unix::fs::symlink(real.path(), &link).unwrap();
        plain_source(real.path(), "crates/kernel/src/id.rs", 2);
        let through_the_link = record(&link.join("crates/kernel/src/id.rs"), &lines_run(1, 2));
        let expected = report(&[("crates/kernel", 1, 2)]);
        assert_eq!(
            check_coverage(real.path(), &through_the_link).unwrap(),
            expected
        );
        assert_eq!(check_coverage(&link, &through_the_link).unwrap(), expected);
        let root_with_dots = link.join("crates/kernel/../..");
        assert_eq!(
            check_coverage(&root_with_dots, &through_the_link).unwrap(),
            expected
        );
    }

    #[cfg(unix)]
    #[test]
    fn one_file_under_two_spellings_is_still_counted_once() {
        let real = tempfile::tempdir().unwrap();
        let links = tempfile::tempdir().unwrap();
        let link = links.path().join("workspace");
        std::os::unix::fs::symlink(real.path(), &link).unwrap();
        let file = plain_source(real.path(), "crates/kernel/src/id.rs", 3);
        let lcov = [
            record(&file.canonicalize().unwrap(), &[(1, 0), (2, 1), (3, 0)]),
            record(&link.join("crates/kernel/src/id.rs"), &[(1, 1), (2, 0)]),
        ]
        .concat();
        assert_eq!(
            check_coverage(real.path(), &lcov).unwrap(),
            report(&[("crates/kernel", 2, 3)])
        );
    }

    #[test]
    fn a_source_file_that_is_gone_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("crates/kernel/src")).unwrap();
        let lcov = record(
            &root
                .path()
                .canonicalize()
                .unwrap()
                .join("crates/kernel/src/gone.rs"),
            &lines_run(1, 1),
        );
        let error = check_coverage(root.path(), &lcov).unwrap_err();
        assert!(
            matches!(&error, XtaskError::Io { path, .. } if path.ends_with("gone.rs")),
            "{error}"
        );
    }

    #[test]
    fn a_report_older_than_the_source_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        let file = plain_source(root.path(), "crates/kernel/src/id.rs", 3);
        let lcov = record(&file, &[(3, 1), (4, 1)]);
        let error = check_coverage(root.path(), &lcov).unwrap_err();
        assert!(
            matches!(error, XtaskError::StaleReport { line: 4, .. }),
            "{error}"
        );
    }

    #[test]
    fn a_root_that_does_not_exist_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        let error = check_coverage(&root.path().join("missing"), "").unwrap_err();
        assert!(matches!(error, XtaskError::Io { .. }), "{error}");
    }

    #[test]
    fn a_report_file_that_cannot_be_read_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("lcov.info");
        let error = check_coverage_file(root.path(), &missing).unwrap_err();
        assert!(
            matches!(&error, XtaskError::Io { path, .. } if *path == missing),
            "{error}"
        );
        fs::write(&missing, [0xff, 0xfe]).unwrap();
        let error = check_coverage_file(root.path(), &missing).unwrap_err();
        assert!(matches!(error, XtaskError::Io { .. }), "{error}");
    }

    #[test]
    fn a_report_file_is_read_and_measured() {
        let root = tempfile::tempdir().unwrap();
        let file = plain_source(root.path(), "crates/kernel/src/id.rs", 2);
        let lcov = root.path().join("lcov.info");
        fs::write(&lcov, record(&file, &lines_run(2, 2))).unwrap();
        assert_eq!(
            check_coverage_file(root.path(), &lcov).unwrap(),
            report(&[("crates/kernel", 2, 2)])
        );
    }

    // --- the bars

    #[test]
    fn the_kernel_passes_at_exactly_its_bar_and_fails_one_line_under() {
        let rest = ("apps/api", 1000, 1000);
        assert_eq!(
            under_the_bars(&report(&[("crates/kernel", 950, 1000), rest])),
            Vec::<String>::new()
        );
        assert_eq!(
            under_the_bars(&report(&[("crates/kernel", 949, 1000), rest])),
            ["crates/kernel: 94.9% (949/1000) is under its bar of 95%"]
        );
    }

    #[test]
    fn each_module_is_held_to_the_bar_on_its_own() {
        let found = report(&[
            ("crates/kernel", 100, 100),
            ("crates/modules/capital", 19, 20),
            ("crates/modules/goals", 94, 100),
            ("crates/modules/identity", 18, 20),
        ]);
        assert_eq!(
            under_the_bars(&found),
            [
                "crates/modules/goals: 94.0% (94/100) is under its bar of 95%",
                "crates/modules/identity: 90.0% (18/20) is under its bar of 95%",
            ]
        );
    }

    #[test]
    fn the_total_passes_at_exactly_its_bar_and_fails_one_line_under() {
        assert_eq!(
            under_the_bars(&report(&[
                ("apps/api", 700, 1000),
                ("crates/kernel", 1000, 1000)
            ])),
            Vec::<String>::new()
        );
        assert_eq!(
            under_the_bars(&report(&[
                ("apps/api", 699, 1000),
                ("crates/kernel", 1000, 1000)
            ])),
            ["total: 84.9% (1699/2000) is under its bar of 85%"]
        );
    }

    #[test]
    fn a_crate_outside_the_kernel_and_the_modules_has_no_bar_of_its_own() {
        let found = report(&[
            ("apps/api", 0, 10),
            ("crates/adapters/http", 0, 10),
            ("crates/bootstrap", 0, 10),
            ("crates/xtask", 0, 10),
            ("crates/kernel", 1000, 1000),
        ]);
        assert_eq!(under_the_bars(&found), Vec::<String>::new());
    }

    #[test]
    fn both_bars_can_fail_at_once() {
        let found = report(&[("apps/api", 0, 100), ("crates/kernel", 94, 100)]);
        assert_eq!(under_the_bars(&found).len(), 2);
    }

    #[test]
    fn an_empty_report_fails() {
        assert_eq!(
            under_the_bars(&CoverageReport::default()),
            ["total: no line was measured"]
        );
        let root = tempfile::tempdir().unwrap();
        let measured = check_coverage(root.path(), "").unwrap();
        assert_eq!(under_the_bars(&measured).len(), 1);
        let nothing_of_ours = record(Path::new("/elsewhere/src/lib.rs"), &lines_run(9, 9));
        let measured = check_coverage(root.path(), &nothing_of_ours).unwrap();
        assert_eq!(under_the_bars(&measured).len(), 1);
    }

    #[test]
    fn a_crate_without_a_counted_line_passes() {
        let found = report(&[("crates/kernel", 0, 0), ("apps/api", 9, 10)]);
        assert_eq!(under_the_bars(&found), Vec::<String>::new());
    }

    #[test]
    fn the_bars_are_compared_without_rounding() {
        // 94.99…% must neither pass nor print as 95.0%.
        let found = report(&[("crates/kernel", 94_999, 100_000)]);
        assert_eq!(
            under_the_bars(&found),
            ["crates/kernel: 94.9% (94999/100000) is under its bar of 95%"]
        );
        let huge = Lines {
            hit: u64::MAX - 1,
            found: u64::MAX,
        };
        assert_eq!(
            huge.to_string(),
            format!("99.9% ({}/{})", u64::MAX - 1, u64::MAX)
        );
    }

    #[test]
    fn the_summary_has_one_line_per_crate_and_the_total() {
        let found = report(&[
            ("crates/kernel", 220, 226),
            ("apps/api", 0, 3),
            ("crates/xtask", 3, 3),
        ]);
        assert_eq!(
            found.summary(),
            [
                "apps/api: 0.0% (0/3)",
                "crates/kernel: 97.3% (220/226)",
                "crates/xtask: 100.0% (3/3)",
                "total: 96.1% (223/232)",
            ]
        );
        assert_eq!(
            CoverageReport::default().summary(),
            ["total: no line was measured"]
        );
    }
}
