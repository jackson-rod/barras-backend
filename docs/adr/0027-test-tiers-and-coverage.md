# ADR-0027: Test tiers and coverage

Status: Accepted
Date: 2026-11-16

## Context

- ADR-0017 sets the coverage thresholds, 95% of lines in module crates and 85% overall. The foundation design names `just test`, `just test-int` and `just test-cov` but not which test runs where, nor what counts as a line.
- cargo-llvm-cov counts the inline `#[cfg(test)]` modules as covered lines. They were about half of the lines when BAR-3 started, and the tests always run them, so they would lift every figure.
- The owner approved the BAR-3 spec, which holds the tiers and how coverage is measured, on 2026-11-15 (spec §9, §12). Its **Calls** are accepted with it: the tiers go by file name, only production lines count, and the kernel is held to the modules' bar.
- How `check-coverage` reads the sources, the shapes it refuses and the limits accepted below are **Calls of the implementation**. The reviews and security audits of the gate while building BAR-3, after that approval, found ways to pass it wrongly and rules that refused honest code; this record holds where their fixes ended.

## Decision

### Tiers

- A test target's file name decides its tier, so a new test has nothing to register:
  - `tests/e2e_*.rs` drives a whole binary: its app in-process, or its real process.
  - `tests/int_*.rs` drives one adapter against a real outside system, from BAR-4.
  - Every other test, unit tests and the integration tests that need nothing outside the process, is in neither.
- `just test` runs every test target not named `e2e_` or `int_`, then the doctests, and needs no Docker. `just test-int` runs the `e2e_` and `int_` targets; from BAR-4 it needs Docker. `just test-cov` runs every test once, both tiers, under coverage, then the thresholds.
- A test still running after 30 seconds is reported as slow; after two minutes it is killed and fails under its own name (`.config/nextest.toml`).

### What is measured

- cargo-llvm-cov 0.9.1 runs every test of the workspace once, through cargo-nextest, and writes the LCOV report afresh on every run (`target/coverage/lcov.info`). Nothing is taken out of the measure: no `-p`, `--exclude` or ignore pattern. `--no-cfg-coverage` leaves `cfg(coverage)` unset, so no code can be compiled out of the instrumented build.
- Measured: the files under the `src/` of `crates/kernel`, of each `crates/modules/<m>`, `crates/adapters/<a>` and `apps/<a>`, of `crates/bootstrap` and of `crates/xtask`. Not measured: `crates/testkit`, `crates/workspace-hack`, and a crate's `tests/`, `benches/`, `examples/` and `build.rs`.
- Only production lines count. An item that carries exactly `#[cfg(test)]` is test code at any depth: in a file, in an inline module, in an `impl`, `trait` or `extern` block. The lines of its whole span, its attributes and doc comments included, are left out, except a line it shares with production code. Test support under `#[cfg(any(test, feature = "…"))]` counts as production.
- A binary that a process test starts is measured: the testkit's process helper passes `LLVM_PROFILE_FILE` on to it. `just test-cov` fails, before the thresholds, when a `.profraw` file lies outside `target/`: the sign of a child process that lost the variable, whose lines then count as never run.
- Doctests run (`just test-doc`) outside the measure: instrumenting them needs a nightly toolchain.

### The bars

- `cargo xtask check-coverage <report>` groups the production lines by crate folder and holds them to their bars: 95% for `crates/kernel` and for each `crates/modules/<m>`, each on its own, and 85% for the measured crates together. It compares in integers, and prints each figure cut, not rounded, so 94.99% never reads as 95.0%.
- `crates/kernel` and every module crate must have a figure: absent from the report, or present with no production line, fails. A report that measures no production line fails.
- It prints one line per crate and the total, then every failure, and exits 1 on any.

### How `check-coverage` reads the sources

- The report is parsed strictly. A line that is not a known LCOV record, a record out of place, a report cut short and a line past the end of its source file are errors that name the report's line. A file of the report inside the workspace but outside every crate's `src/` and outside `target/` is an error; one outside the workspace is ignored.
- A `tests.rs` file or a `tests/` folder inside `src/` is a test module, which cargo-llvm-cov leaves out of the report. It is read only for `include` and `#[path]`, so it cannot pull production code into a test build.
- Every other `.rs` file under each measured `src/`, hidden ones aside, is parsed with `syn`, whether the report names it or not, and a file that does not parse is an error. A report over 64 MiB, a source over 1 MiB, a symlink and an entry that is neither a file nor a folder are refused.
- A source that holds a function body (a function, a method, a default trait method or a closure) outside test code must be named by the report, so a crate whose tests never ran, or a path cargo-llvm-cov drops, cannot escape the bars. A file is exempt only when every `mod` line that declares it carries a cfg other than the test mark, which may compile it out of the measured build. A crate root is never exempt.
- An adapter's tests that need its `testing` feature live in `src/tests/`, declared `#[cfg(test)] mod tests;`: a test under `tests/` would get the feature only through a dev-dependency of the crate on itself, which `check-arch` refuses (`adapter-isolation`).
- These shapes are refused, each naming the file and the line, because each would hide code from the measure or count test code as production. Outside test code:
  - `#[cfg(test)]` on anything but an item, and a `cfg` predicate that names `test` in another form than `test` or `any(test, feature = "…")`, such as `not(test)`;
  - a `cfg` or `cfg_attr` predicate built from anything but `test`, `feature = "…"`, `debug_assertions`, `unix`, `windows` and the `target_*` keys, combined with `all`, `any` and `not`, or with `not` around a feature or `debug_assertions`: code that only a release build, a custom cfg or a missing feature compiles is never measured;
  - a predicate that names `coverage`, a `coverage(…)` attribute, and a `cfg_attr` that applies `cfg`;
  - a test attribute (`#[test]`, `#[tokio::test]`, any attribute whose last name is `test`) outside a `#[cfg(test)]` item;
  - a module named `tests`, or an out-of-line one named `*_tests`, without `#[cfg(test)]`: cargo-llvm-cov drops the files of such modules, so production code in them would escape;
  - in a `macro_rules!` body, the keyword `mod` and a test attribute: macros are not expanded, so the module or the test it writes would escape the rules above.
- Anywhere, test code included:
  - `#[path]` on a module;
  - `include!`, except `include!(concat!(env!("OUT_DIR"), "…"))` with no `..`, and a `use` whose path names `include`: an included file is measured, or dropped, under its own path. A field, a method or a variable named `include` is fine;
  - an out-of-line `#[cfg(test)]` module not named `tests`, and an out-of-line module inside test code that is not named `tests` and does not sit in a module named `tests`: `check-coverage` would read its file as production;
  - under a measured `src/`, a file named `*_tests.rs` or `*-tests.rs` and a folder named `examples` or `benches`, which cargo-llvm-cov leaves out of the report;
  - brackets nested deeper than 256 levels.
- `check-arch` refuses a build script in the kernel and in a module (`no-build-script`, ADR-0024): the code it generated would escape the measure.

### What the gates guard against

- `check-coverage` and `check-arch` must not pass by mistake: a shape an honest contributor, human or agent, could write that hides code from the measure or slips a layer rule is refused or measured.
- They do not guard against a hostile author. The change's own code runs in the same CI job before the gates finish, a test or a build script can rewrite the report or the tool, and the gate files belong to the change. The owner reads every change to them; CODEOWNERS on them is an open owner decision.

### Accepted limits

- A stale report is caught only when it names a line past a file's end, so `just test-cov` always writes it afresh. The report comes from the change's own code and can be forged.
- The total averages across crates. Trivially covered production lines, and test support under `any(test, feature = "…")`, count as production: only review sees padding.
- Code generated under `target/` and code a macro expands are not measured as such. A dependency's procedural macro can emit any attribute; only adapters and apps can take such a dependency.
- Code compiled out of the measured build, under a feature the workspace leaves off or for another platform, is measured only where it is compiled, on CI's Linux or a developer's Mac; a file gated that way may be absent from the report.
- Only `macro_rules!` bodies are judged for `mod` and test attributes, never macro calls, and neither `cfg!` nor a `#[$meta]` passthrough is judged: review covers a macro used to hide code. The rules on attributes and on `include` read every token, macro bodies and calls included.
- Test attributes not named `test`, such as `#[rstest]` and `#[test_case]`, are not recognised as test marks; the kernel's and the modules' dev dependencies allow none of them.
- A file swapped for a FIFO between the check and the open is a residual race.
- The 256-level limit guards nested brackets only: a long chain without brackets (`!!!x`, deep generics) can still exhaust the parse stack, which stops the gate, and so fails it, without naming the file.
- Hostile limits. A build script that prints `cargo::rustc-cfg=feature="testing"` compiles test support into a build that `testing-dev-only` (ADR-0024) sees as free of it. A pull request can poison its own later runs through the CI compile cache: sccache only reads the cache on pull requests (`SCCACHE_GHA_RW_MODE`), but the action hands its cache token to every later step, so the run on `develop` after the merge is the authoritative result. The CI tree check catches honest writes, not a hostile build script, which can undo its writes before the check runs.

## Consequences

- Cost: `just verify` builds the workspace three ways, for the checks, for the tests under coverage and for the release.
- A crate whose tests do not run (`[lib] test = false`) fails the gate, since every source with a function body must be named by the report.
- Test code is inline under `#[cfg(test)]`, in a `tests.rs` file or under `src/tests/`; anything else is refused or measured as production.
- The two nextest filters are valid only while at least one `e2e_` or `int_` test target exists: nextest refuses a `binary()` filter that matches no test target.
- CI's `test` job runs `just test-cov`, and `just verify` runs it before every push.
- ADR-0017 keeps its thresholds; this record says how they are measured.
