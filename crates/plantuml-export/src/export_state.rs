use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::discovery::resolve_project_path;
use crate::AppError;

const APPLICATION_DIRECTORY: &str = "plantuml-export";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExportStateLayout {
    pub directory: PathBuf,
    pub manifest: PathBuf,
    pub lock: PathBuf,
    pub transactions: PathBuf,
}

impl ExportStateLayout {
    pub(crate) fn new(directory: PathBuf) -> Self {
        Self {
            manifest: directory.join("manifest.json"),
            lock: directory.join("export.lock"),
            transactions: directory.join("transactions"),
            directory,
        }
    }
}

/// Return the durable, user-scoped directory for export ownership and
/// transaction state. Renderer assets intentionally use a separate cache.
pub fn default_state_root() -> Result<PathBuf, AppError> {
    #[cfg(target_os = "windows")]
    {
        return env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|base| base.join(APPLICATION_DIRECTORY).join("state"))
            .ok_or_else(|| {
                AppError::environment(
                    "state_directory_unavailable",
                    "LOCALAPPDATA is not set; cannot locate the PlantUML export state directory",
                )
            });
    }

    #[cfg(target_os = "macos")]
    {
        return env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| {
                home.join("Library/Application Support")
                    .join(APPLICATION_DIRECTORY)
            })
            .ok_or_else(|| {
                AppError::environment(
                    "state_directory_unavailable",
                    "HOME is not set; cannot locate the PlantUML export state directory",
                )
            });
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        return env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state"))
            })
            .map(|base| base.join(APPLICATION_DIRECTORY))
            .ok_or_else(|| {
                AppError::environment(
                    "state_directory_unavailable",
                    "neither XDG_STATE_HOME nor HOME is set; cannot locate the PlantUML export state directory",
                )
            });
    }

    #[allow(unreachable_code)]
    Err(AppError::environment(
        "state_directory_unavailable",
        "this platform has no PlantUML export state location",
    ))
}

/// Derive an isolated state directory without exposing the workspace path in
/// its name. The absolute workspace identity prevents unrelated repositories
/// with the same relative output directory from sharing ownership state.
pub fn workspace_state_dir(root: &Path, out_dir: &Path) -> Result<PathBuf, AppError> {
    let canonical_root = fs::canonicalize(root).map_err(|error| {
        AppError::usage(
            "invalid_root",
            format!(
                "failed to resolve worktree root {}: {error}",
                root.display()
            ),
        )
    })?;
    let output_identity = resolve_project_path(&canonical_root, out_dir, "output directory")
        .map_err(|error| AppError::usage("invalid_output_path", error.to_string()))?;

    let mut hasher = Sha256::new();
    hash_path_identity(&mut hasher, &canonical_root);
    hasher.update([0]);
    hash_path_identity(&mut hasher, &output_identity);
    let workspace_key = format!("{:x}", hasher.finalize());

    Ok(default_state_root()?.join("exports").join(workspace_key))
}

#[cfg(unix)]
fn hash_path_identity(hasher: &mut Sha256, path: &Path) {
    use std::os::unix::ffi::OsStrExt;

    hasher.update(path.as_os_str().as_bytes());
}

#[cfg(windows)]
fn hash_path_identity(hasher: &mut Sha256, path: &Path) {
    use std::os::windows::ffi::OsStrExt;

    for unit in path.as_os_str().encode_wide() {
        hasher.update(unit.to_le_bytes());
    }
}

#[cfg(not(any(unix, windows)))]
fn hash_path_identity(hasher: &mut Sha256, path: &Path) {
    hasher.update(path.to_string_lossy().as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn workspace_keys_separate_roots_and_output_directories() {
        let first = TempDir::new().expect("first workspace");
        let second = TempDir::new().expect("second workspace");

        let first_out =
            workspace_state_dir(first.path(), Path::new("out")).expect("first output state");
        let first_other =
            workspace_state_dir(first.path(), Path::new("generated")).expect("second output state");
        let second_out =
            workspace_state_dir(second.path(), Path::new("out")).expect("other workspace state");

        assert_ne!(first_out, first_other);
        assert_ne!(first_out, second_out);
        assert_eq!(
            first_out.parent().and_then(Path::file_name),
            Some(std::ffi::OsStr::new("exports"))
        );
    }

    #[test]
    fn workspace_keys_normalize_equivalent_output_paths() {
        let workspace = TempDir::new().expect("workspace");
        let root = workspace
            .path()
            .canonicalize()
            .expect("canonical workspace");
        let absolute_out = root.join("out");

        let relative = workspace_state_dir(&root, Path::new("out")).expect("relative output");
        let dotted =
            workspace_state_dir(&root, Path::new("./out")).expect("dotted relative output");
        let absolute = workspace_state_dir(&root, &absolute_out).expect("absolute output");

        assert_eq!(relative, dotted);
        assert_eq!(relative, absolute);
    }

    #[cfg(unix)]
    #[test]
    fn workspace_keys_resolve_existing_output_directory_aliases() {
        use std::os::unix::fs::symlink;

        let workspace = TempDir::new().expect("workspace");
        let root = workspace
            .path()
            .canonicalize()
            .expect("canonical workspace");
        let actual_out = root.join("actual-out");
        let alias_out = root.join("alias-out");
        fs::create_dir(&actual_out).expect("actual output directory");
        symlink(&actual_out, &alias_out).expect("output directory alias");

        let actual = workspace_state_dir(&root, &actual_out).expect("actual output");
        let alias = workspace_state_dir(&root, &alias_out).expect("aliased output");

        assert_eq!(actual, alias);
    }
}
