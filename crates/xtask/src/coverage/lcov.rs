//! The LCOV report, read by hand and strictly: anything malformed is an error, never a
//! record skipped.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use crate::error::XtaskError;

/// Line number to "a test ran it".
pub type FileLines = BTreeMap<usize, bool>;

/// A line counts once, and as run if any record ran it.
pub fn record(lines: &mut FileLines, line: usize, hit: bool) {
    *lines.entry(line).or_insert(false) |= hit;
}

/// The `DA` records of the LCOV text of the report at `report`, by the source file their
/// `SF` record names. Every other record repeats or sums these, or describes functions and
/// branches.
pub fn parse_lcov(report: &Path, lcov: &str) -> Result<BTreeMap<PathBuf, FileLines>, XtaskError> {
    let mut files: BTreeMap<PathBuf, FileLines> = BTreeMap::new();
    let mut open: Option<PathBuf> = None;
    let mut last = 0;
    for (index, text) in lcov.lines().enumerate() {
        last = index + 1;
        let malformed = |problem| XtaskError::Lcov {
            report: report.to_path_buf(),
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
            report: report.to_path_buf(),
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

#[cfg(test)]
mod tests {
    use super::*;

    const REPORT: &str = "target/coverage/lcov.info";

    fn text(lines: &[&str]) -> String {
        lines.join("\n") + "\n"
    }

    fn parse(lcov: &str) -> Result<BTreeMap<PathBuf, FileLines>, XtaskError> {
        parse_lcov(Path::new(REPORT), lcov)
    }

    /// One LCOV record: `lines` are (line number, hits).
    fn one_record(file: &str, lines: &[(usize, u64)]) -> String {
        let mut record = format!("SF:{file}\n");
        for (line, hits) in lines {
            record.push_str(&format!("DA:{line},{hits}\n"));
        }
        record.push_str("end_of_record\n");
        record
    }

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
        assert_eq!(
            parse(&lcov).unwrap(),
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
        let file = "/work/crates/kernel/src/id.rs";
        let lcov = [
            one_record(file, &[(1, 0), (2, 3), (3, 0)]),
            one_record("/work/apps/api/src/main.rs", &[(1, 1)]),
            one_record(file, &[(1, 2), (2, 0), (3, 0), (4, 0)]),
        ]
        .concat();
        assert_eq!(
            parse(&lcov).unwrap()[Path::new(file)],
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
            parse(&lcov).unwrap()[Path::new("/work/a.rs")],
            BTreeMap::from([(9, true)])
        );
    }

    #[test]
    fn a_malformed_record_is_an_error_that_names_the_report_and_its_line() {
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
            (
                &[
                    "SF:/work/a.rs",
                    "DA:1,18446744073709551616",
                    "end_of_record",
                ],
                2,
            ),
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
            let error = parse(&text(lcov)).unwrap_err();
            assert!(
                matches!(&error, XtaskError::Lcov { report, line: found, .. }
                    if *found == line && report == Path::new(REPORT)),
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
            let error = parse(&text(lcov)).unwrap_err();
            assert!(
                matches!(error, XtaskError::Lcov { line: found, .. } if found == line),
                "{lcov:?}: {error}"
            );
        }
    }

    #[test]
    fn an_empty_text_holds_no_record() {
        assert_eq!(parse("").unwrap(), BTreeMap::new());
        assert_eq!(parse("TN:\n\n").unwrap(), BTreeMap::new());
    }

    #[test]
    fn windows_line_endings_change_nothing() {
        let lcov = text(&["SF:/work/a.rs", "DA:2,1", "end_of_record"]).replace('\n', "\r\n");
        assert_eq!(
            parse(&lcov).unwrap()[Path::new("/work/a.rs")],
            BTreeMap::from([(2, true)])
        );
    }
}
