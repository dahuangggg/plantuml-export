use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

use globset::{Glob, GlobSet, GlobSetBuilder};
use ignore::WalkBuilder;

pub const SUPPORTED_SOURCE_SUFFIXES: [&str; 5] = ["puml", "plantuml", "pu", "iuml", "wsd"];

#[derive(Clone, Debug)]
pub struct DiscoveryOptions {
    pub root: PathBuf,
    pub input_base: PathBuf,
    pub inputs: Vec<PathBuf>,
    pub workspace: bool,
    pub out_dir: PathBuf,
    pub cache_dir: Option<PathBuf>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub require_input: bool,
}

impl DiscoveryOptions {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            input_base: root.clone(),
            root,
            inputs: Vec::new(),
            workspace: false,
            out_dir: PathBuf::from("out"),
            cache_dir: None,
            include: Vec::new(),
            exclude: Vec::new(),
            require_input: false,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveredInput {
    pub absolute_path: PathBuf,
    pub relative_path: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscoveryErrorKind {
    InvalidRoot,
    InvalidInput,
    UnsupportedInput,
    UnsafePath,
    InvalidPattern,
    Walk,
    NoInputs,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveryError {
    pub kind: DiscoveryErrorKind,
    pub message: String,
}

impl DiscoveryError {
    fn new(kind: DiscoveryErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DiscoveryError {}

pub fn discover_inputs(options: &DiscoveryOptions) -> Result<Vec<DiscoveredInput>, DiscoveryError> {
    let root = canonical_directory(
        &options.root,
        "worktree root",
        DiscoveryErrorKind::InvalidRoot,
    )?;
    let input_base = canonical_directory(
        &options.input_base,
        "input base",
        DiscoveryErrorKind::InvalidInput,
    )?;
    ensure_within(&root, &input_base, "input base")?;

    let out_dir = resolve_project_path(&root, &options.out_dir, "output directory")?;
    let cache_dir = options
        .cache_dir
        .as_deref()
        .map(|path| resolve_project_path(&root, path, "cache directory"))
        .transpose()?;
    let include = build_globset(&options.include, "include")?;
    let exclude = build_globset(&options.exclude, "exclude")?;

    // The normalized relative path is both the deterministic sort key and the
    // de-duplication key when an explicit input is also found by --workspace.
    let mut found = BTreeMap::<String, DiscoveredInput>::new();

    for input in &options.inputs {
        let requested = if input.is_absolute() {
            input.clone()
        } else {
            input_base.join(input)
        };
        let absolute = canonical_file(&requested, "input", DiscoveryErrorKind::InvalidInput)?;
        ensure_within(&root, &absolute, "input")?;
        ensure_not_pruned(&root, &absolute, &out_dir, cache_dir.as_deref())?;
        if !is_supported_source(&absolute) {
            return Err(DiscoveryError::new(
                DiscoveryErrorKind::UnsupportedInput,
                format!(
                    "{} is not a supported standalone PlantUML source (expected {})",
                    requested.display(),
                    SUPPORTED_SOURCE_SUFFIXES
                        .iter()
                        .map(|suffix| format!(".{suffix}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ));
        }
        insert_input(&root, absolute, &mut found)?;
    }

    // If output/cache is the project root, the whole workspace is protected.
    // Do not special-case the walk root and accidentally discover generated
    // files from inside that protected directory.
    let workspace_root_is_pruned = out_dir == root || cache_dir.as_deref() == Some(root.as_path());
    if options.workspace && !workspace_root_is_pruned {
        let walk_root = root.clone();
        let walk_out = out_dir.clone();
        let walk_cache = cache_dir.clone();
        let mut builder = WalkBuilder::new(&root);
        builder
            .standard_filters(true)
            .hidden(false)
            .ignore(true)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            // The CLI deliberately supports a plain current directory when no
            // Git root exists. In that mode a colocated .gitignore is still a
            // user-authored discovery boundary and must remain effective.
            .require_git(false)
            .follow_links(false)
            .filter_entry(move |entry| {
                let path = entry.path();
                if path == walk_root {
                    return true;
                }
                if entry.file_name() == ".git" {
                    return false;
                }
                if path == walk_out || path.starts_with(&walk_out) {
                    return false;
                }
                if walk_cache
                    .as_ref()
                    .is_some_and(|cache| path == cache || path.starts_with(cache))
                {
                    return false;
                }
                true
            });

        for entry in builder.build() {
            let entry = entry.map_err(|error| {
                DiscoveryError::new(
                    DiscoveryErrorKind::Walk,
                    format!("failed while discovering PlantUML inputs: {error}"),
                )
            })?;
            let file_type = match entry.file_type() {
                Some(file_type) => file_type,
                None => continue,
            };
            if !file_type.is_file() || !is_supported_source(entry.path()) {
                continue;
            }

            let relative = entry.path().strip_prefix(&root).map_err(|_| {
                DiscoveryError::new(
                    DiscoveryErrorKind::UnsafePath,
                    format!("discovered input escaped root: {}", entry.path().display()),
                )
            })?;
            let normalized = normalize_relative_path(relative)?;
            if !matches_globs(&normalized, &include, &exclude) {
                continue;
            }

            let absolute = fs::canonicalize(entry.path()).map_err(|error| {
                DiscoveryError::new(
                    DiscoveryErrorKind::InvalidInput,
                    format!(
                        "failed to resolve discovered input {}: {error}",
                        entry.path().display()
                    ),
                )
            })?;
            ensure_within(&root, &absolute, "discovered input")?;
            insert_input(&root, absolute, &mut found)?;
        }
    }

    let inputs = found.into_values().collect::<Vec<_>>();
    if inputs.is_empty() && options.require_input {
        return Err(DiscoveryError::new(
            DiscoveryErrorKind::NoInputs,
            "no supported PlantUML input files were found",
        ));
    }
    Ok(inputs)
}

/// Resolve an existing or future project-local path without allowing lexical
/// traversal or an already-existing symlink ancestor to escape `root`.
pub fn resolve_project_path(
    root: &Path,
    path: &Path,
    label: &str,
) -> Result<PathBuf, DiscoveryError> {
    let root = canonical_directory(root, "worktree root", DiscoveryErrorKind::InvalidRoot)?;
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let normalized = normalize_absolute_path(&candidate).ok_or_else(|| {
        DiscoveryError::new(
            DiscoveryErrorKind::UnsafePath,
            format!(
                "{label} is not a valid absolute project path: {}",
                path.display()
            ),
        )
    })?;
    ensure_within(&root, &normalized, label)?;

    let mut existing = normalized.clone();
    let mut missing = Vec::new();
    loop {
        match fs::symlink_metadata(&existing) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = existing.file_name().ok_or_else(|| {
                    DiscoveryError::new(
                        DiscoveryErrorKind::UnsafePath,
                        format!("could not resolve {label}: {}", path.display()),
                    )
                })?;
                missing.push(name.to_os_string());
                if !existing.pop() {
                    return Err(DiscoveryError::new(
                        DiscoveryErrorKind::UnsafePath,
                        format!("could not resolve {label}: {}", path.display()),
                    ));
                }
            }
            Err(error) => {
                return Err(DiscoveryError::new(
                    DiscoveryErrorKind::UnsafePath,
                    format!("failed to inspect {label} {}: {error}", path.display()),
                ));
            }
        }
    }
    let mut resolved = fs::canonicalize(&existing).map_err(|error| {
        DiscoveryError::new(
            DiscoveryErrorKind::UnsafePath,
            format!("failed to resolve {label} {}: {error}", path.display()),
        )
    })?;
    ensure_within(&root, &resolved, label)?;
    for component in missing.into_iter().rev() {
        resolved.push(component);
    }
    ensure_within(&root, &resolved, label)?;
    Ok(resolved)
}

pub fn normalize_relative_path(path: &Path) -> Result<String, DiscoveryError> {
    if path.is_absolute() {
        return Err(DiscoveryError::new(
            DiscoveryErrorKind::UnsafePath,
            format!("expected a relative project path, got {}", path.display()),
        ));
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(DiscoveryError::new(
                    DiscoveryErrorKind::UnsafePath,
                    format!("project path escapes its root: {}", path.display()),
                ));
            }
        }
    }
    Ok(parts.join("/"))
}

pub fn is_supported_source(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            SUPPORTED_SOURCE_SUFFIXES
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
        .unwrap_or(false)
}

fn canonical_directory(
    path: &Path,
    label: &str,
    kind: DiscoveryErrorKind,
) -> Result<PathBuf, DiscoveryError> {
    let canonical = fs::canonicalize(path).map_err(|error| {
        DiscoveryError::new(
            kind,
            format!("failed to resolve {label} {}: {error}", path.display()),
        )
    })?;
    if !canonical.is_dir() {
        return Err(DiscoveryError::new(
            kind,
            format!("{label} is not a directory: {}", path.display()),
        ));
    }
    Ok(canonical)
}

fn canonical_file(
    path: &Path,
    label: &str,
    kind: DiscoveryErrorKind,
) -> Result<PathBuf, DiscoveryError> {
    let canonical = fs::canonicalize(path).map_err(|error| {
        DiscoveryError::new(
            kind,
            format!("failed to resolve {label} {}: {error}", path.display()),
        )
    })?;
    if !canonical.is_file() {
        return Err(DiscoveryError::new(
            kind,
            format!("{label} is not a file: {}", path.display()),
        ));
    }
    Ok(canonical)
}

fn ensure_within(root: &Path, path: &Path, label: &str) -> Result<(), DiscoveryError> {
    if path == root || path.starts_with(root) {
        return Ok(());
    }
    Err(DiscoveryError::new(
        DiscoveryErrorKind::UnsafePath,
        format!("{label} escapes worktree root: {}", path.display()),
    ))
}

fn ensure_not_pruned(
    root: &Path,
    input: &Path,
    out_dir: &Path,
    cache_dir: Option<&Path>,
) -> Result<(), DiscoveryError> {
    let in_output = input == out_dir || input.starts_with(out_dir);
    let in_cache = cache_dir
        .map(|cache| input == cache || input.starts_with(cache))
        .unwrap_or(false);
    let in_git = input
        .strip_prefix(root)
        .unwrap_or(input)
        .components()
        .any(|component| component.as_os_str() == ".git");
    if in_output || in_cache || in_git {
        return Err(DiscoveryError::new(
            DiscoveryErrorKind::InvalidInput,
            format!(
                "input is inside a directory excluded from discovery: {}",
                input.display()
            ),
        ));
    }
    Ok(())
}

fn insert_input(
    root: &Path,
    absolute: PathBuf,
    found: &mut BTreeMap<String, DiscoveredInput>,
) -> Result<(), DiscoveryError> {
    let relative = absolute
        .strip_prefix(root)
        .map_err(|_| {
            DiscoveryError::new(
                DiscoveryErrorKind::UnsafePath,
                format!("input escaped worktree root: {}", absolute.display()),
            )
        })?
        .to_path_buf();
    let normalized = normalize_relative_path(&relative)?;
    found.entry(normalized).or_insert_with(|| DiscoveredInput {
        absolute_path: absolute,
        relative_path: relative,
    });
    Ok(())
}

fn build_globset(patterns: &[String], label: &str) -> Result<Option<GlobSet>, DiscoveryError> {
    if patterns.is_empty() {
        return Ok(None);
    }
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = Glob::new(pattern).map_err(|error| {
            DiscoveryError::new(
                DiscoveryErrorKind::InvalidPattern,
                format!("invalid {label} glob `{pattern}`: {error}"),
            )
        })?;
        builder.add(glob);
    }
    builder.build().map(Some).map_err(|error| {
        DiscoveryError::new(
            DiscoveryErrorKind::InvalidPattern,
            format!("failed to compile {label} globs: {error}"),
        )
    })
}

fn matches_globs(path: &str, include: &Option<GlobSet>, exclude: &Option<GlobSet>) -> bool {
    let included = include
        .as_ref()
        .map(|patterns| patterns.is_match(path))
        .unwrap_or(true);
    let excluded = exclude
        .as_ref()
        .map(|patterns| patterns.is_match(path))
        .unwrap_or(false);
    included && !excluded
}

fn normalize_absolute_path(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            Component::Normal(part) => normalized.push(part),
        }
    }
    normalized.is_absolute().then_some(normalized)
}
