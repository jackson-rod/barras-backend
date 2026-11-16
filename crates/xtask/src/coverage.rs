//! The coverage bars: reads an LCOV report and measures, crate by crate, the share of
//! production lines that the tests ran. `judge` holds each figure to its bar.

mod attributes;
mod bars;
mod lcov;
mod source;
mod sources;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::ops::AddAssign;
use std::path::{Path, PathBuf};

pub use bars::{BarFailure, judge};
use lcov::{Record, parse_lcov};
use source::Source;
use sources::{gated_sources, measured_crates, read_sources};

use crate::error::{XtaskError, escaped};
use crate::layout::{KERNEL_DIR, Layer, layer_of};
use crate::regular_file::read_regular_file;

/// A report of this workspace is a few megabytes; anything past this is refused unread.
const REPORT_LIMIT: u64 = 64 * 1024 * 1024;

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
    /// The folders that must have a figure: `crates/kernel`, and each `crates/modules/<m>`
    /// that holds a `Cargo.toml`.
    pub required: BTreeSet<PathBuf>,
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
            .map(|(folder, lines)| format!("{}: {lines}", escaped(folder)))
            .collect();
        summary.push(format!("total: {}", self.total()));
        summary
    }
}

/// `measure_coverage` over the report at `report`.
pub fn measure_coverage_file(root: &Path, report: &Path) -> Result<CoverageReport, XtaskError> {
    let lcov = read_regular_file(report, REPORT_LIMIT)?;
    measure_coverage(root, report, &lcov)
}

/// Line number to "a test ran it", a line counted once, and as run if any record ran it.
type FileLines = BTreeMap<usize, bool>;

/// Counts, crate by crate, the production lines that the LCOV text `lcov`, read from
/// `report`, measured in the workspace at `root`.
pub fn measure_coverage(
    root: &Path,
    report: &Path,
    lcov: &str,
) -> Result<CoverageReport, XtaskError> {
    // The report names files by their real path, and `root` may be written with `..` or
    // through a symlink.
    let root = fs::canonicalize(root).map_err(XtaskError::io(root))?;
    let records = parse_lcov(report, lcov)?;
    let crates = measured_crates(&root)?;
    let sources = read_sources(&root, &crates)?;
    let mut measured: BTreeMap<PathBuf, FileLines> = BTreeMap::new();
    for record in &records {
        merge(&root, &sources, record, &mut measured).map_err(|(line, error)| {
            XtaskError::InReport {
                report: report.to_path_buf(),
                line,
                error: Box::new(error),
            }
        })?;
    }
    // A file the report does not name has no figure: a crate whose tests never ran, or a
    // path cargo-llvm-cov drops by default (any with `/rustc/<hex>/`), would escape the bars.
    let gated = gated_sources(&sources);
    if let Some((path, _)) = sources.iter().find(|(path, source)| {
        source.has_function_body && !gated.contains(*path) && !measured.contains_key(*path)
    }) {
        return Err(XtaskError::Unreported {
            report: report.to_path_buf(),
            path: path.clone(),
        });
    }
    let required = crates
        .into_iter()
        .filter(|folder| layer_of(folder) == Some(Layer::Module))
        .chain([PathBuf::from(KERNEL_DIR)])
        .collect();
    Ok(CoverageReport {
        crates: figures(&sources, &measured),
        required,
    })
}

/// Adds the lines of `record` to those `measured` holds for its file. An error comes with
/// the line of the report it is about.
fn merge(
    root: &Path,
    sources: &BTreeMap<PathBuf, Source>,
    record: &Record,
    measured: &mut BTreeMap<PathBuf, FileLines>,
) -> Result<(), (usize, XtaskError)> {
    let Some(relative) = inside(root, &record.path) else {
        return Ok(());
    };
    let path = root.join(&relative);
    let kind = fs::symlink_metadata(&path)
        .map_err(|error| (record.at, XtaskError::io(&path)(error)))?
        .file_type();
    if !kind.is_file() {
        return Err((record.at, XtaskError::NotARegularFile { path }));
    }
    let source = match (place_of(&relative), sources.get(&relative)) {
        (Place::Unmeasured, _) => return Ok(()),
        (Place::Stray, _) => return Err((record.at, XtaskError::StrayFile { path: relative })),
        (Place::Source, Some(source)) => source,
        // cargo-llvm-cov leaves test modules out; a report that names one anyway still
        // measures test code there.
        (Place::Source, None) if is_test_module_file(&relative) => return Ok(()),
        (Place::Source, None) => {
            return Err((record.at, XtaskError::NotRead { path: relative }));
        }
    };
    if let Some(past) = record
        .lines
        .iter()
        .find(|line| line.line > source.line_count)
    {
        // The test items are found in the file as it is now, so the lines of a report
        // older than it would be the wrong ones.
        return Err((
            past.at,
            XtaskError::StaleReport {
                path: relative,
                line: past.line,
                length: source.line_count,
            },
        ));
    }
    let merged = measured.entry(relative).or_default();
    for line in &record.lines {
        *merged.entry(line.line).or_insert(false) |= line.hit;
    }
    Ok(())
}

/// The production lines of each measured crate that has any, by its folder.
fn figures(
    sources: &BTreeMap<PathBuf, Source>,
    measured: &BTreeMap<PathBuf, FileLines>,
) -> BTreeMap<PathBuf, Lines> {
    let mut figures: BTreeMap<PathBuf, Lines> = BTreeMap::new();
    for (file, lines) in measured {
        let (Some(source), Some(folder)) = (sources.get(file), crate_folder(file)) else {
            continue;
        };
        let counted = production_lines(source, lines);
        if counted.found > 0 {
            *figures.entry(folder.to_path_buf()).or_default() += counted;
        }
    }
    figures
}

/// Where a file of the report sits, as far as the measure is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// Under `target/`, `crates/testkit` or `crates/workspace-hack`, or the tests, benches,
    /// examples or build script of a crate.
    Unmeasured,
    /// Under the `src/` of a measured crate.
    Source,
    /// Anywhere else in the workspace: code included from there would escape every bar.
    Stray,
}

fn place_of(file: &Path) -> Place {
    if file.starts_with("target") {
        return Place::Unmeasured;
    }
    let Some(folder) = crate_folder(file) else {
        return Place::Stray;
    };
    if matches!(
        layer_of(folder),
        Some(Layer::Testkit | Layer::WorkspaceHack)
    ) {
        return Place::Unmeasured;
    }
    let inner = file.strip_prefix(folder).unwrap_or(file);
    if inner.starts_with("src") {
        Place::Source
    } else if ["tests", "benches", "examples"]
        .iter()
        .any(|folder| inner.starts_with(folder))
        || inner == Path::new("build.rs")
    {
        Place::Unmeasured
    } else {
        Place::Stray
    }
}

/// The folder of the crate that `file` is in, if any.
fn crate_folder(file: &Path) -> Option<&Path> {
    file.ancestors()
        .skip(1)
        .find(|folder| layer_of(folder).is_some())
}

/// A `tests.rs` file or a file under a `tests` folder, inside a crate's `src/`.
fn is_test_module_file(file: &Path) -> bool {
    let Some(inner) =
        crate_folder(file).and_then(|folder| file.strip_prefix(folder.join("src")).ok())
    else {
        return false;
    };
    inner.file_name().is_some_and(|name| name == "tests.rs")
        || inner
            .parent()
            .is_some_and(|folders| folders.iter().any(|folder| folder == "tests"))
}

/// `file` relative to `root`, which is canonical, or `None` when it is outside under both
/// its real spelling and the one the report gives. A report may reach the workspace
/// through a symlink (`/tmp` on macOS), so the folders are resolved; the file itself is not,
/// so that a symlink the report names is still seen as one.
fn inside(root: &Path, file: &Path) -> Option<PathBuf> {
    let real = resolved_folders(file);
    real.strip_prefix(root)
        .or_else(|_| file.strip_prefix(root))
        .ok()
        .map(Path::to_path_buf)
}

/// `file` with its nearest folder that exists resolved to its real path. A file that does
/// not exist keeps its place in the workspace, and is then an error, not a file outside.
fn resolved_folders(file: &Path) -> PathBuf {
    file.ancestors()
        .skip(1)
        .find_map(|folder| {
            let real = fs::canonicalize(folder).ok()?;
            Some(real.join(file.strip_prefix(folder).ok()?))
        })
        .unwrap_or_else(|| file.to_path_buf())
}

/// The lines of the report for `source` that are production lines.
fn production_lines(source: &Source, lines: &FileLines) -> Lines {
    let mut counted = Lines::default();
    for (line, hit) in lines {
        if !source.test_lines.contains(line) {
            counted += Lines {
                hit: u64::from(*hit),
                found: 1,
            };
        }
    }
    counted
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPORT: &str = "lcov.info";

    fn write(root: &Path, relative: &str, lines: &[&str]) -> PathBuf {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, lines.join("\n") + "\n").unwrap();
        path
    }

    /// A workspace whose crates each hold a `Cargo.toml`.
    fn workspace(crates: &[&str]) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        for folder in crates {
            write(root.path(), &format!("{folder}/Cargo.toml"), &["[package]"]);
        }
        root
    }

    /// A source of `count` production lines.
    fn plain_source(root: &Path, relative: &str, count: usize) -> PathBuf {
        write(root, relative, &vec!["pub fn production() {}"; count])
    }

    /// One LCOV record: `lines` are (line number, hits).
    fn record_of(file: &Path, lines: &[(usize, u64)]) -> String {
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

    fn measure(root: &Path, lcov: &str) -> Result<CoverageReport, XtaskError> {
        measure_coverage(root, Path::new(REPORT), lcov)
    }

    /// The error about what line `line` of the report names; any other error fails the test.
    fn at_report_line(error: &XtaskError, line: usize) -> &XtaskError {
        match error {
            XtaskError::InReport {
                report,
                line: at,
                error,
            } if report == Path::new(REPORT) && *at == line => error,
            other => panic!("no error at line {line} of the report: {other}"),
        }
    }

    fn figures(crates: &[(&str, u64, u64)]) -> BTreeMap<PathBuf, Lines> {
        crates
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
            .collect()
    }

    #[test]
    fn a_file_is_placed_by_the_crate_folder_and_the_part_of_it_it_is_in() {
        for (file, place) in [
            ("crates/kernel/src/money.rs", Place::Source),
            ("crates/modules/goals/src/domain/goal.rs", Place::Source),
            ("crates/adapters/http/src/envelope.rs", Place::Source),
            ("crates/bootstrap/src/logging.rs", Place::Source),
            ("crates/xtask/src/main.rs", Place::Source),
            ("apps/api/src/main.rs", Place::Source),
            ("crates/kernel/tests/x.rs", Place::Unmeasured),
            ("crates/kernel/benches/x.rs", Place::Unmeasured),
            ("apps/api/examples/demo.rs", Place::Unmeasured),
            ("apps/ingest/build.rs", Place::Unmeasured),
            ("crates/testkit/src/process.rs", Place::Unmeasured),
            ("crates/workspace-hack/src/lib.rs", Place::Unmeasured),
            ("target/debug/build/x/out/generated.rs", Place::Unmeasured),
            ("shared/logic.rs", Place::Stray),
            ("crates/misc/src/lib.rs", Place::Stray),
            ("crates/kernel/generated/x.rs", Place::Stray),
            ("crates/kernel/gen.rs", Place::Stray),
            ("targets/x.rs", Place::Stray),
            ("build.rs", Place::Stray),
        ] {
            assert_eq!(place_of(Path::new(file)), place, "{file}");
        }
    }

    #[test]
    fn a_test_module_file_is_known_by_its_name_or_its_folder() {
        for file in [
            "crates/kernel/src/tests.rs",
            "crates/kernel/src/money/tests.rs",
            "crates/kernel/src/tests/helpers.rs",
            "crates/kernel/src/money/tests/deep/x.rs",
        ] {
            assert!(is_test_module_file(Path::new(file)), "{file}");
        }
        for file in [
            "crates/kernel/src/money.rs",
            "crates/kernel/src/contests.rs",
            "crates/kernel/src/ab_tests.rs",
            "crates/kernel/tests/x.rs",
        ] {
            assert!(!is_test_module_file(Path::new(file)), "{file}");
        }
    }

    #[test]
    fn production_lines_are_counted_crate_by_crate_and_test_lines_are_left_out() {
        let root = workspace(&["crates/kernel", "crates/modules/goals", "apps/api"]);
        let lib = write(
            root.path(),
            "crates/kernel/src/lib.rs",
            &[
                "pub fn double(n: u32) -> u32 {",
                "    n * 2",
                "}",
                "#[cfg(test)]",
                "mod tests {",
                "    fn doubles() {}",
                "}",
            ],
        );
        let lcov = [
            record_of(&lib, &[(1, 4), (2, 0), (3, 4), (6, 1)]),
            record_of(
                &plain_source(root.path(), "crates/kernel/src/id.rs", 6),
                &lines_run(5, 6),
            ),
            record_of(
                &plain_source(root.path(), "crates/modules/goals/src/lib.rs", 2),
                &lines_run(1, 2),
            ),
            record_of(
                &plain_source(root.path(), "apps/api/src/main.rs", 3),
                &lines_run(0, 3),
            ),
        ]
        .concat();
        let measured = measure(root.path(), &lcov).unwrap();
        assert_eq!(
            measured.crates,
            figures(&[
                ("apps/api", 0, 3),
                ("crates/kernel", 7, 9),
                ("crates/modules/goals", 1, 2),
            ])
        );
        assert_eq!(
            measured.required,
            ["crates/kernel", "crates/modules/goals"]
                .map(PathBuf::from)
                .into()
        );
        assert_eq!(measured.total(), Lines { hit: 8, found: 14 });
    }

    #[test]
    fn what_is_outside_the_workspace_or_not_measured_is_ignored() {
        let outside = tempfile::tempdir().unwrap();
        let root = workspace(&["crates/kernel", "crates/testkit"]);
        let lcov = [
            record_of(
                &plain_source(root.path(), "crates/kernel/src/id.rs", 2),
                &lines_run(2, 2),
            ),
            record_of(
                &plain_source(outside.path(), "crates/kernel/src/id.rs", 9),
                &lines_run(0, 9),
            ),
            record_of(
                Path::new("/rustc/0123abcd/library/core/src/option.rs"),
                &lines_run(0, 9),
            ),
            record_of(
                &plain_source(root.path(), "crates/testkit/src/process.rs", 9),
                &lines_run(9, 9),
            ),
            record_of(
                &plain_source(root.path(), "target/debug/build/out/generated.rs", 9),
                &lines_run(0, 9),
            ),
            record_of(
                &plain_source(root.path(), "crates/kernel/tests/x.rs", 9),
                &lines_run(9, 9),
            ),
            record_of(
                &plain_source(root.path(), "crates/kernel/benches/x.rs", 9),
                &lines_run(9, 9),
            ),
        ]
        .concat();
        assert_eq!(
            measure(root.path(), &lcov).unwrap().crates,
            figures(&[("crates/kernel", 2, 2)])
        );
    }

    #[test]
    fn a_file_inside_the_workspace_but_outside_every_crate_src_is_an_error() {
        for stray in [
            "shared/logic.rs",
            "crates/kernel/generated/logic.rs",
            "crates/misc/src/lib.rs",
        ] {
            let root = workspace(&["crates/kernel"]);
            let file = plain_source(root.path(), stray, 1);
            let error = measure(root.path(), &record_of(&file, &[(1, 0)])).unwrap_err();
            assert!(
                matches!(at_report_line(&error, 1), XtaskError::StrayFile { path }
                    if path == Path::new(stray)),
                "{stray}: {error}"
            );
        }
    }

    #[test]
    fn a_file_under_src_that_was_not_read_is_an_error() {
        let root = workspace(&["crates/kernel"]);
        for not_read in [
            "crates/kernel/src/generated.inc",
            "crates/kernel/src/.hidden.rs",
            "crates/modules/ghost/src/lib.rs",
        ] {
            let file = plain_source(root.path(), not_read, 1);
            let error = measure(root.path(), &record_of(&file, &[(1, 0)])).unwrap_err();
            assert!(
                matches!(at_report_line(&error, 1), XtaskError::NotRead { path }
                    if path == Path::new(not_read)),
                "{not_read}: {error}"
            );
        }
    }

    #[test]
    fn a_test_module_file_the_report_names_anyway_is_not_measured() {
        let root = workspace(&["crates/kernel"]);
        let lcov = [
            record_of(
                &plain_source(root.path(), "crates/kernel/src/lib.rs", 1),
                &lines_run(1, 1),
            ),
            record_of(
                &plain_source(root.path(), "crates/kernel/src/tests.rs", 9),
                &lines_run(9, 9),
            ),
        ]
        .concat();
        assert_eq!(
            measure(root.path(), &lcov).unwrap().crates,
            figures(&[("crates/kernel", 1, 1)])
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_root_and_the_report_may_spell_the_workspace_through_a_symlink() {
        let real = workspace(&["crates/kernel"]);
        let links = tempfile::tempdir().unwrap();
        let link = links.path().join("workspace");
        std::os::unix::fs::symlink(real.path(), &link).unwrap();
        plain_source(real.path(), "crates/kernel/src/id.rs", 2);
        let through_the_link = record_of(&link.join("crates/kernel/src/id.rs"), &lines_run(1, 2));
        let expected = figures(&[("crates/kernel", 1, 2)]);
        assert_eq!(
            measure(real.path(), &through_the_link).unwrap().crates,
            expected
        );
        assert_eq!(measure(&link, &through_the_link).unwrap().crates, expected);
        let root_with_dots = link.join("crates/kernel/../..");
        assert_eq!(
            measure(&root_with_dots, &through_the_link).unwrap().crates,
            expected
        );
    }

    #[cfg(unix)]
    #[test]
    fn one_file_under_two_spellings_is_still_counted_once() {
        let real = workspace(&["crates/kernel"]);
        let links = tempfile::tempdir().unwrap();
        let link = links.path().join("workspace");
        std::os::unix::fs::symlink(real.path(), &link).unwrap();
        let file = plain_source(real.path(), "crates/kernel/src/id.rs", 3);
        let lcov = [
            record_of(&file.canonicalize().unwrap(), &[(1, 0), (2, 1), (3, 0)]),
            record_of(&link.join("crates/kernel/src/id.rs"), &[(1, 1), (2, 0)]),
        ]
        .concat();
        assert_eq!(
            measure(real.path(), &lcov).unwrap().crates,
            figures(&[("crates/kernel", 2, 3)])
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_missing_source_file_is_an_error_under_either_spelling() {
        let real = workspace(&["crates/kernel"]);
        plain_source(real.path(), "crates/kernel/src/lib.rs", 1);
        let links = tempfile::tempdir().unwrap();
        let link = links.path().join("workspace");
        std::os::unix::fs::symlink(real.path(), &link).unwrap();
        for gone in [
            real.path()
                .canonicalize()
                .unwrap()
                .join("crates/kernel/src/gone.rs"),
            link.join("crates/kernel/src/gone.rs"),
            link.join("crates/kernel/src/gone/deeper.rs"),
        ] {
            let error = measure(real.path(), &record_of(&gone, &lines_run(1, 1))).unwrap_err();
            assert!(
                matches!(at_report_line(&error, 1), XtaskError::Io { path, .. }
                    if path.file_name() == gone.file_name()),
                "{}: {error}",
                gone.display()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_a_fifo_or_a_folder_the_report_names_is_an_error() {
        let root = workspace(&["crates/kernel"]);
        plain_source(root.path(), "crates/kernel/src/lib.rs", 1);
        let real = root.path().canonicalize().unwrap();
        let link = real.join("crates/kernel/tests/link.rs");
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink("/etc/hosts", &link).unwrap();
        let fifo = real.join("crates/kernel/tests/fifo.rs");
        let made = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(made.success());
        for named in [link, fifo, real.join("crates/kernel/src")] {
            let error = measure(root.path(), &record_of(&named, &[(1, 1)])).unwrap_err();
            assert!(
                matches!(at_report_line(&error, 1), XtaskError::NotARegularFile { path }
                    if *path == named),
                "{}: {error}",
                named.display()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_file_reached_through_a_link_to_outside_is_judged_where_the_report_spells_it() {
        let root = workspace(&["crates/kernel"]);
        let elsewhere = tempfile::tempdir().unwrap();
        plain_source(elsewhere.path(), "logic.rs", 1);
        std::os::unix::fs::symlink(elsewhere.path(), root.path().join("shared")).unwrap();
        let spelled = root.path().canonicalize().unwrap().join("shared/logic.rs");
        let error = measure(root.path(), &record_of(&spelled, &[(1, 0)])).unwrap_err();
        assert!(
            matches!(at_report_line(&error, 1), XtaskError::StrayFile { path }
                if path == Path::new("shared/logic.rs")),
            "{error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_the_report_names_is_refused_even_when_it_points_inside() {
        let root = workspace(&["crates/kernel"]);
        plain_source(root.path(), "crates/kernel/src/lib.rs", 1);
        let real = root.path().canonicalize().unwrap();
        let link = real.join("crates/kernel/tests/copy.rs");
        fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink("../src/lib.rs", &link).unwrap();
        let error = measure(root.path(), &record_of(&link, &[(1, 1)])).unwrap_err();
        assert!(
            matches!(at_report_line(&error, 1), XtaskError::NotARegularFile { path }
                if *path == link),
            "{error}"
        );
    }

    #[test]
    fn a_source_with_a_function_body_must_be_named_by_the_report() {
        let root = workspace(&["crates/kernel", "crates/adapters/ghost"]);
        let lcov = record_of(
            &plain_source(root.path(), "crates/kernel/src/lib.rs", 1),
            &lines_run(1, 1),
        );
        write(
            root.path(),
            "crates/kernel/src/types.rs",
            &["pub struct Price(u64);", "pub use std::fs;"],
        );
        write(
            root.path(),
            "crates/kernel/src/fixtures.rs",
            &["#[cfg(test)]", "pub fn fixture() {}"],
        );
        assert!(measure(root.path(), &lcov).is_ok());
        write(
            root.path(),
            "crates/adapters/ghost/src/lib.rs",
            &["pub fn never_tested() {}"],
        );
        let error = measure(root.path(), &lcov).unwrap_err();
        assert!(
            matches!(&error, XtaskError::Unreported { report, path }
                if report == Path::new(REPORT)
                    && path == Path::new("crates/adapters/ghost/src/lib.rs")),
            "{error}"
        );
    }

    #[test]
    fn a_source_a_cfg_may_compile_out_need_not_be_named_by_the_report() {
        let root = workspace(&["crates/kernel"]);
        let lib = write(
            root.path(),
            "crates/kernel/src/lib.rs",
            &[
                "pub fn tested() {}",
                "#[cfg(windows)]",
                "mod win;",
                "pub mod plain;",
            ],
        );
        let lcov = record_of(&lib, &[(1, 1)]);
        plain_source(root.path(), "crates/kernel/src/win.rs", 1);
        write(
            root.path(),
            "crates/kernel/src/plain.rs",
            &["#[cfg(feature = \"extra\")]", "pub fn extra() {}"],
        );
        assert!(measure(root.path(), &lcov).is_ok());
        write(
            root.path(),
            "crates/kernel/src/plain.rs",
            &["pub fn plain() {}"],
        );
        let error = measure(root.path(), &lcov).unwrap_err();
        assert!(
            matches!(&error, XtaskError::Unreported { path, .. }
                if path == Path::new("crates/kernel/src/plain.rs")),
            "{error}"
        );
    }

    #[test]
    fn a_file_in_several_records_counts_each_line_once_and_as_run_if_any_record_ran_it() {
        let root = workspace(&["crates/kernel"]);
        let file = plain_source(root.path(), "crates/kernel/src/id.rs", 4);
        let lcov = [
            record_of(&file, &[(1, 0), (2, 3), (3, 0)]),
            record_of(&file, &[(1, 2), (2, 0), (3, 0), (4, 0)]),
        ]
        .concat();
        assert_eq!(
            measure(root.path(), &lcov).unwrap().crates,
            figures(&[("crates/kernel", 2, 4)])
        );
    }

    #[test]
    fn a_line_past_the_end_of_its_file_is_an_error_that_says_so() {
        let root = workspace(&["crates/kernel"]);
        let file = plain_source(root.path(), "crates/kernel/src/id.rs", 3);
        let error = measure(root.path(), &record_of(&file, &[(3, 1), (4, 1)])).unwrap_err();
        assert!(
            matches!(at_report_line(&error, 3), XtaskError::StaleReport { path, line: 4, length: 3 }
                if path == Path::new("crates/kernel/src/id.rs")),
            "{error}"
        );
    }

    #[test]
    fn a_root_that_does_not_exist_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        let error = measure(&root.path().join("missing"), "").unwrap_err();
        assert!(matches!(error, XtaskError::Io { .. }), "{error}");
    }

    #[test]
    fn a_report_file_is_read_with_its_limits_and_measured() {
        let root = workspace(&["crates/kernel"]);
        let report = root.path().join(REPORT);
        let error = measure_coverage_file(root.path(), &report).unwrap_err();
        assert!(
            matches!(&error, XtaskError::Io { path, .. } if *path == report),
            "{error}"
        );
        fs::File::create(&report)
            .unwrap()
            .set_len(REPORT_LIMIT + 1)
            .unwrap();
        let error = measure_coverage_file(root.path(), &report).unwrap_err();
        assert!(
            matches!(&error, XtaskError::TooLarge { path, .. } if *path == report),
            "{error}"
        );
        let file = plain_source(root.path(), "crates/kernel/src/id.rs", 2);
        fs::write(&report, record_of(&file, &lines_run(2, 2))).unwrap();
        assert_eq!(
            measure_coverage_file(root.path(), &report).unwrap().crates,
            figures(&[("crates/kernel", 2, 2)])
        );
    }

    #[test]
    fn a_figure_is_cut_and_never_rounded() {
        let lines = |hit, found| Lines { hit, found }.to_string();
        assert_eq!(lines(94_999, 100_000), "94.9% (94999/100000)");
        assert_eq!(lines(220, 226), "97.3% (220/226)");
        assert_eq!(lines(0, 0), "no line was measured");
        assert_eq!(
            lines(u64::MAX - 1, u64::MAX),
            format!("99.9% ({}/{})", u64::MAX - 1, u64::MAX)
        );
    }

    #[test]
    fn a_crate_folder_is_printed_escaped_in_the_summary_and_the_failures() {
        let folder = "crates/modules/\u{1b}[2Jgoals";
        let root = workspace(&["crates/kernel", folder]);
        let lcov = [
            record_of(
                &plain_source(root.path(), "crates/kernel/src/lib.rs", 1),
                &lines_run(1, 1),
            ),
            record_of(
                &plain_source(root.path(), &format!("{folder}/src/lib.rs"), 2),
                &lines_run(1, 2),
            ),
        ]
        .concat();
        let measured = measure(root.path(), &lcov).unwrap();
        let printed = [
            measured.summary(),
            judge(&measured).iter().map(ToString::to_string).collect(),
        ]
        .concat();
        assert!(
            printed.iter().any(|line| line.contains("\\u{1b}[2Jgoals")),
            "{printed:?}"
        );
        assert!(
            !printed.iter().any(|line| line.contains('\u{1b}')),
            "{printed:?}"
        );
    }

    #[test]
    fn the_summary_has_one_line_per_crate_and_the_total() {
        let report = CoverageReport {
            crates: figures(&[
                ("crates/kernel", 220, 226),
                ("apps/api", 0, 3),
                ("crates/xtask", 3, 3),
            ]),
            required: BTreeSet::new(),
        };
        assert_eq!(
            report.summary(),
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
