//! The layer rule: who may depend on whom, checked against `cargo metadata`.

mod hack;
mod manifests;
mod metadata;
mod rules;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cargo_metadata::TargetKind;

pub use metadata::load_workspace;
pub use rules::{check_arch, expected_name};

use crate::error::XtaskError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepKind {
    Normal,
    Dev,
    Build,
}

/// Where a dependency comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// A path dependency.
    Path,
    CratesIo,
    /// A git repository or a registry other than crates.io.
    Elsewhere,
}

impl Origin {
    /// Classifies the `source` that `cargo metadata` reports, which is absent for a path dependency.
    pub fn of(source: Option<&cargo_metadata::Source>) -> Self {
        match source {
            None => Self::Path,
            Some(source) if source.is_crates_io() => Self::CratesIo,
            Some(_) => Self::Elsewhere,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Dep {
    /// The real package name, never the local rename.
    pub name: String,
    /// The key the manifest gives it when that differs from `name`: the name a
    /// `[features]` entry calls it by.
    pub rename: Option<String>,
    pub kind: DepKind,
    pub origin: Origin,
    /// The folder of a path dependency: relative to the workspace root when it is inside
    /// it, as cargo reports it otherwise.
    pub path: Option<PathBuf>,
    /// The features this dependency turns on, those of a `[workspace.dependencies]` entry
    /// it inherits included.
    pub features: Vec<String>,
}

/// What a target of a package is, as far as `hack-is-a-stub` is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetRole {
    /// A library of the plain `lib` crate type.
    Library,
    BuildScript,
    /// A binary, an example, a test, a bench, or a library of another crate type.
    Other,
}

impl TargetRole {
    pub fn of(kinds: &[TargetKind]) -> Self {
        match kinds {
            [TargetKind::Lib] => Self::Library,
            [TargetKind::CustomBuild] => Self::BuildScript,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Target {
    pub role: TargetRole,
    /// The target's main file: relative to the workspace root when it is inside it.
    pub src_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct CrateInfo {
    pub name: String,
    /// Relative to the workspace root.
    pub dir: PathBuf,
    pub deps: Vec<Dep>,
    pub targets: Vec<Target>,
    /// The crate's own `[features]` table: each feature and what it turns on, as written.
    pub features: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// The package at fault, or the folder for a rule about a folder.
    pub krate: String,
    /// The dependency at fault; for a rule about the crate itself, what in it is at fault.
    pub dependency: String,
    pub rule: &'static str,
}

fn violation(krate: &CrateInfo, dependency: &str, rule: &'static str) -> Violation {
    Violation {
        krate: krate.name.clone(),
        dependency: dependency.to_owned(),
        rule,
    }
}

/// Every rule of `check-arch` over the workspace at `root`.
pub fn check_workspace(root: &Path) -> Result<Vec<Violation>, XtaskError> {
    let crates = load_workspace(root)?;
    let mut violations = check_arch(&crates);
    violations.extend(hack::check_hack(root, &crates)?);
    violations.extend(manifests::check_members(root, &crates)?);
    violations.extend(manifests::check_patches(root)?);
    Ok(violations)
}

pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_target_is_a_library_only_when_its_one_kind_is_lib() {
        assert_eq!(TargetRole::of(&[TargetKind::Lib]), TargetRole::Library);
        assert_eq!(
            TargetRole::of(&[TargetKind::CustomBuild]),
            TargetRole::BuildScript
        );
        for kinds in [
            &[TargetKind::Bin][..],
            &[TargetKind::ProcMacro],
            &[TargetKind::RLib, TargetKind::CDyLib],
            &[TargetKind::Lib, TargetKind::CDyLib],
            &[],
        ] {
            assert_eq!(TargetRole::of(kinds), TargetRole::Other, "{kinds:?}");
        }
    }
}
