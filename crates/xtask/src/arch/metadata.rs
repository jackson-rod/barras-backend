//! The crates of a workspace as `cargo metadata` reports them.

use std::path::Path;

use cargo_metadata::{DependencyKind, MetadataCommand};

use super::{CrateInfo, Dep, DepKind, Origin, Target, TargetRole};
use crate::error::XtaskError;

pub fn load_workspace(root: &Path) -> Result<Vec<CrateInfo>, XtaskError> {
    let metadata = MetadataCommand::new()
        .current_dir(root)
        .manifest_path(root.join("Cargo.toml"))
        .no_deps()
        .exec()
        .map_err(|e| XtaskError::Metadata(e.to_string()))?;
    // Taken from the metadata, not from `root`: cargo may report a canonicalised path.
    let workspace_root = metadata.workspace_root.as_std_path();
    // Cargo spells a path like the root and normalises it, so a plain prefix strip is
    // enough. A path outside the root keeps its whole spelling and so equals no member's.
    let relative = |path: &Path| {
        path.strip_prefix(workspace_root)
            .unwrap_or(path)
            .to_path_buf()
    };
    let mut crates = Vec::new();
    for package in &metadata.packages {
        let manifest_dir = package
            .manifest_path
            .as_std_path()
            .parent()
            .ok_or_else(|| {
                XtaskError::Metadata(format!("manifest of {} has no directory", package.name))
            })?;
        let dir = manifest_dir
            .strip_prefix(workspace_root)
            .map_err(|e| XtaskError::Metadata(format!("{}: {e}", package.name)))?
            .to_path_buf();
        let deps = package
            .dependencies
            .iter()
            .map(|d| Dep {
                name: d.name.clone(),
                kind: match d.kind {
                    DependencyKind::Development => DepKind::Dev,
                    DependencyKind::Build => DepKind::Build,
                    _ => DepKind::Normal,
                },
                origin: Origin::of(d.source.as_ref()),
                path: d.path.as_ref().map(|path| relative(path.as_std_path())),
            })
            .collect();
        let targets = package
            .targets
            .iter()
            .map(|target| Target {
                role: TargetRole::of(&target.kind),
                src_path: relative(target.src_path.as_std_path()),
            })
            .collect();
        crates.push(CrateInfo {
            name: package.name.to_string(),
            dir,
            deps,
            targets,
        });
    }
    Ok(crates)
}
