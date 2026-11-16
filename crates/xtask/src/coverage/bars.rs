//! The verdict: each crate held to its bar, the measured crates held to theirs.

use std::fmt;
use std::path::{Path, PathBuf};

use super::{CoverageReport, Lines};
use crate::arch::{Layer, layer_of};
use crate::error::escaped;

/// Percent of production lines, for `crates/kernel` and for each `crates/modules/<m>`.
const CRATE_BAR: u64 = 95;
/// Percent of production lines, for the measured crates together.
const TOTAL_BAR: u64 = 85;

/// Why a report fails its bars.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BarFailure {
    /// `folder` is `total` for the measured crates together.
    UnderBar {
        folder: PathBuf,
        hit: u64,
        found: u64,
        bar: u64,
    },
    /// A crate that must have a figure has no production line in the report.
    NotInReport { folder: PathBuf },
    /// The report measured no production line at all: a run that went wrong, not a pass.
    NothingMeasured,
}

impl fmt::Display for BarFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnderBar {
                folder,
                hit,
                found,
                bar,
            } => {
                let lines = Lines {
                    hit: *hit,
                    found: *found,
                };
                write!(f, "{}: {lines} is under its bar of {bar}%", escaped(folder))
            }
            Self::NotInReport { folder } => write!(
                f,
                "{}: the report has no production line of it, and it must have a figure",
                escaped(folder)
            ),
            Self::NothingMeasured => f.write_str("total: the report measured no production line"),
        }
    }
}

/// Every bar `report` fails; empty when it passes.
pub fn judge(report: &CoverageReport) -> Vec<BarFailure> {
    let mut failures = Vec::new();
    for (folder, lines) in &report.crates {
        if has_its_own_bar(folder) && !lines.meets(CRATE_BAR) {
            failures.push(under_bar(folder, *lines, CRATE_BAR));
        }
    }
    for folder in &report.required {
        if !report
            .crates
            .get(folder)
            .is_some_and(|lines| lines.found > 0)
        {
            failures.push(BarFailure::NotInReport {
                folder: folder.clone(),
            });
        }
    }
    let total = report.total();
    if total.found == 0 {
        failures.push(BarFailure::NothingMeasured);
    } else if !total.meets(TOTAL_BAR) {
        failures.push(under_bar(Path::new("total"), total, TOTAL_BAR));
    }
    failures
}

fn under_bar(folder: &Path, lines: Lines, bar: u64) -> BarFailure {
    BarFailure::UnderBar {
        folder: folder.to_path_buf(),
        hit: lines.hit,
        found: lines.found,
        bar,
    }
}

fn has_its_own_bar(folder: &Path) -> bool {
    matches!(layer_of(folder), Some(Layer::Kernel | Layer::Module))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    const KERNEL: &str = "crates/kernel";

    /// A report of these figures that must have one for the kernel only.
    fn report(crates: &[(&str, u64, u64)]) -> CoverageReport {
        report_requiring(crates, &[KERNEL])
    }

    fn report_requiring(crates: &[(&str, u64, u64)], required: &[&str]) -> CoverageReport {
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
            required: required.iter().map(PathBuf::from).collect::<BTreeSet<_>>(),
        }
    }

    fn under(folder: &str, hit: u64, found: u64, bar: u64) -> BarFailure {
        BarFailure::UnderBar {
            folder: PathBuf::from(folder),
            hit,
            found,
            bar,
        }
    }

    #[test]
    fn the_kernel_passes_at_exactly_its_bar_and_fails_one_line_under() {
        let rest = ("apps/api", 1000, 1000);
        assert_eq!(judge(&report(&[(KERNEL, 950, 1000), rest])), []);
        assert_eq!(
            judge(&report(&[(KERNEL, 949, 1000), rest])),
            [under(KERNEL, 949, 1000, 95)]
        );
    }

    #[test]
    fn each_module_is_held_to_the_bar_on_its_own() {
        let found = report(&[
            (KERNEL, 100, 100),
            ("crates/modules/capital", 19, 20),
            ("crates/modules/goals", 94, 100),
            ("crates/modules/identity", 18, 20),
        ]);
        assert_eq!(
            judge(&found),
            [
                under("crates/modules/goals", 94, 100, 95),
                under("crates/modules/identity", 18, 20, 95),
            ]
        );
    }

    #[test]
    fn the_total_passes_at_exactly_its_bar_and_fails_one_line_under() {
        assert_eq!(
            judge(&report(&[("apps/api", 700, 1000), (KERNEL, 1000, 1000)])),
            []
        );
        assert_eq!(
            judge(&report(&[("apps/api", 699, 1000), (KERNEL, 1000, 1000)])),
            [under("total", 1699, 2000, 85)]
        );
    }

    #[test]
    fn a_crate_outside_the_kernel_and_the_modules_has_no_bar_of_its_own() {
        let found = report(&[
            ("apps/api", 0, 10),
            ("crates/adapters/http", 0, 10),
            ("crates/bootstrap", 0, 10),
            ("crates/xtask", 0, 10),
            (KERNEL, 1000, 1000),
        ]);
        assert_eq!(judge(&found), []);
    }

    #[test]
    fn both_bars_can_fail_at_once() {
        let found = report(&[("apps/api", 0, 100), (KERNEL, 94, 100)]);
        assert_eq!(
            judge(&found),
            [under(KERNEL, 94, 100, 95), under("total", 94, 200, 85)]
        );
    }

    #[test]
    fn a_crate_that_must_have_a_figure_fails_without_one() {
        let goals = "crates/modules/goals";
        let found = report_requiring(&[("apps/api", 10, 10)], &[KERNEL, goals]);
        assert_eq!(
            judge(&found),
            [
                BarFailure::NotInReport {
                    folder: PathBuf::from(KERNEL)
                },
                BarFailure::NotInReport {
                    folder: PathBuf::from(goals)
                },
            ]
        );
        let without_production =
            report_requiring(&[(KERNEL, 0, 0), ("apps/api", 10, 10)], &[KERNEL]);
        assert_eq!(
            judge(&without_production),
            [BarFailure::NotInReport {
                folder: PathBuf::from(KERNEL)
            }]
        );
    }

    #[test]
    fn a_report_that_measured_nothing_fails() {
        assert_eq!(
            judge(&report_requiring(&[], &[])),
            [BarFailure::NothingMeasured]
        );
        assert_eq!(
            judge(&report(&[])),
            [
                BarFailure::NotInReport {
                    folder: PathBuf::from(KERNEL)
                },
                BarFailure::NothingMeasured,
            ]
        );
    }

    #[test]
    fn the_bars_are_compared_without_rounding() {
        assert_eq!(
            judge(&report(&[(KERNEL, 94_999, 100_000)])),
            [under(KERNEL, 94_999, 100_000, 95)]
        );
        let huge = u64::MAX;
        assert_eq!(judge(&report(&[(KERNEL, huge, huge)])), []);
    }
}
