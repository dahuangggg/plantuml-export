use std::cell::Cell;
#[cfg(test)]
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::cli::OutputFormat;
use crate::discovery::{
    normalize_relative_path, resolve_project_path, DiscoveredInput, DiscoveryError,
};
use crate::export_state::ExportStateLayout;

const MANIFEST_SCHEMA_VERSION: u32 = 2;
const EXPORT_LOCK_TIMEOUT: Duration = Duration::from_secs(30);
const EXPORT_LOCK_POLL_INTERVAL: Duration = Duration::from_millis(50);
const STAGING_DIRECTORY_PREFIX: &str = ".plantuml-export-staging-";
const TRANSACTION_JOURNAL_SCHEMA_VERSION: u32 = 2;
const TRANSACTION_JOURNAL_NAME: &str = "transaction-journal.json";
const TRANSACTION_RENDERING_MARKER_NAME: &str = "transaction-rendering";
const TRANSACTION_BACKUPS_COMPLETE_MARKER_NAME: &str = "transaction-backups-complete";
const TRANSACTION_COMMIT_MARKER_NAME: &str = "transaction-committed";
static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
thread_local! {
    static BEFORE_COMMIT_MUTATION: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
    static BEFORE_BACKUP_RENAME: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RendererMetadata {
    pub mode: String,
    pub version: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvironmentMetadata {
    pub java_version: Option<String>,
    pub graphviz_version: Option<String>,
    pub os: String,
    pub architecture: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RendererError {
    pub kind: RendererFailureKind,
    pub code: String,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RendererFailureKind {
    Operation,
    Environment,
}

impl RendererError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: RendererFailureKind::Operation,
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn environment(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: RendererFailureKind::Environment,
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for RendererError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for RendererError {}

pub trait Renderer {
    fn render(
        &self,
        input: &Path,
        staging_dir: &Path,
        format: OutputFormat,
    ) -> Result<(), RendererError>;

    fn validate(&self, output: &Path, format: OutputFormat) -> Result<(), RendererError>;

    fn metadata(&self) -> RendererMetadata;
}

#[derive(Clone, Debug)]
pub struct ExportRequest {
    pub root: PathBuf,
    pub out_dir: PathBuf,
    pub state_dir: PathBuf,
    pub inputs: Vec<DiscoveredInput>,
    pub format: OutputFormat,
    pub keep_going: bool,
    pub tool_version: String,
    pub environment: EnvironmentMetadata,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportErrorKind {
    Environment,
    InputFailure,
    OutputValidation,
    OwnershipConflict,
    UnsafePath,
    InvalidManifest,
    Io,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportError {
    pub kind: ExportErrorKind,
    pub message: String,
}

impl ExportError {
    fn new(kind: ExportErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for ExportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ExportError {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSuccess {
    pub input: String,
    pub outputs: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportFailure {
    pub input: String,
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportReport {
    pub succeeded: Vec<ExportSuccess>,
    pub failures: Vec<ExportFailure>,
}

impl ExportReport {
    pub fn is_partial(&self) -> bool {
        !self.succeeded.is_empty() && !self.failures.is_empty()
    }
}

enum PreparationFailure {
    Operation(ExportFailure),
    Environment(ExportError),
}

impl From<ExportFailure> for PreparationFailure {
    fn from(failure: ExportFailure) -> Self {
        Self::Operation(failure)
    }
}

pub struct ExportSession<R> {
    renderer: R,
}

impl<R> ExportSession<R>
where
    R: Renderer,
{
    pub fn new(renderer: R) -> Self {
        Self { renderer }
    }

    pub fn run(&self, request: ExportRequest) -> Result<ExportReport, ExportError> {
        let root = canonical_root(&request.root)?;
        let out_dir = project_path(&root, &request.out_dir, "output directory")?;
        if out_dir == root {
            return Err(ExportError::new(
                ExportErrorKind::UnsafePath,
                "the worktree root cannot be used as the export output directory",
            ));
        }
        let requested_state = ExportStateLayout::new(request.state_dir.clone());
        if request.inputs.is_empty() {
            return Ok(ExportReport::default());
        }

        create_output_directory(&root, &out_dir)?;
        let state = create_state_directory(&requested_state, &out_dir)?;
        ensure_transaction_filesystem(&out_dir, &state.directory)?;
        let _export_lock = ExportLock::acquire(&state.lock)?;
        recover_abandoned_transactions(&root, &out_dir, &state)?;
        let staging = StagingDirectory::create(&state.transactions)?;
        let previous = load_manifest(&root, &out_dir, &state.manifest)?;
        let previous_owners = manifest_owners(&previous)?;
        let mut claimed = BTreeMap::<String, String>::new();
        let mut prepared = Vec::<PreparedInput>::new();
        let mut failures = Vec::<ExportFailure>::new();

        for (index, input) in request.inputs.iter().enumerate() {
            let input_key = validate_input(&root, input)?;
            let input_staging = staging.path.join(format!("render-{index}"));
            fs::create_dir(&input_staging).map_err(|error| {
                io_error(
                    format!("create staging directory {}", input_staging.display()),
                    error,
                )
            })?;

            let prepared_input = self
                .render_one(
                    &root,
                    &out_dir,
                    input,
                    &input_key,
                    &input_staging,
                    request.format,
                )
                .and_then(|prepared| {
                    check_ownership(&root, &prepared, &previous_owners, &mut claimed)
                        .map_err(PreparationFailure::from)?;
                    Ok(prepared)
                });

            match prepared_input {
                Ok(item) => prepared.push(item),
                Err(PreparationFailure::Environment(error)) => return Err(error),
                Err(PreparationFailure::Operation(failure)) if request.keep_going => {
                    failures.push(failure)
                }
                Err(PreparationFailure::Operation(failure)) => {
                    return Err(ExportError::new(
                        failure_kind(&failure.code),
                        format!("{}: {}", failure.input, failure.message),
                    ));
                }
            }
        }

        if prepared.is_empty() {
            return Ok(ExportReport {
                succeeded: Vec::new(),
                failures,
            });
        }

        let renderer = self.renderer.metadata();
        let next = next_manifest(
            previous.as_ref(),
            &prepared,
            request.tool_version,
            renderer,
            request.environment,
        );
        commit_transaction(
            &root,
            &out_dir,
            &state.manifest,
            &staging,
            previous.as_ref(),
            &prepared,
            &next,
        )?;

        let succeeded = prepared
            .into_iter()
            .map(|item| ExportSuccess {
                input: item.input,
                outputs: item
                    .outputs
                    .into_iter()
                    .map(|output| output.relative)
                    .collect(),
            })
            .collect();
        Ok(ExportReport {
            succeeded,
            failures,
        })
    }

    fn render_one(
        &self,
        root: &Path,
        out_dir: &Path,
        input: &DiscoveredInput,
        input_key: &str,
        staging_dir: &Path,
        format: OutputFormat,
    ) -> Result<PreparedInput, PreparationFailure> {
        self.renderer
            .render(&input.absolute_path, staging_dir, format)
            .map_err(|error| match error.kind {
                RendererFailureKind::Operation => PreparationFailure::Operation(ExportFailure {
                    input: input_key.to_string(),
                    code: error.code,
                    message: error.message,
                }),
                RendererFailureKind::Environment => {
                    PreparationFailure::Environment(ExportError::new(
                        ExportErrorKind::Environment,
                        format!("{input_key}: {}", error.message),
                    ))
                }
            })?;

        let mut staged_files = Vec::new();
        let entries = fs::read_dir(staging_dir).map_err(|error| ExportFailure {
            input: input_key.to_string(),
            code: "output_validation".to_string(),
            message: format!("failed to inspect staged output: {error}"),
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| ExportFailure {
                input: input_key.to_string(),
                code: "output_validation".to_string(),
                message: format!("failed to inspect staged output: {error}"),
            })?;
            let file_type = entry.file_type().map_err(|error| ExportFailure {
                input: input_key.to_string(),
                code: "output_validation".to_string(),
                message: format!("failed to inspect staged output: {error}"),
            })?;
            let extension_matches = entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case(format.extension()));
            if !file_type.is_file() || file_type.is_symlink() || !extension_matches {
                return Err(ExportFailure {
                    input: input_key.to_string(),
                    code: "output_validation".to_string(),
                    message: format!(
                        "renderer produced an unexpected staged entry: {}",
                        entry.path().display()
                    ),
                }
                .into());
            }
            staged_files.push(entry.path());
        }
        staged_files.sort_by(|left, right| left.file_name().cmp(&right.file_name()));
        if staged_files.is_empty() {
            return Err(ExportFailure {
                input: input_key.to_string(),
                code: "output_validation".to_string(),
                message: "renderer did not produce an output file".to_string(),
            }
            .into());
        }

        let relative_parent = input
            .relative_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty());
        let mut outputs = Vec::with_capacity(staged_files.len());
        for staged in staged_files {
            let metadata = fs::metadata(&staged).map_err(|error| ExportFailure {
                input: input_key.to_string(),
                code: "output_validation".to_string(),
                message: format!("failed to inspect {}: {error}", staged.display()),
            })?;
            if metadata.len() == 0 {
                return Err(ExportFailure {
                    input: input_key.to_string(),
                    code: "output_validation".to_string(),
                    message: format!("renderer produced an empty output: {}", staged.display()),
                }
                .into());
            }
            self.renderer
                .validate(&staged, format)
                .map_err(|error| ExportFailure {
                    input: input_key.to_string(),
                    code: "output_validation".to_string(),
                    message: format!("{} ({})", error.message, error.code),
                })?;
            let sha256 = sha256_file(&staged).map_err(|error| ExportFailure {
                input: input_key.to_string(),
                code: "output_validation".to_string(),
                message: format!("failed to hash {}: {error}", staged.display()),
            })?;

            let file_name = staged.file_name().expect("staged entry has a file name");
            let mut target = out_dir.to_path_buf();
            if let Some(parent) = relative_parent {
                target.push(parent);
            }
            target.push(file_name);
            let target =
                project_path(root, &target, "export target").map_err(|error| ExportFailure {
                    input: input_key.to_string(),
                    code: "unsafe_path".to_string(),
                    message: error.message,
                })?;
            if !target.starts_with(out_dir) {
                return Err(ExportFailure {
                    input: input_key.to_string(),
                    code: "unsafe_path".to_string(),
                    message: format!(
                        "export target escaped output directory: {}",
                        target.display()
                    ),
                }
                .into());
            }
            let relative = normalize_relative_path(
                target
                    .strip_prefix(root)
                    .expect("validated target is in root"),
            )
            .map_err(|error| ExportFailure {
                input: input_key.to_string(),
                code: "unsafe_path".to_string(),
                message: error.to_string(),
            })?;
            outputs.push(PreparedOutput {
                staged,
                target,
                relative,
                sha256,
            });
        }
        Ok(PreparedInput {
            input: input_key.to_string(),
            format,
            outputs,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    inputs: BTreeMap<String, ManifestInput>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManifestInput {
    formats: BTreeMap<OutputFormat, ManifestFormat>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManifestFormat {
    outputs: Vec<ManifestOutput>,
    tool_version: String,
    renderer: RendererMetadata,
    environment: EnvironmentMetadata,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManifestOutput {
    path: String,
    sha256: String,
}

#[derive(Debug)]
struct PreparedInput {
    input: String,
    format: OutputFormat,
    outputs: Vec<PreparedOutput>,
}

#[derive(Debug)]
struct PreparedOutput {
    staged: PathBuf,
    target: PathBuf,
    relative: String,
    sha256: String,
}

#[derive(Clone, Debug)]
struct ManifestOwner {
    input: String,
    format: OutputFormat,
    sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TransactionJournal {
    schema_version: u32,
    entries: Vec<TransactionJournalEntry>,
    created_directories: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TransactionJournalEntry {
    target: TransactionTarget,
    had_existing_file: bool,
    install: bool,
    install_sha256: Option<String>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum TransactionTarget {
    Output { path: String },
    Manifest,
}

#[derive(Debug)]
struct ResolvedTransactionJournal {
    entries: Vec<ResolvedTransactionEntry>,
    created_directories: Vec<PathBuf>,
    backups_complete: bool,
}

#[derive(Debug)]
struct ResolvedTransactionEntry {
    path: PathBuf,
    backup: PathBuf,
    had_existing_file: bool,
    install: bool,
    install_sha256: Option<String>,
}

struct StagingDirectory {
    path: PathBuf,
    preserve_on_drop: Cell<bool>,
}

#[derive(Debug)]
struct ExportLock {
    _file: File,
}

impl ExportLock {
    fn acquire(lock_path: &Path) -> Result<Self, ExportError> {
        Self::acquire_with_policy(lock_path, EXPORT_LOCK_TIMEOUT, EXPORT_LOCK_POLL_INTERVAL)
    }

    fn acquire_with_policy(
        lock_path: &Path,
        timeout: Duration,
        poll_interval: Duration,
    ) -> Result<Self, ExportError> {
        let file = open_export_lock(lock_path)?;
        let started = Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Self { _file: file }),
                Err(TryLockError::WouldBlock) if started.elapsed() < timeout => {
                    if poll_interval.is_zero() {
                        thread::yield_now();
                    } else {
                        thread::sleep(poll_interval);
                    }
                }
                Err(TryLockError::WouldBlock) => {
                    return Err(ExportError::new(
                        ExportErrorKind::Environment,
                        format!(
                            "timed out after {} ms waiting for export lock {}",
                            timeout.as_millis(),
                            lock_path.display()
                        ),
                    ));
                }
                Err(TryLockError::Error(error)) => {
                    return Err(io_error(
                        format!("acquire export lock {}", lock_path.display()),
                        error,
                    ));
                }
            }
        }
    }
}

fn open_export_lock(path: &Path) -> Result<File, ExportError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {}
        Ok(_) => {
            return Err(ExportError::new(
                ExportErrorKind::OwnershipConflict,
                format!("export lock is not a regular file: {}", path.display()),
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(io_error(
                format!("inspect export lock {}", path.display()),
                error,
            ));
        }
    }

    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }

    let file = options
        .open(path)
        .map_err(|error| io_error(format!("open export lock {}", path.display()), error))?;
    let metadata = file.metadata().map_err(|error| {
        io_error(
            format!("inspect open export lock {}", path.display()),
            error,
        )
    })?;
    if !metadata.file_type().is_file() || metadata_is_reparse_point(&metadata) {
        return Err(ExportError::new(
            ExportErrorKind::OwnershipConflict,
            format!("export lock is not a regular file: {}", path.display()),
        ));
    }
    Ok(file)
}

#[cfg(not(windows))]
fn metadata_is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(windows)]
fn metadata_is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

impl StagingDirectory {
    fn create(out_dir: &Path) -> Result<Self, ExportError> {
        for _ in 0..100 {
            let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = out_dir.join(format!(
                "{STAGING_DIRECTORY_PREFIX}{}-{sequence}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => {
                    if let Err(error) =
                        write_synced(&path.join(TRANSACTION_RENDERING_MARKER_NAME), b"rendering")
                    {
                        let _ = fs::remove_dir_all(&path);
                        return Err(error);
                    }
                    return Ok(Self {
                        path,
                        preserve_on_drop: Cell::new(false),
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(io_error(
                        format!("create transaction staging directory {}", path.display()),
                        error,
                    ));
                }
            }
        }
        Err(ExportError::new(
            ExportErrorKind::Io,
            "could not allocate a unique export staging directory",
        ))
    }

    fn preserve(&self) {
        self.preserve_on_drop.set(true);
    }
}

impl Drop for StagingDirectory {
    fn drop(&mut self) {
        if self.preserve_on_drop.get() {
            // An incomplete rollback remains recoverable on the next run. Do
            // not destroy its backups while unwinding the current process.
            return;
        }
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn canonical_root(root: &Path) -> Result<PathBuf, ExportError> {
    let canonical = fs::canonicalize(root)
        .map_err(|error| io_error(format!("resolve worktree root {}", root.display()), error))?;
    if !canonical.is_dir() {
        return Err(ExportError::new(
            ExportErrorKind::UnsafePath,
            format!("worktree root is not a directory: {}", root.display()),
        ));
    }
    Ok(canonical)
}

fn project_path(root: &Path, path: &Path, label: &str) -> Result<PathBuf, ExportError> {
    resolve_project_path(root, path, label).map_err(map_discovery_error)
}

fn map_discovery_error(error: DiscoveryError) -> ExportError {
    ExportError::new(ExportErrorKind::UnsafePath, error.to_string())
}

fn create_output_directory(root: &Path, out_dir: &Path) -> Result<(), ExportError> {
    fs::create_dir_all(out_dir).map_err(|error| {
        io_error(
            format!("create output directory {}", out_dir.display()),
            error,
        )
    })?;
    let canonical = fs::canonicalize(out_dir).map_err(|error| {
        io_error(
            format!("resolve output directory {}", out_dir.display()),
            error,
        )
    })?;
    if canonical != out_dir || !canonical.starts_with(root) {
        return Err(ExportError::new(
            ExportErrorKind::UnsafePath,
            format!(
                "output directory escaped the worktree root: {}",
                out_dir.display()
            ),
        ));
    }
    Ok(())
}

fn create_state_directory(
    requested: &ExportStateLayout,
    out_dir: &Path,
) -> Result<ExportStateLayout, ExportError> {
    if !requested.directory.is_absolute()
        || requested
            .directory
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(ExportError::new(
            ExportErrorKind::UnsafePath,
            format!(
                "export state must use an absolute normalized directory: {}",
                requested.directory.display()
            ),
        ));
    }
    let state = ExportStateLayout::new(resolve_future_path(&requested.directory)?);
    if state.directory.starts_with(out_dir) {
        return Err(ExportError::new(
            ExportErrorKind::UnsafePath,
            format!(
                "export state must be outside the output tree: {}",
                requested.directory.display()
            ),
        ));
    }
    fs::create_dir_all(&state.transactions).map_err(|error| {
        io_error(
            format!(
                "create export state directory {}",
                state.transactions.display()
            ),
            error,
        )
    })?;
    for (label, path) in [
        ("export state directory", &state.directory),
        ("export transaction directory", &state.transactions),
    ] {
        let metadata = fs::symlink_metadata(path)
            .map_err(|error| io_error(format!("inspect {label} {}", path.display()), error))?;
        if !metadata.file_type().is_dir()
            || metadata.file_type().is_symlink()
            || metadata_is_reparse_point(&metadata)
        {
            return Err(ExportError::new(
                ExportErrorKind::OwnershipConflict,
                format!("{label} is not a real directory: {}", path.display()),
            ));
        }
        let canonical = fs::canonicalize(path)
            .map_err(|error| io_error(format!("resolve {label} {}", path.display()), error))?;
        if canonical != *path {
            return Err(ExportError::new(
                ExportErrorKind::UnsafePath,
                format!("{label} contains a symlink or alias: {}", path.display()),
            ));
        }
    }
    Ok(state)
}

fn resolve_future_path(path: &Path) -> Result<PathBuf, ExportError> {
    let mut cursor = path;
    let mut missing = Vec::new();
    loop {
        match fs::canonicalize(cursor) {
            Ok(mut canonical) => {
                for component in missing.iter().rev() {
                    canonical.push(component);
                }
                return Ok(canonical);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = cursor.file_name().ok_or_else(|| {
                    ExportError::new(
                        ExportErrorKind::UnsafePath,
                        format!("could not resolve export state path: {}", path.display()),
                    )
                })?;
                missing.push(name.to_os_string());
                cursor = cursor.parent().ok_or_else(|| {
                    ExportError::new(
                        ExportErrorKind::UnsafePath,
                        format!("could not resolve export state path: {}", path.display()),
                    )
                })?;
            }
            Err(error) => {
                return Err(io_error(
                    format!("resolve export state path {}", path.display()),
                    error,
                ));
            }
        }
    }
}

#[cfg(unix)]
fn ensure_transaction_filesystem(out_dir: &Path, state_dir: &Path) -> Result<(), ExportError> {
    use std::os::unix::fs::MetadataExt;

    let out_device = fs::metadata(out_dir)
        .map_err(|error| {
            io_error(
                format!("inspect output volume {}", out_dir.display()),
                error,
            )
        })?
        .dev();
    let state_device = fs::metadata(state_dir)
        .map_err(|error| {
            io_error(
                format!("inspect state volume {}", state_dir.display()),
                error,
            )
        })?
        .dev();
    if out_device != state_device {
        return Err(ExportError::new(
            ExportErrorKind::Environment,
            format!(
                "the output directory and PlantUML export state are on different filesystems; atomic export requires one volume (output: {}, state: {})",
                out_dir.display(),
                state_dir.display()
            ),
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn windows_volume_guid(path: &Path) -> Result<String, ExportError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        GetVolumeNameForVolumeMountPointW, GetVolumePathNameW,
    };

    let path_wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mount_capacity = path_wide.len().checked_add(1).ok_or_else(|| {
        ExportError::new(
            ExportErrorKind::Environment,
            format!(
                "Windows path is too long to inspect its volume: {}",
                path.display()
            ),
        )
    })?;
    let mount_capacity_u32 = u32::try_from(mount_capacity).map_err(|_| {
        ExportError::new(
            ExportErrorKind::Environment,
            format!(
                "Windows path is too long to inspect its volume: {}",
                path.display()
            ),
        )
    })?;
    let mut mount_point = vec![0_u16; mount_capacity];

    if unsafe {
        GetVolumePathNameW(
            path_wide.as_ptr(),
            mount_point.as_mut_ptr(),
            mount_capacity_u32,
        )
    } == 0
    {
        let error = std::io::Error::last_os_error();
        return Err(io_error(
            format!("resolve Windows volume mount point for {}", path.display()),
            error,
        ));
    }

    let mount_len = mount_point
        .iter()
        .position(|unit| *unit == 0)
        .ok_or_else(|| {
            ExportError::new(
                ExportErrorKind::Environment,
                format!(
                    "Windows returned a non-terminated volume mount point for {}",
                    path.display()
                ),
            )
        })?;
    if mount_len == 0 || mount_point[mount_len - 1] != b'\\' as u16 {
        return Err(ExportError::new(
            ExportErrorKind::Environment,
            format!(
                "Windows returned an invalid volume mount point for {}",
                path.display()
            ),
        ));
    }

    let mut volume_name = [0_u16; 50];
    if unsafe {
        GetVolumeNameForVolumeMountPointW(
            mount_point.as_ptr(),
            volume_name.as_mut_ptr(),
            volume_name.len() as u32,
        )
    } == 0
    {
        let error = std::io::Error::last_os_error();
        return Err(io_error(
            format!("resolve Windows volume identity for {}", path.display()),
            error,
        ));
    }

    let volume_len = volume_name
        .iter()
        .position(|unit| *unit == 0)
        .ok_or_else(|| {
            ExportError::new(
                ExportErrorKind::Environment,
                format!(
                    "Windows returned a non-terminated volume identity for {}",
                    path.display()
                ),
            )
        })?;
    String::from_utf16(&volume_name[..volume_len]).map_err(|error| {
        ExportError::new(
            ExportErrorKind::Environment,
            format!(
                "Windows returned an invalid volume identity for {}: {error}",
                path.display()
            ),
        )
    })
}

#[cfg(windows)]
fn ensure_transaction_filesystem(out_dir: &Path, state_dir: &Path) -> Result<(), ExportError> {
    let out_volume = windows_volume_guid(out_dir)?;
    let state_volume = windows_volume_guid(state_dir)?;
    if !out_volume.eq_ignore_ascii_case(&state_volume) {
        return Err(ExportError::new(
            ExportErrorKind::Environment,
            format!(
                "the output directory and PlantUML export state are on different volumes; atomic export requires one volume (output: {}, state: {})",
                out_dir.display(),
                state_dir.display()
            ),
        ));
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn ensure_transaction_filesystem(_out_dir: &Path, _state_dir: &Path) -> Result<(), ExportError> {
    Ok(())
}

fn validate_input(root: &Path, input: &DiscoveredInput) -> Result<String, ExportError> {
    let input_key = normalize_relative_path(&input.relative_path).map_err(map_discovery_error)?;
    if input_key.is_empty() {
        return Err(ExportError::new(
            ExportErrorKind::UnsafePath,
            "an export input cannot be the worktree root",
        ));
    }
    let canonical = fs::canonicalize(&input.absolute_path).map_err(|error| {
        io_error(
            format!("resolve export input {}", input.absolute_path.display()),
            error,
        )
    })?;
    if !canonical.is_file() || !canonical.starts_with(root) {
        return Err(ExportError::new(
            ExportErrorKind::UnsafePath,
            format!(
                "export input escaped the worktree root: {}",
                input.absolute_path.display()
            ),
        ));
    }
    let expected = root.join(&input.relative_path);
    if canonical != expected {
        return Err(ExportError::new(
            ExportErrorKind::UnsafePath,
            format!(
                "export input path does not match its project-relative identity: {}",
                input.absolute_path.display()
            ),
        ));
    }
    Ok(input_key)
}

fn load_manifest(
    root: &Path,
    out_dir: &Path,
    manifest_path: &Path,
) -> Result<Option<Manifest>, ExportError> {
    let metadata = match fs::symlink_metadata(manifest_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(io_error(
                format!("inspect export manifest {}", manifest_path.display()),
                error,
            ));
        }
    };
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(ExportError::new(
            ExportErrorKind::InvalidManifest,
            format!(
                "export manifest is not a regular file: {}",
                manifest_path.display()
            ),
        ));
    }
    let bytes = fs::read(manifest_path).map_err(|error| {
        io_error(
            format!("read export manifest {}", manifest_path.display()),
            error,
        )
    })?;
    let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|error| {
        ExportError::new(
            ExportErrorKind::InvalidManifest,
            format!(
                "invalid export manifest {}: {error}",
                manifest_path.display()
            ),
        )
    })?;
    if manifest.schema_version != MANIFEST_SCHEMA_VERSION {
        return Err(ExportError::new(
            ExportErrorKind::InvalidManifest,
            format!(
                "unsupported export manifest schemaVersion {}; expected {}",
                manifest.schema_version, MANIFEST_SCHEMA_VERSION
            ),
        ));
    }
    for (input, entry) in &manifest.inputs {
        let normalized_input = normalize_relative_path(Path::new(input)).map_err(|error| {
            ExportError::new(ExportErrorKind::InvalidManifest, error.to_string())
        })?;
        if normalized_input != *input || input.is_empty() {
            return Err(ExportError::new(
                ExportErrorKind::InvalidManifest,
                format!("manifest contains a non-normalized input path: {input}"),
            ));
        }
        if entry.formats.is_empty() {
            return Err(ExportError::new(
                ExportErrorKind::InvalidManifest,
                format!("manifest input {input} has no format entries"),
            ));
        }
        for (format, format_entry) in &entry.formats {
            validate_manifest_provenance(input, *format, format_entry)?;
            if format_entry.outputs.is_empty() {
                return Err(ExportError::new(
                    ExportErrorKind::InvalidManifest,
                    format!(
                        "manifest input {input}/{} has no outputs",
                        format.extension()
                    ),
                ));
            }
            for output in &format_entry.outputs {
                let normalized =
                    normalize_relative_path(Path::new(&output.path)).map_err(|error| {
                        ExportError::new(ExportErrorKind::InvalidManifest, error.to_string())
                    })?;
                if normalized != output.path || output.path.is_empty() {
                    return Err(ExportError::new(
                        ExportErrorKind::InvalidManifest,
                        format!(
                            "manifest contains a non-normalized output path: {}",
                            output.path
                        ),
                    ));
                }
                if !is_lower_sha256(&output.sha256) {
                    return Err(ExportError::new(
                        ExportErrorKind::InvalidManifest,
                        format!("manifest output {} has an invalid SHA-256", output.path),
                    ));
                }
                let path = project_path(root, Path::new(&output.path), "manifest-owned output")?;
                if !path.starts_with(out_dir) || path == out_dir || path == manifest_path {
                    return Err(ExportError::new(
                        ExportErrorKind::InvalidManifest,
                        format!("manifest claims an unsafe output path: {}", output.path),
                    ));
                }
                let matches_format = Path::new(&output.path)
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case(format.extension()));
                if !matches_format {
                    return Err(ExportError::new(
                        ExportErrorKind::InvalidManifest,
                        format!(
                            "manifest output {} does not match its {} format entry",
                            output.path,
                            format.extension()
                        ),
                    ));
                }
            }
        }
    }
    Ok(Some(manifest))
}

fn validate_manifest_provenance(
    input: &str,
    format: OutputFormat,
    entry: &ManifestFormat,
) -> Result<(), ExportError> {
    let owner = format!("{input}/{}", format.extension());
    for (label, value) in [
        ("toolVersion", entry.tool_version.as_str()),
        ("renderer.mode", entry.renderer.mode.as_str()),
        ("renderer.version", entry.renderer.version.as_str()),
        ("environment.os", entry.environment.os.as_str()),
        (
            "environment.architecture",
            entry.environment.architecture.as_str(),
        ),
    ] {
        if value.trim().is_empty() {
            return Err(ExportError::new(
                ExportErrorKind::InvalidManifest,
                format!("manifest input {owner} has an empty {label}"),
            ));
        }
    }
    for (label, value) in [
        (
            "environment.javaVersion",
            entry.environment.java_version.as_deref(),
        ),
        (
            "environment.graphvizVersion",
            entry.environment.graphviz_version.as_deref(),
        ),
    ] {
        if value.is_some_and(|value| value.trim().is_empty()) {
            return Err(ExportError::new(
                ExportErrorKind::InvalidManifest,
                format!("manifest input {owner} has an empty {label}"),
            ));
        }
    }
    Ok(())
}

fn manifest_owners(
    manifest: &Option<Manifest>,
) -> Result<BTreeMap<String, ManifestOwner>, ExportError> {
    let mut owners = BTreeMap::new();
    let Some(manifest) = manifest else {
        return Ok(owners);
    };
    for (input, entry) in &manifest.inputs {
        for (format, format_entry) in &entry.formats {
            for output in &format_entry.outputs {
                if let Some(previous) = owners.insert(
                    output.path.clone(),
                    ManifestOwner {
                        input: input.clone(),
                        format: *format,
                        sha256: output.sha256.clone(),
                    },
                ) {
                    return Err(ExportError::new(
                        ExportErrorKind::InvalidManifest,
                        format!(
                            "manifest assigns output {} to both {}/{} and {input}/{}",
                            output.path,
                            previous.input,
                            previous.format.extension(),
                            format.extension()
                        ),
                    ));
                }
            }
        }
    }
    Ok(owners)
}

fn check_ownership(
    root: &Path,
    prepared: &PreparedInput,
    previous_owners: &BTreeMap<String, ManifestOwner>,
    claimed: &mut BTreeMap<String, String>,
) -> Result<(), ExportFailure> {
    for output in &prepared.outputs {
        if let Some(owner) = claimed.get(&output.relative) {
            return Err(ownership_failure(
                &prepared.input,
                format!("output {} is also produced by {owner}", output.relative),
            ));
        }
        if let Some(owner) = previous_owners.get(&output.relative) {
            if owner.input != prepared.input || owner.format != prepared.format {
                return Err(ownership_failure(
                    &prepared.input,
                    format!(
                        "output {} is owned by another source-format: {}/{}",
                        output.relative,
                        owner.input,
                        owner.format.extension()
                    ),
                ));
            }
            verify_owned_output(
                &prepared.input,
                &output.relative,
                &output.target,
                &owner.sha256,
            )?;
        } else {
            match fs::symlink_metadata(&output.target) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Ok(_) => {
                    return Err(ownership_failure(
                        &prepared.input,
                        format!("refusing to overwrite unmanaged output {}", output.relative),
                    ));
                }
                Err(error) => {
                    return Err(ownership_failure(
                        &prepared.input,
                        format!(
                            "failed to inspect intended output {}: {error}",
                            output.relative
                        ),
                    ));
                }
            }
        }
        let resolved =
            project_path(root, &output.target, "export target").map_err(|error| ExportFailure {
                input: prepared.input.clone(),
                code: "unsafe_path".to_string(),
                message: error.message,
            })?;
        if resolved != output.target {
            return Err(ExportFailure {
                input: prepared.input.clone(),
                code: "unsafe_path".to_string(),
                message: format!(
                    "export target changed during validation: {}",
                    output.relative
                ),
            });
        }
    }
    for output in &prepared.outputs {
        claimed.insert(output.relative.clone(), prepared.input.clone());
    }
    Ok(())
}

fn verify_owned_output(
    input: &str,
    relative: &str,
    path: &Path,
    expected_sha256: &str,
) -> Result<(), ExportFailure> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(ownership_failure(
                input,
                format!("failed to inspect managed output {relative}: {error}"),
            ));
        }
    };
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(ownership_failure(
            input,
            format!("managed output is no longer a regular file: {relative}"),
        ));
    }
    let actual = sha256_file(path).map_err(|error| {
        ownership_failure(
            input,
            format!("failed to hash managed output {relative}: {error}"),
        )
    })?;
    if actual != expected_sha256 {
        return Err(ownership_failure(
            input,
            format!(
                "managed output changed outside PlantUML Export; refusing to overwrite {relative}"
            ),
        ));
    }
    Ok(())
}

fn ownership_failure(input: &str, message: String) -> ExportFailure {
    ExportFailure {
        input: input.to_string(),
        code: "ownership_conflict".to_string(),
        message,
    }
}

fn failure_kind(code: &str) -> ExportErrorKind {
    match code {
        "output_validation" => ExportErrorKind::OutputValidation,
        "ownership_conflict" => ExportErrorKind::OwnershipConflict,
        "unsafe_path" => ExportErrorKind::UnsafePath,
        _ => ExportErrorKind::InputFailure,
    }
}

fn next_manifest(
    previous: Option<&Manifest>,
    prepared: &[PreparedInput],
    tool_version: String,
    renderer: RendererMetadata,
    environment: EnvironmentMetadata,
) -> Manifest {
    let mut inputs = previous
        .map(|manifest| manifest.inputs.clone())
        .unwrap_or_default();
    for item in prepared {
        inputs
            .entry(item.input.clone())
            .or_insert_with(|| ManifestInput {
                formats: BTreeMap::new(),
            })
            .formats
            .insert(
                item.format,
                ManifestFormat {
                    outputs: item
                        .outputs
                        .iter()
                        .map(|output| ManifestOutput {
                            path: output.relative.clone(),
                            sha256: output.sha256.clone(),
                        })
                        .collect(),
                    tool_version: tool_version.clone(),
                    renderer: renderer.clone(),
                    environment: environment.clone(),
                },
            );
    }
    Manifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        inputs,
    }
}

fn commit_transaction(
    root: &Path,
    out_dir: &Path,
    manifest_path: &Path,
    staging: &StagingDirectory,
    previous: Option<&Manifest>,
    prepared: &[PreparedInput],
    next: &Manifest,
) -> Result<(), ExportError> {
    let manifest_bytes = serde_json::to_vec_pretty(next).map_err(|error| {
        ExportError::new(
            ExportErrorKind::InvalidManifest,
            format!("failed to serialize export manifest: {error}"),
        )
    })?;
    let manifest_temp = staging.path.join("next-manifest.json");
    write_synced(&manifest_temp, &manifest_bytes)?;

    let mut install = BTreeMap::<PathBuf, String>::new();
    let mut mutate = BTreeSet::<PathBuf>::new();
    let mut expected_existing = BTreeMap::<PathBuf, Option<String>>::new();
    for item in prepared {
        let previous_entry = previous
            .and_then(|manifest| manifest.inputs.get(&item.input))
            .and_then(|entry| entry.formats.get(&item.format));
        for output in &item.outputs {
            let hash = sha256_file(&output.staged).map_err(|error| {
                io_error(
                    format!("hash staged output {}", output.staged.display()),
                    error,
                )
            })?;
            if hash != output.sha256 {
                return Err(ExportError::new(
                    ExportErrorKind::OutputValidation,
                    format!(
                        "staged output changed after validation: {}",
                        output.staged.display()
                    ),
                ));
            }
            let previous_hash = previous_entry.and_then(|entry| {
                entry
                    .outputs
                    .iter()
                    .find(|previous| previous.path == output.relative)
                    .map(|previous| previous.sha256.clone())
            });
            expected_existing.insert(output.target.clone(), previous_hash);
            install.insert(output.target.clone(), output.sha256.clone());
            mutate.insert(output.target.clone());
        }
        if let Some(previous_entry) = previous_entry {
            let current = item
                .outputs
                .iter()
                .map(|output| output.relative.as_str())
                .collect::<BTreeSet<_>>();
            for stale in &previous_entry.outputs {
                if !current.contains(stale.path.as_str()) {
                    let path = project_path(root, Path::new(&stale.path), "stale owned output")?;
                    if !path.starts_with(out_dir) || path == manifest_path {
                        return Err(ExportError::new(
                            ExportErrorKind::InvalidManifest,
                            format!("manifest claims an unsafe stale output: {}", stale.path),
                        ));
                    }
                    expected_existing.insert(path.clone(), Some(stale.sha256.clone()));
                    mutate.insert(path);
                }
            }
        }
    }
    let manifest_hash = sha256_file(&manifest_temp).map_err(|error| {
        io_error(
            format!("hash staged manifest {}", manifest_temp.display()),
            error,
        )
    })?;
    if install.keys().any(|target| {
        target == manifest_path
            || target.starts_with(manifest_path)
            || manifest_path.starts_with(target)
    }) {
        return Err(ExportError::new(
            ExportErrorKind::OwnershipConflict,
            format!(
                "export manifest path collides with a rendered output: {}",
                manifest_path.display()
            ),
        ));
    }
    install.insert(manifest_path.to_path_buf(), manifest_hash);
    mutate.insert(manifest_path.to_path_buf());

    let entries = plan_transaction_entries(
        root,
        out_dir,
        manifest_path,
        &mutate,
        &install,
        &expected_existing,
    )?;
    let created_directories =
        plan_created_directories(root, out_dir, manifest_path, &install, &entries)?;
    let journal = TransactionJournal {
        schema_version: TRANSACTION_JOURNAL_SCHEMA_VERSION,
        entries,
        created_directories,
    };

    let backup_root = staging.path.join("backup");
    fs::create_dir(&backup_root).map_err(|error| {
        io_error(
            format!(
                "create transaction backup directory {}",
                backup_root.display()
            ),
            error,
        )
    })?;
    let journal_bytes = serde_json::to_vec_pretty(&journal).map_err(|error| {
        ExportError::new(
            ExportErrorKind::InvalidManifest,
            format!("failed to serialize transaction journal: {error}"),
        )
    })?;
    let journal_path = staging.path.join(TRANSACTION_JOURNAL_NAME);
    write_synced(&journal_path, &journal_bytes)?;
    fs::remove_file(staging.path.join(TRANSACTION_RENDERING_MARKER_NAME)).map_err(|error| {
        io_error(
            format!(
                "finish render phase for transaction {}",
                staging.path.display()
            ),
            error,
        )
    })?;
    let mut resolved =
        resolve_transaction_journal(root, out_dir, manifest_path, &staging.path, journal)?;
    for entry in &mut resolved.entries {
        if entry.install {
            // The durable journal records the full crash-recovery plan. The
            // in-process view tracks which links were actually installed so
            // an immediate rollback never mistakes a raced user file for one
            // created by this transaction.
            entry.install = false;
        }
    }

    #[cfg(test)]
    BEFORE_COMMIT_MUTATION.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });

    let result = (|| -> Result<(), ExportError> {
        for entry in &resolved.entries {
            if !entry.had_existing_file {
                continue;
            }
            let metadata = fs::symlink_metadata(&entry.path).map_err(|error| {
                io_error(
                    format!("inspect owned path {}", entry.path.display()),
                    error,
                )
            })?;
            if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
                return Err(ExportError::new(
                    ExportErrorKind::OwnershipConflict,
                    format!(
                        "owned output is not a regular file: {}",
                        entry.path.display()
                    ),
                ));
            }
            if entry.path != manifest_path {
                let expected = expected_existing
                    .get(&entry.path)
                    .and_then(|hash| hash.as_deref())
                    .ok_or_else(|| {
                        ExportError::new(
                            ExportErrorKind::OwnershipConflict,
                            format!(
                                "refusing to replace output without verified ownership: {}",
                                entry.path.display()
                            ),
                        )
                    })?;
                let actual = sha256_file(&entry.path).map_err(|error| {
                    io_error(format!("hash owned output {}", entry.path.display()), error)
                })?;
                if actual != expected {
                    return Err(ExportError::new(
                        ExportErrorKind::OwnershipConflict,
                        format!(
                            "managed output changed before commit; refusing to replace {}",
                            entry.path.display()
                        ),
                    ));
                }
                #[cfg(test)]
                BEFORE_BACKUP_RENAME.with(|slot| {
                    if let Some(hook) = slot.borrow_mut().take() {
                        hook();
                    }
                });
            }
            fs::rename(&entry.path, &entry.backup).map_err(|error| {
                io_error(
                    format!("backup existing output {}", entry.path.display()),
                    error,
                )
            })?;
            if entry.path != manifest_path {
                let expected = expected_existing
                    .get(&entry.path)
                    .and_then(|hash| hash.as_deref())
                    .expect("verified ownership expectation exists");
                let backup_metadata = fs::symlink_metadata(&entry.backup).map_err(|error| {
                    io_error(
                        format!("inspect backed-up output {}", entry.backup.display()),
                        error,
                    )
                })?;
                if !backup_metadata.file_type().is_file()
                    || backup_metadata.file_type().is_symlink()
                {
                    return Err(ExportError::new(
                        ExportErrorKind::OwnershipConflict,
                        format!(
                            "managed output changed during backup; refusing to replace {}",
                            entry.path.display()
                        ),
                    ));
                }
                let actual = sha256_file(&entry.backup).map_err(|error| {
                    io_error(
                        format!("hash backed-up output {}", entry.backup.display()),
                        error,
                    )
                })?;
                if actual != expected {
                    return Err(ExportError::new(
                        ExportErrorKind::OwnershipConflict,
                        format!(
                            "managed output changed during backup; refusing to replace {}",
                            entry.path.display()
                        ),
                    ));
                }
            }
        }
        write_synced(
            &staging.path.join(TRANSACTION_BACKUPS_COMPLETE_MARKER_NAME),
            b"backups-complete",
        )?;
        resolved.backups_complete = true;

        for directory in &resolved.created_directories {
            fs::create_dir(directory).map_err(|error| {
                io_error(
                    format!("create output directory {}", directory.display()),
                    error,
                )
            })?;
        }

        for item in prepared {
            for output in &item.outputs {
                link_without_clobber(&output.staged, &output.target, "export output")?;
                mark_runtime_installed(&mut resolved, &output.target);
                finish_staged_install(&output.staged, &output.target, "export output")?;
            }
        }
        link_without_clobber(&manifest_temp, manifest_path, "export manifest")?;
        mark_runtime_installed(&mut resolved, manifest_path);
        finish_staged_install(&manifest_temp, manifest_path, "export manifest")?;
        write_synced(
            &staging.path.join(TRANSACTION_COMMIT_MARKER_NAME),
            b"committed",
        )?;
        Ok(())
    })();

    if let Err(error) = result {
        let _ = fs::remove_file(staging.path.join(TRANSACTION_COMMIT_MARKER_NAME));
        return match rollback_transaction(&resolved) {
            Err(rollback) => {
                staging.preserve();
                Err(ExportError::new(
                    ExportErrorKind::Io,
                    format!("{error}; transaction rollback was incomplete: {rollback}"),
                ))
            }
            Ok(()) => Err(error),
        };
    }
    Ok(())
}

fn link_without_clobber(staged: &Path, target: &Path, label: &str) -> Result<(), ExportError> {
    match fs::hard_link(staged, target) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Err(ExportError::new(
            ExportErrorKind::OwnershipConflict,
            format!(
                "refusing to overwrite a file created during export: {}",
                target.display()
            ),
        )),
        Err(error) => Err(io_error(
            format!("install {label} {}", target.display()),
            error,
        )),
    }
}

fn mark_runtime_installed(journal: &mut ResolvedTransactionJournal, path: &Path) {
    journal
        .entries
        .iter_mut()
        .find(|entry| entry.path == path)
        .expect("installed path has a resolved transaction entry")
        .install = true;
}

fn finish_staged_install(staged: &Path, target: &Path, label: &str) -> Result<(), ExportError> {
    fs::remove_file(staged).map_err(|error| {
        io_error(
            format!("finish installing {label} {}", target.display()),
            error,
        )
    })
}

fn plan_transaction_entries(
    root: &Path,
    out_dir: &Path,
    manifest_path: &Path,
    mutate: &BTreeSet<PathBuf>,
    install: &BTreeMap<PathBuf, String>,
    expected_existing: &BTreeMap<PathBuf, Option<String>>,
) -> Result<Vec<TransactionJournalEntry>, ExportError> {
    let mut entries = Vec::with_capacity(mutate.len());
    for path in mutate {
        let is_manifest = path == manifest_path;
        let target = if is_manifest {
            TransactionTarget::Manifest
        } else if path.starts_with(out_dir) && path != out_dir {
            TransactionTarget::Output {
                path: project_relative_path(root, path, "transaction path")?,
            }
        } else {
            return Err(ExportError::new(
                ExportErrorKind::UnsafePath,
                format!(
                    "transaction path is neither an output nor the state manifest: {}",
                    path.display()
                ),
            ));
        };
        let had_existing_file = match fs::symlink_metadata(path) {
            Ok(metadata)
                if metadata.file_type().is_file() && !metadata.file_type().is_symlink() =>
            {
                if !is_manifest {
                    let expected = expected_existing.get(path).ok_or_else(|| {
                        ExportError::new(
                            ExportErrorKind::InvalidManifest,
                            format!(
                                "transaction output has no ownership expectation: {}",
                                path.display()
                            ),
                        )
                    })?;
                    let Some(expected) = expected.as_deref() else {
                        return Err(ExportError::new(
                            ExportErrorKind::OwnershipConflict,
                            format!("refusing to overwrite unmanaged output: {}", path.display()),
                        ));
                    };
                    let actual = sha256_file(path).map_err(|error| {
                        io_error(format!("hash managed output {}", path.display()), error)
                    })?;
                    if actual != expected {
                        return Err(ExportError::new(
                            ExportErrorKind::OwnershipConflict,
                            format!(
                                "managed output changed before commit; refusing to replace {}",
                                path.display()
                            ),
                        ));
                    }
                }
                true
            }
            Ok(_) => {
                return Err(ExportError::new(
                    ExportErrorKind::OwnershipConflict,
                    format!("owned output is not a regular file: {}", path.display()),
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                return Err(io_error(
                    format!("inspect transaction path {}", path.display()),
                    error,
                ));
            }
        };
        if !had_existing_file && !install.contains_key(path) {
            continue;
        }
        entries.push(TransactionJournalEntry {
            target,
            had_existing_file,
            install: install.contains_key(path),
            install_sha256: install.get(path).cloned(),
        });
    }
    Ok(entries)
}

fn plan_created_directories(
    root: &Path,
    out_dir: &Path,
    manifest_path: &Path,
    install: &BTreeMap<PathBuf, String>,
    entries: &[TransactionJournalEntry],
) -> Result<Vec<String>, ExportError> {
    let stale_files = entries
        .iter()
        .filter(|entry| entry.had_existing_file && !entry.install)
        .filter_map(|entry| match &entry.target {
            TransactionTarget::Output { path } => Some(root.join(path)),
            TransactionTarget::Manifest => None,
        })
        .collect::<BTreeSet<_>>();
    let mut directories = BTreeSet::<PathBuf>::new();

    for target in install.keys() {
        if target == manifest_path {
            continue;
        }
        let parent = target.parent().ok_or_else(|| {
            ExportError::new(
                ExportErrorKind::UnsafePath,
                format!("transaction target has no parent: {}", target.display()),
            )
        })?;
        let relative_parent = parent.strip_prefix(out_dir).map_err(|_| {
            ExportError::new(
                ExportErrorKind::UnsafePath,
                format!(
                    "transaction parent escaped the output directory: {}",
                    parent.display()
                ),
            )
        })?;
        let mut current = out_dir.to_path_buf();
        let mut ancestor_will_be_created = false;
        for component in relative_parent.components() {
            current.push(component.as_os_str());
            if ancestor_will_be_created {
                directories.insert(current.clone());
                continue;
            }
            match fs::symlink_metadata(&current) {
                Ok(metadata)
                    if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {}
                Ok(metadata)
                    if metadata.file_type().is_file()
                        && !metadata.file_type().is_symlink()
                        && stale_files.contains(&current) =>
                {
                    directories.insert(current.clone());
                    ancestor_will_be_created = true;
                }
                Ok(_) => {
                    return Err(ExportError::new(
                        ExportErrorKind::OwnershipConflict,
                        format!(
                            "output parent is not a managed directory: {}",
                            current.display()
                        ),
                    ));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    directories.insert(current.clone());
                    ancestor_will_be_created = true;
                }
                Err(error) => {
                    return Err(io_error(
                        format!("inspect output parent {}", current.display()),
                        error,
                    ));
                }
            }
        }
    }

    let mut directories = directories.into_iter().collect::<Vec<_>>();
    directories.sort_by_key(|path| path.components().count());
    directories
        .into_iter()
        .map(|path| project_relative_path(root, &path, "created output directory"))
        .collect()
}

fn resolve_transaction_journal(
    root: &Path,
    out_dir: &Path,
    manifest_path: &Path,
    staging: &Path,
    journal: TransactionJournal,
) -> Result<ResolvedTransactionJournal, ExportError> {
    if journal.schema_version != TRANSACTION_JOURNAL_SCHEMA_VERSION {
        return Err(ExportError::new(
            ExportErrorKind::InvalidManifest,
            format!(
                "unsupported transaction journal schemaVersion {}; expected {}; transaction evidence was retained at {} for manual inspection",
                journal.schema_version,
                TRANSACTION_JOURNAL_SCHEMA_VERSION,
                staging.display()
            ),
        ));
    }
    let backup_root = validate_transaction_backup_root(staging)?;
    if journal.entries.is_empty() || !journal.entries.iter().any(|entry| entry.install) {
        return Err(ExportError::new(
            ExportErrorKind::InvalidManifest,
            "transaction journal does not describe an installed file",
        ));
    }

    let mut seen_targets = BTreeSet::new();
    let mut entries = Vec::with_capacity(journal.entries.len());
    for (index, entry) in journal.entries.into_iter().enumerate() {
        let target_description = match &entry.target {
            TransactionTarget::Output { path } => path.as_str(),
            TransactionTarget::Manifest => "state manifest",
        };
        if !entry.had_existing_file && !entry.install {
            return Err(ExportError::new(
                ExportErrorKind::InvalidManifest,
                format!(
                    "transaction journal path is neither backed up nor installed: {}",
                    target_description
                ),
            ));
        }
        if !seen_targets.insert(entry.target.clone()) {
            return Err(ExportError::new(
                ExportErrorKind::InvalidManifest,
                format!("transaction journal repeats target {target_description}"),
            ));
        }
        match (&entry.install_sha256, entry.install) {
            (Some(hash), true) if is_lower_sha256(hash) => {}
            (None, false) => {}
            _ => {
                return Err(ExportError::new(
                    ExportErrorKind::InvalidManifest,
                    format!(
                        "transaction journal has an invalid install checksum for {}",
                        target_description
                    ),
                ));
            }
        }
        let path = match &entry.target {
            TransactionTarget::Manifest => manifest_path.to_path_buf(),
            TransactionTarget::Output { path } => {
                validate_normalized_journal_path(path, "transaction path")?;
                let lexical = root.join(path);
                let resolved = project_path(root, &lexical, "transaction journal path")?;
                if resolved != lexical || !resolved.starts_with(out_dir) || resolved == out_dir {
                    return Err(ExportError::new(
                        ExportErrorKind::InvalidManifest,
                        format!("transaction journal claims an unsafe path: {path}"),
                    ));
                }
                resolved
            }
        };
        entries.push(ResolvedTransactionEntry {
            path,
            backup: backup_root.join(index.to_string()),
            had_existing_file: entry.had_existing_file,
            install: entry.install,
            install_sha256: entry.install_sha256,
        });
    }

    let mut seen_directories = BTreeSet::new();
    let mut created_directories = Vec::with_capacity(journal.created_directories.len());
    for directory in journal.created_directories {
        validate_normalized_journal_path(&directory, "created directory")?;
        if !seen_directories.insert(directory.clone()) {
            return Err(ExportError::new(
                ExportErrorKind::InvalidManifest,
                format!("transaction journal repeats directory {directory}"),
            ));
        }
        let lexical = root.join(&directory);
        let path = project_path(root, &lexical, "transaction-created directory")?;
        if path != lexical || !path.starts_with(out_dir) || path == out_dir {
            return Err(ExportError::new(
                ExportErrorKind::InvalidManifest,
                format!("transaction journal claims an unsafe directory: {directory}"),
            ));
        }
        created_directories.push(path);
    }
    created_directories.sort_by_key(|path| path.components().count());
    for directory in &created_directories {
        if !entries.iter().any(|entry| {
            entry.install && entry.path.starts_with(directory) && entry.path != *directory
        }) {
            return Err(ExportError::new(
                ExportErrorKind::InvalidManifest,
                format!(
                    "transaction-created directory is not an installed path parent: {}",
                    directory.display()
                ),
            ));
        }
    }

    Ok(ResolvedTransactionJournal {
        entries,
        created_directories,
        backups_complete: false,
    })
}

fn validate_transaction_backup_root(staging: &Path) -> Result<PathBuf, ExportError> {
    let backup_root = staging.join("backup");
    let metadata = fs::symlink_metadata(&backup_root).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            ExportError::new(
                ExportErrorKind::InvalidManifest,
                format!(
                    "transaction backup root is missing: {}",
                    backup_root.display()
                ),
            )
        } else {
            io_error(
                format!("inspect transaction backup root {}", backup_root.display()),
                error,
            )
        }
    })?;
    if !metadata.file_type().is_dir()
        || metadata.file_type().is_symlink()
        || metadata_is_reparse_point(&metadata)
    {
        return Err(ExportError::new(
            ExportErrorKind::InvalidManifest,
            format!(
                "transaction backup root is not a real directory: {}",
                backup_root.display()
            ),
        ));
    }
    let canonical = fs::canonicalize(&backup_root).map_err(|error| {
        io_error(
            format!("resolve transaction backup root {}", backup_root.display()),
            error,
        )
    })?;
    if canonical != backup_root || !canonical.starts_with(staging) {
        return Err(ExportError::new(
            ExportErrorKind::InvalidManifest,
            format!(
                "transaction backup root escaped its staging directory: {}",
                backup_root.display()
            ),
        ));
    }
    Ok(backup_root)
}

fn validate_normalized_journal_path(path: &str, label: &str) -> Result<(), ExportError> {
    let normalized = normalize_relative_path(Path::new(path))
        .map_err(|error| ExportError::new(ExportErrorKind::InvalidManifest, error.to_string()))?;
    if path.is_empty() || normalized != path {
        return Err(ExportError::new(
            ExportErrorKind::InvalidManifest,
            format!("transaction journal contains a non-normalized {label}: {path}"),
        ));
    }
    Ok(())
}

fn project_relative_path(root: &Path, path: &Path, label: &str) -> Result<String, ExportError> {
    let relative = path.strip_prefix(root).map_err(|_| {
        ExportError::new(
            ExportErrorKind::UnsafePath,
            format!("{label} escaped the worktree root: {}", path.display()),
        )
    })?;
    normalize_relative_path(relative).map_err(map_discovery_error)
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn sha256_file(path: &Path) -> Result<String, std::io::Error> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn rollback_transaction(journal: &ResolvedTransactionJournal) -> Result<(), ExportError> {
    #[derive(Clone, Copy)]
    enum BackupState {
        Missing,
        Regular,
        Invalid,
    }

    let mut failures = Vec::new();
    let mut backup_states = Vec::with_capacity(journal.entries.len());

    for entry in &journal.entries {
        let state = match fs::symlink_metadata(&entry.backup) {
            Ok(metadata)
                if metadata.file_type().is_file() && !metadata.file_type().is_symlink() =>
            {
                if entry.had_existing_file {
                    BackupState::Regular
                } else {
                    failures.push(format!(
                        "unexpected transaction backup for a newly created path: {}",
                        entry.backup.display()
                    ));
                    BackupState::Invalid
                }
            }
            Ok(_) => {
                failures.push(format!(
                    "transaction backup is not a regular file: {}",
                    entry.backup.display()
                ));
                BackupState::Invalid
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if entry.had_existing_file && journal.backups_complete {
                    failures.push(format!(
                        "transaction backup is missing after the backups-complete marker: {}",
                        entry.backup.display()
                    ));
                }
                BackupState::Missing
            }
            Err(error) => {
                failures.push(format!(
                    "failed to inspect transaction backup {}: {error}",
                    entry.backup.display()
                ));
                BackupState::Invalid
            }
        };
        backup_states.push(state);
    }

    for (entry, backup_state) in journal
        .entries
        .iter()
        .zip(&backup_states)
        .rev()
        .filter(|(entry, _)| entry.install)
    {
        if (entry.had_existing_file && matches!(backup_state, BackupState::Regular))
            || (!entry.had_existing_file
                && journal.backups_complete
                && matches!(backup_state, BackupState::Missing))
        {
            let expected = entry
                .install_sha256
                .as_deref()
                .expect("installed transaction entries have a validated checksum");
            remove_installed_file(&entry.path, expected, &mut failures);
        } else if !entry.had_existing_file
            && !journal.backups_complete
            && matches!(backup_state, BackupState::Missing)
        {
            match fs::symlink_metadata(&entry.path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Ok(_) => failures.push(format!(
                    "refusing to remove transaction path without a backups-complete marker: {}",
                    entry.path.display()
                )),
                Err(error) => failures.push(format!(
                    "failed to inspect pre-install transaction path {}: {error}",
                    entry.path.display()
                )),
            }
        }
    }

    for directory in journal.created_directories.iter().rev() {
        match fs::remove_dir(directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => failures.push(format!(
                "failed to remove transaction-created directory {}: {error}",
                directory.display()
            )),
        }
    }

    for (entry, backup_state) in journal
        .entries
        .iter()
        .zip(&backup_states)
        .rev()
        .filter(|(entry, _)| entry.had_existing_file)
    {
        if matches!(backup_state, BackupState::Regular) {
            match fs::symlink_metadata(&entry.path) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if let Err(error) = fs::rename(&entry.backup, &entry.path) {
                        failures.push(format!(
                            "failed to restore {}: {error}",
                            entry.path.display()
                        ));
                    }
                }
                Ok(_) => failures.push(format!(
                    "refusing to restore over an occupied path: {}",
                    entry.path.display()
                )),
                Err(error) => failures.push(format!(
                    "failed to inspect restore target {}: {error}",
                    entry.path.display()
                )),
            }
        } else if matches!(backup_state, BackupState::Missing) && !journal.backups_complete {
            match fs::symlink_metadata(&entry.path) {
                Ok(metadata)
                    if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {}
                Ok(_) => failures.push(format!(
                    "original transaction path is not a regular file: {}",
                    entry.path.display()
                )),
                Err(error) => failures.push(format!(
                    "transaction lost both original and backup for {}: {error}",
                    entry.path.display()
                )),
            }
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(ExportError::new(ExportErrorKind::Io, failures.join("; ")))
    }
}

fn remove_installed_file(path: &Path, expected_sha256: &str, failures: &mut Vec<String>) {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {
            match sha256_file(path) {
                Ok(actual) if actual == expected_sha256 => {
                    if let Err(error) = fs::remove_file(path) {
                        failures.push(format!(
                            "failed to remove installed transaction file {}: {error}",
                            path.display()
                        ));
                    }
                }
                Ok(actual) => failures.push(format!(
                    "refusing to remove transaction path {} because its SHA-256 changed from {} to {}",
                    path.display(), expected_sha256, actual
                )),
                Err(error) => failures.push(format!(
                    "failed to hash installed transaction file {}: {error}",
                    path.display()
                )),
            }
        }
        Ok(_) => failures.push(format!(
            "refusing to remove a non-regular transaction path: {}",
            path.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => failures.push(format!(
            "failed to inspect installed transaction file {}: {error}",
            path.display()
        )),
    }
}

fn recover_abandoned_transactions(
    root: &Path,
    out_dir: &Path,
    state: &ExportStateLayout,
) -> Result<(), ExportError> {
    let mut pending = Vec::<(PathBuf, ResolvedTransactionJournal)>::new();
    for entry in fs::read_dir(&state.transactions).map_err(|error| {
        io_error(
            format!(
                "inspect state directory for abandoned transactions {}",
                state.transactions.display()
            ),
            error,
        )
    })? {
        let entry = entry.map_err(|error| {
            io_error(
                format!(
                    "inspect abandoned transaction below {}",
                    state.transactions.display()
                ),
                error,
            )
        })?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !is_transaction_staging_name(name) {
            continue;
        }
        let file_type = entry.file_type().map_err(|error| {
            io_error(
                format!(
                    "inspect transaction staging path {}",
                    entry.path().display()
                ),
                error,
            )
        })?;
        if !file_type.is_dir() || file_type.is_symlink() {
            return Err(ExportError::new(
                ExportErrorKind::OwnershipConflict,
                format!(
                    "reserved transaction staging path is not a directory: {}",
                    entry.path().display()
                ),
            ));
        }
        let journal_path = entry.path().join(TRANSACTION_JOURNAL_NAME);
        let metadata = match fs::symlink_metadata(&journal_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let rendering = entry.path().join(TRANSACTION_RENDERING_MARKER_NAME);
                // The journal is durable before any output mutation. A
                // reserved staging directory without one is therefore safe to
                // remove, including the crash window between directory
                // creation and writing the rendering marker. Validate a marker
                // when present so tampered state still fails closed.
                let _ = transaction_marker_exists(
                    &rendering,
                    b"rendering\n",
                    "transaction rendering marker",
                )?;
                fs::remove_dir_all(entry.path()).map_err(|error| {
                    io_error(
                        format!("clean abandoned render staging {}", entry.path().display()),
                        error,
                    )
                })?;
                continue;
            }
            Err(error) => {
                return Err(io_error(
                    format!("inspect transaction journal {}", journal_path.display()),
                    error,
                ));
            }
        };
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err(ExportError::new(
                ExportErrorKind::InvalidManifest,
                format!(
                    "transaction journal is not a regular file: {}",
                    journal_path.display()
                ),
            ));
        }
        let bytes = fs::read(&journal_path).map_err(|error| {
            io_error(
                format!("read transaction journal {}", journal_path.display()),
                error,
            )
        })?;
        let journal: TransactionJournal = serde_json::from_slice(&bytes).map_err(|error| {
            ExportError::new(
                ExportErrorKind::InvalidManifest,
                format!(
                    "invalid transaction journal {}: {error}",
                    journal_path.display()
                ),
            )
        })?;
        let mut resolved =
            resolve_transaction_journal(root, out_dir, &state.manifest, &entry.path(), journal)?;
        resolved.backups_complete = transaction_marker_exists(
            &entry.path().join(TRANSACTION_BACKUPS_COMPLETE_MARKER_NAME),
            b"backups-complete\n",
            "transaction backups-complete marker",
        )?;

        let committed = entry.path().join(TRANSACTION_COMMIT_MARKER_NAME);
        if transaction_marker_exists(&committed, b"committed\n", "transaction commit marker")? {
            if !resolved.backups_complete {
                return Err(ExportError::new(
                    ExportErrorKind::InvalidManifest,
                    format!(
                        "committed transaction is missing its backups-complete marker; transaction evidence was retained at {}",
                        entry.path().display()
                    ),
                ));
            }
            fs::remove_dir_all(entry.path()).map_err(|error| {
                io_error(
                    format!("clean committed transaction {}", entry.path().display()),
                    error,
                )
            })?;
        } else {
            pending.push((entry.path(), resolved));
        }
    }

    if pending.len() > 1 {
        return Err(ExportError::new(
            ExportErrorKind::OwnershipConflict,
            "multiple abandoned export transactions require manual inspection",
        ));
    }
    if let Some((staging, resolved)) = pending.pop() {
        rollback_transaction(&resolved)?;
        fs::remove_dir_all(&staging).map_err(|error| {
            io_error(
                format!("clean recovered transaction {}", staging.display()),
                error,
            )
        })?;
    }
    Ok(())
}

fn transaction_marker_exists(
    path: &Path,
    expected: &[u8],
    label: &str,
) -> Result<bool, ExportError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(io_error(
                format!("inspect {label} {}", path.display()),
                error,
            ));
        }
    };
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(ExportError::new(
            ExportErrorKind::InvalidManifest,
            format!("{label} is not a regular file: {}", path.display()),
        ));
    }
    let bytes = fs::read(path)
        .map_err(|error| io_error(format!("read {label} {}", path.display()), error))?;
    if bytes != expected {
        return Err(ExportError::new(
            ExportErrorKind::InvalidManifest,
            format!("invalid {label}: {}", path.display()),
        ));
    }
    Ok(true)
}

fn is_transaction_staging_name(name: &str) -> bool {
    let Some(suffix) = name.strip_prefix(STAGING_DIRECTORY_PREFIX) else {
        return false;
    };
    let Some((process_id, sequence)) = suffix.split_once('-') else {
        return false;
    };
    !process_id.is_empty()
        && !sequence.is_empty()
        && process_id.bytes().all(|byte| byte.is_ascii_digit())
        && sequence.bytes().all(|byte| byte.is_ascii_digit())
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), ExportError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| io_error(format!("create {}", path.display()), error))?;
    file.write_all(bytes)
        .and_then(|()| file.write_all(b"\n"))
        .and_then(|()| file.sync_all())
        .map_err(|error| io_error(format!("write {}", path.display()), error))
}

fn io_error(context: String, error: std::io::Error) -> ExportError {
    ExportError::new(ExportErrorKind::Io, format!("{context}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_manifest() -> Manifest {
        Manifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            inputs: BTreeMap::new(),
        }
    }

    fn state_layout(root: &Path) -> ExportStateLayout {
        let state = ExportStateLayout::new(root.join("state"));
        fs::create_dir_all(&state.transactions).expect("state transactions");
        state
    }

    fn output_target(path: &str) -> TransactionTarget {
        TransactionTarget::Output {
            path: path.to_string(),
        }
    }

    fn write_transaction_journal(staging: &Path, journal: &TransactionJournal) {
        let bytes = serde_json::to_vec_pretty(journal).expect("journal JSON");
        fs::write(staging.join(TRANSACTION_JOURNAL_NAME), bytes).expect("transaction journal");
    }

    fn write_backups_complete_marker(staging: &Path) {
        fs::write(
            staging.join(TRANSACTION_BACKUPS_COMPLETE_MARKER_NAME),
            b"backups-complete\n",
        )
        .expect("backups-complete marker");
    }

    fn installed_hash(path: &Path) -> Option<String> {
        Some(sha256_file(path).expect("installed file hash"))
    }

    fn bytes_hash(bytes: &[u8]) -> Option<String> {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        Some(format!("{:x}", hasher.finalize()))
    }

    fn manifest_with_svg_output(relative: &str, sha256: &str) -> Manifest {
        Manifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            inputs: BTreeMap::from([(
                "diagram.puml".to_string(),
                ManifestInput {
                    formats: BTreeMap::from([(
                        OutputFormat::Svg,
                        ManifestFormat {
                            outputs: vec![ManifestOutput {
                                path: relative.to_string(),
                                sha256: sha256.to_string(),
                            }],
                            tool_version: "test-tool".to_string(),
                            renderer: RendererMetadata {
                                mode: "test-renderer".to_string(),
                                version: "test-renderer-version".to_string(),
                            },
                            environment: EnvironmentMetadata {
                                java_version: None,
                                graphviz_version: None,
                                os: "test-os".to_string(),
                                architecture: "test-architecture".to_string(),
                            },
                        },
                    )]),
                },
            )]),
        }
    }

    #[test]
    fn export_lock_serializes_writers_without_rewriting_the_state_file() {
        let project = TempDir::new().expect("project");
        let lock = project.path().join("export.lock");
        fs::write(&lock, b"stale pid").expect("stale lock file");

        let first = ExportLock::acquire_with_policy(
            &lock,
            Duration::from_millis(25),
            Duration::from_millis(1),
        )
        .expect("an unlocked crash leftover must be reusable");
        assert_eq!(fs::read(&lock).expect("lock contents"), b"stale pid");

        let error = ExportLock::acquire_with_policy(
            &lock,
            Duration::from_millis(10),
            Duration::from_millis(1),
        )
        .expect_err("a concurrent writer must not enter the transaction");
        assert_eq!(error.kind, ExportErrorKind::Environment);
        assert!(error.message.contains("waiting for export lock"));

        drop(first);
        ExportLock::acquire_with_policy(&lock, Duration::from_millis(25), Duration::from_millis(1))
            .expect("dropping the owner must release the operating-system lock");
    }

    #[cfg(unix)]
    #[test]
    fn cross_filesystem_transactions_are_rejected_before_commit() {
        use std::os::unix::fs::MetadataExt;

        let project = TempDir::new().expect("project");
        let out_device = fs::metadata(project.path()).expect("output device").dev();
        let other_device = fs::metadata("/dev").expect("device filesystem").dev();
        if out_device == other_device {
            return;
        }

        let error = ensure_transaction_filesystem(project.path(), Path::new("/dev"))
            .expect_err("different filesystems must not enter an atomic transaction");

        assert_eq!(error.kind, ExportErrorKind::Environment);
        assert!(error.message.contains("different filesystems"));
    }

    #[cfg(unix)]
    #[test]
    fn export_lock_rejects_a_symlink_without_touching_its_target() {
        use std::os::unix::fs::symlink;

        let project = TempDir::new().expect("project");
        let outside = project.path().join("outside.txt");
        let lock = project.path().join("export.lock");
        fs::write(&outside, b"user-owned").expect("outside file");
        symlink(&outside, &lock).expect("lock symlink");

        let error = ExportLock::acquire_with_policy(
            &lock,
            Duration::from_millis(25),
            Duration::from_millis(1),
        )
        .expect_err("a lock symlink must be rejected");

        assert_eq!(error.kind, ExportErrorKind::OwnershipConflict);
        assert_eq!(fs::read(outside).expect("outside file"), b"user-owned");
    }

    #[test]
    fn failed_commit_rolls_back_outputs_and_keeps_state_outside_output() {
        let project = TempDir::new().expect("project");
        let root = fs::canonicalize(project.path()).expect("canonical project");
        let out_dir = root.join("out");
        fs::create_dir_all(&out_dir).expect("output directory");
        let state = state_layout(&root);
        let staging = StagingDirectory::create(&state.transactions).expect("staging");
        let first_staged = staging.path.join("first.svg");
        fs::write(&first_staged, b"first").expect("first staged output");
        let first_sha256 = sha256_file(&first_staged).expect("staged output hash");

        let prepared = vec![PreparedInput {
            input: "diagram.puml".to_string(),
            format: OutputFormat::Svg,
            outputs: vec![
                PreparedOutput {
                    staged: first_staged.clone(),
                    target: out_dir.join("new/deep/first.svg"),
                    relative: "out/new/deep/first.svg".to_string(),
                    sha256: first_sha256.clone(),
                },
                PreparedOutput {
                    staged: first_staged,
                    target: out_dir.join("second.svg"),
                    relative: "out/second.svg".to_string(),
                    sha256: first_sha256,
                },
            ],
        }];

        let error = commit_transaction(
            &root,
            &out_dir,
            &state.manifest,
            &staging,
            None,
            &prepared,
            &test_manifest(),
        )
        .expect_err("the reused staged file must fail during install");

        assert_eq!(error.kind, ExportErrorKind::Io);
        assert!(!out_dir.join("new").exists());
        assert!(!out_dir.join("second.svg").exists());
        assert!(!state.manifest.exists());
        assert!(fs::read_dir(&out_dir)
            .expect("output directory")
            .next()
            .is_none());
    }

    #[test]
    fn commit_never_clobbers_a_file_created_after_transaction_planning() {
        let project = TempDir::new().expect("project");
        let root = fs::canonicalize(project.path()).expect("canonical project");
        let out_dir = root.join("out");
        fs::create_dir_all(&out_dir).expect("output directory");
        let state = state_layout(&root);
        let staging = StagingDirectory::create(&state.transactions).expect("staging");
        let staged = staging.path.join("diagram.svg");
        fs::write(&staged, b"new export").expect("staged output");
        let staged_sha256 = sha256_file(&staged).expect("staged output hash");
        let target = out_dir.join("diagram.svg");
        let prepared = vec![PreparedInput {
            input: "diagram.puml".to_string(),
            format: OutputFormat::Svg,
            outputs: vec![PreparedOutput {
                staged,
                target: target.clone(),
                relative: "out/diagram.svg".to_string(),
                sha256: staged_sha256.clone(),
            }],
        }];
        let next = manifest_with_svg_output("out/diagram.svg", &staged_sha256);
        let raced_target = target.clone();
        BEFORE_COMMIT_MUTATION.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move || {
                fs::write(raced_target, b"user-created during commit").expect("raced user output");
            }));
        });

        let error = commit_transaction(
            &root,
            &out_dir,
            &state.manifest,
            &staging,
            None,
            &prepared,
            &next,
        )
        .expect_err("a raced target must never be overwritten");

        assert_eq!(error.kind, ExportErrorKind::OwnershipConflict);
        assert_eq!(
            fs::read(target).expect("raced user output"),
            b"user-created during commit"
        );
        assert!(!state.manifest.exists());
    }

    #[test]
    fn commit_restores_a_replacement_raced_between_hash_and_backup_rename() {
        let project = TempDir::new().expect("project");
        let root = fs::canonicalize(project.path()).expect("canonical project");
        let out_dir = root.join("out");
        fs::create_dir_all(&out_dir).expect("output directory");
        let state = state_layout(&root);
        let target = out_dir.join("diagram.svg");
        fs::write(&target, b"previous managed output").expect("previous output");
        let previous_sha256 = sha256_file(&target).expect("previous output hash");
        let previous = manifest_with_svg_output("out/diagram.svg", &previous_sha256);
        let previous_manifest_bytes = serde_json::to_vec_pretty(&previous).expect("manifest JSON");
        fs::write(&state.manifest, &previous_manifest_bytes).expect("previous manifest");

        let staging = StagingDirectory::create(&state.transactions).expect("staging");
        let staged = staging.path.join("diagram.svg");
        fs::write(&staged, b"new export").expect("staged output");
        let staged_sha256 = sha256_file(&staged).expect("staged output hash");
        let prepared = vec![PreparedInput {
            input: "diagram.puml".to_string(),
            format: OutputFormat::Svg,
            outputs: vec![PreparedOutput {
                staged,
                target: target.clone(),
                relative: "out/diagram.svg".to_string(),
                sha256: staged_sha256.clone(),
            }],
        }];
        let next = manifest_with_svg_output("out/diagram.svg", &staged_sha256);
        let raced_target = target.clone();
        BEFORE_BACKUP_RENAME.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move || {
                fs::write(raced_target, b"user replacement during backup")
                    .expect("raced replacement");
            }));
        });

        let error = commit_transaction(
            &root,
            &out_dir,
            &state.manifest,
            &staging,
            Some(&previous),
            &prepared,
            &next,
        )
        .expect_err("a replacement raced at backup must be restored");

        assert_eq!(error.kind, ExportErrorKind::OwnershipConflict);
        assert_eq!(
            fs::read(target).expect("restored raced replacement"),
            b"user replacement during backup"
        );
        assert_eq!(
            fs::read(&state.manifest).expect("restored manifest"),
            previous_manifest_bytes
        );
    }

    #[test]
    fn abandoned_render_staging_without_a_journal_is_cleaned_from_external_state() {
        let project = TempDir::new().expect("project");
        let root = fs::canonicalize(project.path()).expect("canonical project");
        let out_dir = root.join("out");
        fs::create_dir_all(&out_dir).expect("output directory");
        let visible_output = out_dir.join("diagram.svg");
        fs::write(&visible_output, b"existing output").expect("existing output");
        let state = state_layout(&root);
        let staging = StagingDirectory::create(&state.transactions).expect("staging");
        let staging_path = staging.path.clone();
        fs::write(staging_path.join("rendered.svg"), b"unfinished render")
            .expect("unfinished render");
        staging.preserve();
        drop(staging);

        recover_abandoned_transactions(&root, &out_dir, &state)
            .expect("clean abandoned render staging");

        assert!(!staging_path.exists());
        assert_eq!(
            fs::read(visible_output).expect("visible output"),
            b"existing output"
        );
    }

    #[test]
    fn crash_before_the_rendering_marker_is_cleaned_from_external_state() {
        let project = TempDir::new().expect("project");
        let root = fs::canonicalize(project.path()).expect("canonical project");
        let out_dir = root.join("out");
        fs::create_dir_all(&out_dir).expect("output directory");
        let state = state_layout(&root);
        let staging = state
            .transactions
            .join(format!("{STAGING_DIRECTORY_PREFIX}123-456"));
        fs::create_dir(&staging).expect("bare staging directory");

        recover_abandoned_transactions(&root, &out_dir, &state)
            .expect("clean pre-marker staging directory");

        assert!(!staging.exists());
    }

    #[test]
    fn abandoned_transaction_restores_manifest_and_outputs_from_external_state() {
        let project = TempDir::new().expect("project");
        let root = fs::canonicalize(project.path()).expect("canonical project");
        let out_dir = root.join("out");
        fs::create_dir_all(&out_dir).expect("output directory");
        let state = state_layout(&root);
        let staging = state
            .transactions
            .join(format!("{STAGING_DIRECTORY_PREFIX}999999-0"));
        let backup = staging.join("backup");
        fs::create_dir_all(&backup).expect("backup directory");

        let output = out_dir.join("diagram.svg");
        fs::write(&state.manifest, b"old manifest").expect("old manifest");
        fs::write(&output, b"old output").expect("old output");
        fs::rename(&state.manifest, backup.join("0")).expect("backup manifest");
        fs::rename(&output, backup.join("1")).expect("backup output");
        fs::write(&state.manifest, b"new manifest").expect("new manifest");
        fs::write(&output, b"new output").expect("new output");

        write_transaction_journal(
            &staging,
            &TransactionJournal {
                schema_version: TRANSACTION_JOURNAL_SCHEMA_VERSION,
                entries: vec![
                    TransactionJournalEntry {
                        target: TransactionTarget::Manifest,
                        had_existing_file: true,
                        install: true,
                        install_sha256: installed_hash(&state.manifest),
                    },
                    TransactionJournalEntry {
                        target: output_target("out/diagram.svg"),
                        had_existing_file: true,
                        install: true,
                        install_sha256: installed_hash(&output),
                    },
                ],
                created_directories: Vec::new(),
            },
        );
        write_backups_complete_marker(&staging);

        recover_abandoned_transactions(&root, &out_dir, &state).expect("recover transaction");

        assert_eq!(
            fs::read(&state.manifest).expect("manifest"),
            b"old manifest"
        );
        assert_eq!(fs::read(output).expect("output"), b"old output");
        assert!(!staging.exists());
    }

    #[test]
    fn recovery_rejects_output_traversal_without_touching_outside_files() {
        let project = TempDir::new().expect("project");
        let root = fs::canonicalize(project.path()).expect("canonical project");
        let out_dir = root.join("out");
        fs::create_dir_all(&out_dir).expect("output directory");
        let state = state_layout(&root);
        let staging = state
            .transactions
            .join(format!("{STAGING_DIRECTORY_PREFIX}999999-0"));
        fs::create_dir_all(staging.join("backup")).expect("backup directory");
        let outside = root.join("outside.txt");
        fs::write(&outside, b"user-owned").expect("outside file");
        write_transaction_journal(
            &staging,
            &TransactionJournal {
                schema_version: TRANSACTION_JOURNAL_SCHEMA_VERSION,
                entries: vec![TransactionJournalEntry {
                    target: output_target("../outside.txt"),
                    had_existing_file: false,
                    install: true,
                    install_sha256: bytes_hash(b"transaction output"),
                }],
                created_directories: Vec::new(),
            },
        );
        write_backups_complete_marker(&staging);

        let error = recover_abandoned_transactions(&root, &out_dir, &state)
            .expect_err("escaped journal must be rejected");

        assert_eq!(error.kind, ExportErrorKind::InvalidManifest);
        assert_eq!(fs::read(outside).expect("outside file"), b"user-owned");
        assert!(staging.exists());
    }

    #[test]
    fn recovery_refuses_to_delete_a_changed_new_output() {
        let project = TempDir::new().expect("project");
        let root = fs::canonicalize(project.path()).expect("canonical project");
        let out_dir = root.join("out");
        fs::create_dir_all(&out_dir).expect("output directory");
        let state = state_layout(&root);
        let staging = state
            .transactions
            .join(format!("{STAGING_DIRECTORY_PREFIX}999999-0"));
        fs::create_dir_all(staging.join("backup")).expect("backup directory");
        let output = out_dir.join("diagram.svg");
        fs::write(&output, b"user replacement after crash").expect("replacement output");
        write_transaction_journal(
            &staging,
            &TransactionJournal {
                schema_version: TRANSACTION_JOURNAL_SCHEMA_VERSION,
                entries: vec![TransactionJournalEntry {
                    target: output_target("out/diagram.svg"),
                    had_existing_file: false,
                    install: true,
                    install_sha256: bytes_hash(b"transaction output"),
                }],
                created_directories: Vec::new(),
            },
        );
        write_backups_complete_marker(&staging);

        let error = recover_abandoned_transactions(&root, &out_dir, &state)
            .expect_err("changed output must require inspection");

        assert_eq!(error.kind, ExportErrorKind::Io);
        assert!(error.message.contains("SHA-256 changed"));
        assert_eq!(
            fs::read(output).expect("replacement output"),
            b"user replacement after crash"
        );
        assert!(staging.exists());
    }
}
