//! The LCOV report, read by hand and strictly: anything malformed is an error, never a
//! record skipped.

use std::path::{Component, Path, PathBuf};

use crate::error::XtaskError;

/// The records a report may hold besides `SF`, `DA` and `end_of_record`, which are read.
/// cargo-llvm-cov 0.9.1 writes `FN`, `FNDA`, `FNF`, `FNH`, `BRF`, `BRH`, `LF` and `LH`; the
/// rest are LCOV's own. Each repeats or sums the line records, or describes functions and
/// branches.
const OTHER_RECORDS: [&str; 11] = [
    "TN:", "VER:", "FN:", "FNDA:", "FNF:", "FNH:", "BRDA:", "BRF:", "BRH:", "LF:", "LH:",
];

/// One `SF` record: the source file it names, and its `DA` records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub path: PathBuf,
    /// The line of the report that names the file.
    pub at: usize,
    pub lines: Vec<LineRecord>,
}

/// One `DA` record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineRecord {
    /// The line of the source file, numbered from 1.
    pub line: usize,
    pub hit: bool,
    /// The line of the report that holds the record.
    pub at: usize,
}

/// The records of the LCOV text of the report at `report`, in the order it gives them.
pub fn parse_lcov(report: &Path, lcov: &str) -> Result<Vec<Record>, XtaskError> {
    let mut records = Vec::new();
    let mut open: Option<Record> = None;
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
            let path = source_path(path).ok_or_else(|| {
                malformed("the path of a source file must be absolute and without `..`")
            })?;
            open = Some(Record {
                path,
                at: last,
                lines: Vec::new(),
            });
        } else if let Some(data) = text.strip_prefix("DA:") {
            let record = open
                .as_mut()
                .ok_or_else(|| malformed("a line record outside a file record"))?;
            let (line, hits) = line_record(data)
                .ok_or_else(|| malformed("a line record must be `DA:<line>,<hits>`"))?;
            record.lines.push(LineRecord {
                line,
                hit: hits > 0,
                at: last,
            });
        } else if text == "end_of_record" {
            let record = open
                .take()
                .ok_or_else(|| malformed("a record ends that never started"))?;
            records.push(record);
        } else if !text.is_empty() && !OTHER_RECORDS.iter().any(|kind| text.starts_with(kind)) {
            // A line record written any other way, indented or in lower case, would drop
            // its line from the measure.
            return Err(malformed(
                "a line that is not an LCOV record, which starts at column 0 with its upper-case name",
            ));
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
    Ok(records)
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

    fn parse(lcov: &str) -> Result<Vec<Record>, XtaskError> {
        parse_lcov(Path::new(REPORT), lcov)
    }

    /// The (source line, hit) pairs of each record, by the path it names.
    fn lines_of(records: &[Record]) -> Vec<(&str, Vec<(usize, bool)>)> {
        records
            .iter()
            .map(|record| {
                (
                    record.path.to_str().unwrap(),
                    record
                        .lines
                        .iter()
                        .map(|line| (line.line, line.hit))
                        .collect(),
                )
            })
            .collect()
    }

    #[test]
    fn the_line_records_are_read_and_every_other_record_is_ignored() {
        let lcov = text(&[
            "TN:",
            "VER:2.0",
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
            "",
            "SF:/work/apps/api/src/main.rs",
            "DA:1,0",
            "end_of_record",
        ]);
        assert_eq!(
            lines_of(&parse(&lcov).unwrap()),
            [
                (
                    "/work/crates/kernel/src/money.rs",
                    vec![(3, true), (4, false), (5, true)]
                ),
                ("/work/apps/api/src/main.rs", vec![(1, false)]),
            ]
        );
    }

    #[test]
    fn each_record_keeps_the_report_lines_of_its_file_and_of_its_lines() {
        let lcov = text(&[
            "TN:",
            "SF:/work/a.rs",
            "DA:9,1",
            "LF:1",
            "DA:10,0",
            "end_of_record",
        ]);
        assert_eq!(
            parse(&lcov).unwrap(),
            [Record {
                path: PathBuf::from("/work/a.rs"),
                at: 2,
                lines: vec![
                    LineRecord {
                        line: 9,
                        hit: true,
                        at: 3
                    },
                    LineRecord {
                        line: 10,
                        hit: false,
                        at: 5
                    },
                ],
            }]
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
            lines_of(&parse(&lcov).unwrap()),
            [("/work/a.rs", vec![(9, true)])]
        );
    }

    fn assert_malformed_at(lcov: &[&str], line: usize) {
        let error = parse(&text(lcov)).unwrap_err();
        assert!(
            matches!(&error, XtaskError::Lcov { report, line: found, .. }
                if *found == line && report == Path::new(REPORT)),
            "{lcov:?}: {error}"
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
            assert_malformed_at(lcov, line);
        }
    }

    #[test]
    fn a_line_that_is_no_record_is_an_error_that_names_the_report_and_its_line() {
        for unknown in [
            " DA:2,0",
            "\tDA:2,0",
            "da:2,0",
            "Da:2,0",
            "DA :2,0",
            "\u{feff}DA:2,0",
            "end_of_record ",
            " ",
            "garbage",
            "LINES:2",
        ] {
            assert_malformed_at(&["SF:/work/a.rs", "DA:1,1", unknown, "end_of_record"], 3);
        }
        assert_malformed_at(&["\u{feff}SF:/work/a.rs", "DA:1,1", "end_of_record"], 1);
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
        ] {
            assert_malformed_at(lcov, line);
        }
        assert_malformed_at(&["SF:/work/a.rs", "DA:1,1", "end_of_recor"], 3);
    }

    #[test]
    fn an_empty_text_holds_no_record() {
        assert_eq!(parse("").unwrap(), []);
        assert_eq!(parse("TN:\n\n").unwrap(), []);
    }

    #[test]
    fn windows_line_endings_change_nothing() {
        let lcov = text(&["SF:/work/a.rs", "DA:2,1", "end_of_record"]).replace('\n', "\r\n");
        assert_eq!(
            lines_of(&parse(&lcov).unwrap()),
            [("/work/a.rs", vec![(2, true)])]
        );
    }
}
