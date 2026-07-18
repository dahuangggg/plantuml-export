use std::collections::BTreeSet;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use url::{Host, Url};

use crate::cli::{CheckArgs, ExportArgs, Layout as CliLayout, OutputFormat, RendererMode};
use crate::config::{RemoteIncludes, ResolvedConfig};
use crate::diagnostics::parse_standard_report;
use crate::discovery::{
    discover_inputs, normalize_relative_path, DiscoveredInput, DiscoveryError, DiscoveryErrorKind,
    DiscoveryOptions,
};
use crate::export::{
    EnvironmentMetadata, ExportError, ExportErrorKind, ExportReport, ExportRequest, ExportSession,
    Renderer as ExportRenderer, RendererError as ExportRendererError, RendererMetadata,
};
use crate::export_state::workspace_state_dir;
use crate::process_control::{execute_cancellable, ControlledProcessFailure};
use crate::renderer::{
    self, build_render_command, build_syntax_command, check_health, create_syntax_output_dir,
    current_managed_java_asset, ensure_managed_jar, ensure_managed_java, managed_java_path,
    CommandExecutor, CommandSpec, Format, HealthReport, HealthStatus, Layout, ProcessFailure,
    ProcessOutput, RenderRequest, RenderSecurity, Renderer, SecurityProfile, StdCommandExecutor,
    SyntaxRequest, MANAGED_PLANTUML_VERSION,
};
use crate::AppError;

const RENDER_TIMEOUT: Duration = Duration::from_secs(120);
const CHECK_TIMEOUT: Duration = Duration::from_secs(10);
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_ERROR_DETAIL_BYTES: usize = 8 * 1024;

#[derive(Clone, Debug, Default)]
pub struct ExportCancellation {
    cancelled: Arc<AtomicBool>,
}

impl ExportCancellation {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

/// Return the platform-native, user-scoped cache used by managed PlantUML.
/// Merely resolving this path never creates it and never performs a download.
pub fn default_cache_dir() -> Result<PathBuf, AppError> {
    #[cfg(target_os = "windows")]
    {
        return env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|base| base.join("plantuml-export/cache"))
            .ok_or_else(|| {
                AppError::environment(
                    "cache_directory_unavailable",
                    "LOCALAPPDATA is not set; cannot locate the managed PlantUML cache",
                )
            });
    }

    #[cfg(target_os = "macos")]
    {
        return env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join("Library/Caches/plantuml-export"))
            .ok_or_else(|| {
                AppError::environment(
                    "cache_directory_unavailable",
                    "HOME is not set; cannot locate the managed PlantUML cache",
                )
            });
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        return env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
            .map(|base| base.join("plantuml-export"))
            .ok_or_else(|| {
                AppError::environment(
                    "cache_directory_unavailable",
                    "neither XDG_CACHE_HOME nor HOME is set; cannot locate the managed PlantUML cache",
                )
            });
    }

    #[allow(unreachable_code)]
    Err(AppError::environment(
        "cache_directory_unavailable",
        "this platform has no managed PlantUML cache location",
    ))
}

/// Map a resolved configuration to exactly one renderer. This function never
/// probes another renderer and never installs managed assets.
pub fn renderer_from_config(
    config: &ResolvedConfig,
    cache_dir: PathBuf,
) -> Result<Renderer, AppError> {
    match config.renderer {
        RendererMode::Managed => {
            let asset = current_managed_java_asset().map_err(|error| {
                AppError::environment("managed_java_unsupported", error.to_string())
            })?;
            Ok(Renderer::Managed {
                java: managed_java_path(&cache_dir, asset),
                cache_dir,
            })
        }
        RendererMode::Binary => config
            .binary_path
            .clone()
            .map(|executable| Renderer::Binary { executable })
            .ok_or_else(|| {
                AppError::environment(
                    "binary_path_required",
                    "renderer `binary` requires --plantuml or binaryPath in user config",
                )
            }),
        RendererMode::Jar => config
            .jar_path
            .clone()
            .map(|jar| Renderer::Jar {
                java: config.java_path.clone(),
                jar,
            })
            .ok_or_else(|| {
                AppError::environment(
                    "jar_path_required",
                    "renderer `jar` requires --jar or jarPath in user config",
                )
            }),
    }
}

/// Resolve the configured renderer without installing managed assets.
pub fn configured_renderer(config: &ResolvedConfig) -> Result<Renderer, AppError> {
    let cache_dir = if config.renderer == RendererMode::Managed {
        default_cache_dir()?
    } else {
        PathBuf::new()
    };
    renderer_from_config(config, cache_dir)
}

pub fn resolve_render_security(config: &ResolvedConfig) -> RenderSecurity {
    if config.offline || config.remote_includes == RemoteIncludes::Disabled {
        return RenderSecurity {
            profile: SecurityProfile::Allowlist,
            allowed_remote_urls: Vec::new(),
        };
    }
    RenderSecurity {
        profile: match config.remote_includes {
            RemoteIncludes::Public => SecurityProfile::Internet,
            RemoteIncludes::Allowlist => SecurityProfile::Allowlist,
            RemoteIncludes::Disabled => unreachable!("disabled handled above"),
        },
        allowed_remote_urls: config.allowed_remote_urls.clone(),
    }
}

/// Canonicalize include roots and enforce the v0.1 project-local read boundary.
pub fn resolve_include_paths(config: &ResolvedConfig) -> Result<Vec<PathBuf>, AppError> {
    let root = config.root.canonicalize().map_err(|error| {
        AppError::usage(
            "invalid_root",
            format!(
                "failed to resolve worktree root {}: {error}",
                config.root.display()
            ),
        )
    })?;
    let mut resolved = BTreeSet::new();
    for configured in &config.include_paths {
        let explicitly_absolute = configured.is_absolute();
        let candidate = if explicitly_absolute {
            configured.clone()
        } else {
            root.join(configured)
        };
        let canonical = candidate.canonicalize().map_err(|error| {
            AppError::usage(
                "include_path_not_found",
                format!(
                    "include path does not exist at {}: {error}",
                    candidate.display()
                ),
            )
        })?;
        if !canonical.is_dir() {
            return Err(AppError::usage(
                "include_path_not_directory",
                format!("include path is not a directory: {}", canonical.display()),
            ));
        }
        if !explicitly_absolute && !canonical.starts_with(&root) {
            return Err(AppError::usage(
                "include_path_outside_root",
                format!(
                    "include path must stay inside the worktree root: {}",
                    configured.display()
                ),
            ));
        }
        resolved.insert(canonical);
    }
    Ok(resolved.into_iter().collect())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentHealthData {
    pub status: &'static str,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthData {
    pub ready: bool,
    pub renderer_mode: RendererMode,
    pub format: OutputFormat,
    pub layout: CliLayout,
    pub renderer: ComponentHealthData,
    pub java: ComponentHealthData,
    pub graphviz: ComponentHealthData,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckDiagnostic {
    /// One-based source line, matching PlantUML's standard report.
    pub line: u32,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckFailure {
    pub input: String,
    pub code: String,
    pub message: String,
    pub diagnostics: Vec<CheckDiagnostic>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckReport {
    pub checked: Vec<String>,
    pub failures: Vec<CheckFailure>,
}

pub fn run_health(config: &ResolvedConfig) -> Result<HealthData, AppError> {
    let renderer = configured_renderer(config)?;
    Ok(health_data(
        config,
        &renderer,
        renderer_format(config.format),
        renderer_layout(config.layout),
    ))
}

/// Prepare and validate the renderer required by LSP diagnostics and exports.
/// Managed mode may install only its pinned JRE and JAR; no alternate renderer
/// is considered. Syntax diagnostics use SVG/Smetana and never require Graphviz.
pub fn prepare_lsp_renderer(config: &ResolvedConfig) -> Result<(), AppError> {
    prepare_syntax_renderer(config).map(drop)
}

pub fn run_export(config: &ResolvedConfig, args: &ExportArgs) -> Result<ExportReport, AppError> {
    run_export_selection(
        config,
        &args.inputs,
        args.workspace,
        args.require_input,
        args.keep_going,
        None,
    )
}

/// Export one saved file from an already-running native integration such as the
/// Zed language server. The caller supplies the format explicitly because CLI
/// flag resolution has not run for an LSP request.
pub fn run_export_file(
    config: &ResolvedConfig,
    input: PathBuf,
    format: OutputFormat,
) -> Result<ExportReport, AppError> {
    let mut effective = config.clone();
    effective.format = format;
    run_export_selection(&effective, &[input], false, true, false, None)
}

pub fn run_export_file_cancellable(
    config: &ResolvedConfig,
    input: PathBuf,
    format: OutputFormat,
    cancellation: ExportCancellation,
) -> Result<ExportReport, AppError> {
    let mut effective = config.clone();
    effective.format = format;
    let result = run_export_selection(
        &effective,
        &[input],
        false,
        true,
        false,
        Some(cancellation.clone()),
    );
    if cancellation.is_cancelled() {
        Err(AppError::operation(
            "export_cancelled",
            "PlantUML export was cancelled",
        ))
    } else {
        result
    }
}

fn run_export_selection(
    config: &ResolvedConfig,
    requested_inputs: &[PathBuf],
    workspace: bool,
    require_input: bool,
    keep_going: bool,
    cancellation: Option<ExportCancellation>,
) -> Result<ExportReport, AppError> {
    if cancellation
        .as_ref()
        .is_some_and(ExportCancellation::is_cancelled)
    {
        return Err(AppError::operation(
            "export_cancelled",
            "PlantUML export was cancelled",
        ));
    }
    let inputs = discover(config, requested_inputs, workspace, require_input)?;
    if inputs.is_empty() {
        return Ok(ExportReport::default());
    }

    let include_paths = resolve_include_paths(config)?;
    let renderer = configured_renderer(config)?;
    prepare_renderer(config, &renderer)?;
    let graphviz_path = if config.layout == CliLayout::Graphviz {
        resolve_executable_path(&config.graphviz_path, "Graphviz")?
    } else {
        config.graphviz_path.clone()
    };
    let health = check_health(
        &renderer,
        renderer_format(config.format),
        renderer_layout(config.layout),
        &graphviz_path,
        &StdCommandExecutor,
    );
    require_ready(&health)?;

    let native = NativeRenderer {
        renderer: renderer.clone(),
        root: config.root.clone(),
        include_paths,
        security: resolve_render_security(config),
        remote_includes: config.remote_includes,
        offline: config.offline,
        layout: renderer_layout(config.layout),
        graphviz_path: graphviz_path.clone(),
        embed_source_metadata: config.embed_source_metadata,
        metadata: renderer_metadata(&renderer),
        executor: StdCommandExecutor,
        cancellation,
    };
    let environment = environment_metadata(config, &renderer, &graphviz_path);
    let state_dir = workspace_state_dir(&config.root, &config.out_dir)?;
    ExportSession::new(native)
        .run(ExportRequest {
            root: config.root.clone(),
            out_dir: config.out_dir.clone(),
            state_dir,
            inputs,
            format: config.format,
            keep_going,
            tool_version: env!("CARGO_PKG_VERSION").to_string(),
            environment,
        })
        .map_err(map_export_error)
}

pub fn run_check(config: &ResolvedConfig, args: &CheckArgs) -> Result<CheckReport, AppError> {
    let inputs = discover(config, &args.inputs, args.workspace, args.require_input)?;
    if inputs.is_empty() {
        return Ok(CheckReport::default());
    }

    let include_paths = resolve_include_paths(config)?;
    let renderer = prepare_syntax_renderer(config)?;

    let mut report = CheckReport::default();
    for input in inputs {
        let identity = input_identity(&input)?;
        let input_path = input.absolute_path.clone();
        let syntax_output = create_syntax_output_dir().map_err(|error| {
            AppError::environment("syntax_scratch_unavailable", error.to_string())
        })?;
        let command = build_syntax_command(
            &renderer,
            &SyntaxRequest {
                input: input.absolute_path,
                output_dir: syntax_output.path().to_path_buf(),
                worktree_root: config.root.clone(),
                include_paths: include_paths.clone(),
                security: resolve_render_security(config),
            },
        )
        .map_err(|error| AppError::environment("syntax_command_invalid", error.to_string()))?;
        let process_result = StdCommandExecutor.execute(&command, CHECK_TIMEOUT);
        let scratch_path = syntax_output.path().to_path_buf();
        syntax_output.close().map_err(|error| {
            AppError::environment(
                "syntax_scratch_cleanup",
                format!(
                    "could not clean isolated syntax output directory {}: {error}",
                    scratch_path.display()
                ),
            )
        })?;
        match process_result {
            Ok(output) => {
                let diagnostics = diagnostics_from_output(&output);
                if let Some(hint) = remote_policy_hint_from_diagnostics(
                    &input_path,
                    &diagnostics,
                    &resolve_render_security(config),
                    config.remote_includes,
                    config.offline,
                ) {
                    return Err(AppError::operation("remote_include_blocked", hint));
                }
                if diagnostics.is_empty() {
                    report.checked.push(identity);
                } else {
                    report.failures.push(check_failure(
                        identity,
                        "syntax_error",
                        "PlantUML reported a syntax error",
                        diagnostics,
                    ));
                }
            }
            Err(ProcessFailure::NonZero { stdout, stderr, .. }) => {
                let output = ProcessOutput { stdout, stderr };
                let diagnostics = diagnostics_from_output(&output);
                if let Some(hint) = remote_policy_hint_from_diagnostics(
                    &input_path,
                    &diagnostics,
                    &resolve_render_security(config),
                    config.remote_includes,
                    config.offline,
                ) {
                    return Err(AppError::operation("remote_include_blocked", hint));
                }
                if diagnostics.is_empty() {
                    let detail = process_output_detail(&output);
                    return Err(AppError::environment(
                        "syntax_check_environment",
                        if detail.is_empty() {
                            "PlantUML syntax validation failed without a source diagnostic"
                                .to_string()
                        } else {
                            detail
                        },
                    ));
                }
                report.failures.push(check_failure(
                    identity,
                    "syntax_error",
                    "PlantUML reported a syntax error",
                    diagnostics,
                ));
            }
            Err(ProcessFailure::Timeout { .. }) => {
                return Err(AppError::environment(
                    "syntax_check_timeout",
                    format!(
                        "PlantUML syntax validation timed out after {} seconds while checking {identity}",
                        CHECK_TIMEOUT.as_secs()
                    ),
                ));
            }
            Err(error) => {
                return Err(AppError::environment(
                    "syntax_check_environment",
                    error.to_string(),
                ));
            }
        }
    }
    Ok(report)
}

fn discover(
    config: &ResolvedConfig,
    inputs: &[PathBuf],
    workspace: bool,
    require_input: bool,
) -> Result<Vec<DiscoveredInput>, AppError> {
    let mut options = DiscoveryOptions::new(config.root.clone());
    options.inputs = inputs.to_vec();
    options.workspace = workspace;
    options.out_dir = config.out_dir.clone();
    options.include = config.include.clone();
    options.exclude = config.exclude.clone();
    options.require_input = require_input;
    discover_inputs(&options).map_err(map_discovery_error)
}

fn map_discovery_error(error: DiscoveryError) -> AppError {
    match error.kind {
        DiscoveryErrorKind::InvalidPattern
        | DiscoveryErrorKind::InvalidRoot
        | DiscoveryErrorKind::UnsafePath => AppError::usage("input_discovery", error.to_string()),
        DiscoveryErrorKind::NoInputs => AppError::operation("no_inputs", error.to_string()),
        DiscoveryErrorKind::InvalidInput | DiscoveryErrorKind::UnsupportedInput => {
            AppError::operation("invalid_input", error.to_string())
        }
        DiscoveryErrorKind::Walk => AppError::operation("input_discovery", error.to_string()),
    }
}

fn prepare_renderer(config: &ResolvedConfig, renderer: &Renderer) -> Result<(), AppError> {
    if let Renderer::Managed { cache_dir, .. } = renderer {
        ensure_managed_java(cache_dir, config.offline).map_err(|error| {
            AppError::environment("managed_java_unavailable", error.to_string())
        })?;
        ensure_managed_jar(cache_dir, config.offline).map_err(|error| {
            AppError::environment("managed_renderer_unavailable", error.to_string())
        })?;
    }
    Ok(())
}

fn prepare_syntax_renderer(config: &ResolvedConfig) -> Result<Renderer, AppError> {
    let renderer = configured_renderer(config)?;
    prepare_renderer(config, &renderer)?;
    let health = check_health(
        &renderer,
        Format::Svg,
        Layout::Smetana,
        &config.graphviz_path,
        &StdCommandExecutor,
    );
    require_ready(&health)?;
    Ok(renderer)
}

fn require_ready(health: &HealthReport) -> Result<(), AppError> {
    if health.ready {
        return Ok(());
    }
    Err(AppError::environment(
        "renderer_unhealthy",
        format!(
            "renderer prerequisites are not ready (renderer: {}; Java: {}; Graphviz: {})",
            health.renderer.detail, health.java.detail, health.graphviz.detail
        ),
    ))
}

fn health_data(
    config: &ResolvedConfig,
    renderer: &Renderer,
    format: Format,
    layout: Layout,
) -> HealthData {
    let report = check_health(
        renderer,
        format,
        layout,
        &config.graphviz_path,
        &StdCommandExecutor,
    );
    HealthData {
        ready: report.ready,
        renderer_mode: config.renderer,
        format: config.format,
        layout: config.layout,
        renderer: component_health(report.renderer.status, report.renderer.detail),
        java: component_health(report.java.status, report.java.detail),
        graphviz: component_health(report.graphviz.status, report.graphviz.detail),
    }
}

fn component_health(status: HealthStatus, detail: String) -> ComponentHealthData {
    ComponentHealthData {
        status: match status {
            HealthStatus::Ready => "ready",
            HealthStatus::Missing => "missing",
            HealthStatus::Incompatible => "incompatible",
            HealthStatus::NotRequired => "not_required",
        },
        detail,
    }
}

fn renderer_format(format: OutputFormat) -> Format {
    match format {
        OutputFormat::Svg => Format::Svg,
        OutputFormat::Png => Format::Png,
        OutputFormat::Pdf => Format::Pdf,
    }
}

fn renderer_layout(layout: CliLayout) -> Layout {
    match layout {
        CliLayout::Graphviz => Layout::Graphviz,
        CliLayout::Smetana => Layout::Smetana,
    }
}

fn renderer_metadata(renderer: &Renderer) -> RendererMetadata {
    let (mode, version) = match renderer {
        Renderer::Managed { .. } => ("managed", MANAGED_PLANTUML_VERSION.to_string()),
        Renderer::Binary { executable } => (
            "binary",
            probe(CommandSpec {
                program: executable.clone(),
                args: vec![OsString::from("--version")],
                env: Vec::new(),
                env_remove: Vec::new(),
            })
            .unwrap_or_else(|| "external".to_string()),
        ),
        Renderer::Jar { java, jar } => (
            "jar",
            probe(CommandSpec {
                program: java.clone(),
                args: vec![
                    OsString::from("-jar"),
                    jar.as_os_str().to_os_string(),
                    OsString::from("--version"),
                ],
                env: Vec::new(),
                env_remove: Vec::new(),
            })
            .unwrap_or_else(|| "external".to_string()),
        ),
    };
    RendererMetadata {
        mode: mode.to_string(),
        version,
    }
}

fn environment_metadata(
    config: &ResolvedConfig,
    renderer: &Renderer,
    graphviz_path: &Path,
) -> EnvironmentMetadata {
    let java_version = match renderer {
        Renderer::Managed { java, .. } | Renderer::Jar { java, .. } => probe(CommandSpec {
            program: java.clone(),
            args: vec![OsString::from("-version")],
            env: Vec::new(),
            env_remove: Vec::new(),
        }),
        Renderer::Binary { .. } => None,
    };
    let graphviz_version = (config.layout == CliLayout::Graphviz)
        .then(|| {
            probe(CommandSpec {
                program: graphviz_path.to_path_buf(),
                args: vec![OsString::from("-V")],
                env: Vec::new(),
                env_remove: Vec::new(),
            })
        })
        .flatten();
    EnvironmentMetadata {
        java_version,
        graphviz_version,
        os: env::consts::OS.to_string(),
        architecture: env::consts::ARCH.to_string(),
    }
}

fn resolve_executable_path(program: &Path, label: &str) -> Result<PathBuf, AppError> {
    let candidates = if program.is_absolute() || program.components().count() > 1 {
        let candidate = if program.is_absolute() {
            program.to_path_buf()
        } else {
            env::current_dir()
                .map_err(|error| {
                    AppError::environment(
                        "current_directory",
                        format!("failed to resolve {label} executable: {error}"),
                    )
                })?
                .join(program)
        };
        vec![candidate]
    } else {
        executable_search_candidates(program)
    };

    for candidate in candidates {
        if is_executable_file(&candidate) {
            return fs::canonicalize(&candidate).map_err(|error| {
                AppError::environment(
                    "executable_path_invalid",
                    format!(
                        "failed to resolve {label} executable {}: {error}",
                        candidate.display()
                    ),
                )
            });
        }
    }

    Err(AppError::environment(
        "executable_not_found",
        format!(
            "{label} executable {} was not found or is not executable",
            program.display()
        ),
    ))
}

fn executable_search_candidates(program: &Path) -> Vec<PathBuf> {
    let Some(search_path) = env::var_os("PATH") else {
        return Vec::new();
    };
    env::split_paths(&search_path)
        .flat_map(|directory| {
            executable_names(program)
                .into_iter()
                .map(move |name| directory.join(name))
        })
        .collect()
}

#[cfg(not(target_os = "windows"))]
fn executable_names(program: &Path) -> Vec<PathBuf> {
    vec![program.to_path_buf()]
}

#[cfg(target_os = "windows")]
fn executable_names(program: &Path) -> Vec<PathBuf> {
    if program.extension().is_some() {
        return vec![program.to_path_buf()];
    }
    let extensions =
        env::var_os("PATHEXT").unwrap_or_else(|| OsString::from(".COM;.EXE;.BAT;.CMD"));
    extensions
        .to_string_lossy()
        .split(';')
        .filter(|extension| !extension.is_empty())
        .map(|extension| {
            let mut name = program.as_os_str().to_os_string();
            name.push(extension);
            PathBuf::from(name)
        })
        .collect()
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

fn probe(command: CommandSpec) -> Option<String> {
    StdCommandExecutor
        .execute(&command, PROBE_TIMEOUT)
        .ok()
        .and_then(|output| {
            let detail = process_output_detail(&output);
            (!detail.is_empty()).then_some(detail)
        })
}

struct NativeRenderer {
    renderer: Renderer,
    root: PathBuf,
    include_paths: Vec<PathBuf>,
    security: RenderSecurity,
    remote_includes: RemoteIncludes,
    offline: bool,
    layout: Layout,
    graphviz_path: PathBuf,
    embed_source_metadata: bool,
    metadata: RendererMetadata,
    executor: StdCommandExecutor,
    cancellation: Option<ExportCancellation>,
}

impl ExportRenderer for NativeRenderer {
    fn render(
        &self,
        input: &Path,
        staging_dir: &Path,
        format: OutputFormat,
    ) -> Result<(), ExportRendererError> {
        let command = build_render_command(
            &self.renderer,
            &RenderRequest {
                input: input.to_path_buf(),
                output_dir: staging_dir.to_path_buf(),
                worktree_root: self.root.clone(),
                include_paths: self.include_paths.clone(),
                security: self.security.clone(),
                format: renderer_format(format),
                layout: self.layout,
                graphviz_path: self.graphviz_path.clone(),
                embed_source_metadata: self.embed_source_metadata,
            },
        )
        .map_err(|error| {
            ExportRendererError::environment("renderer_configuration", error.to_string())
        })?;
        match &self.cancellation {
            Some(cancellation) => {
                execute_cancellable(&command, RENDER_TIMEOUT, || cancellation.is_cancelled())
                    .map(|_| ())
                    .map_err(|error| match error {
                        ControlledProcessFailure::Cancelled => ExportRendererError::environment(
                            "export_cancelled",
                            "PlantUML export was cancelled",
                        ),
                        ControlledProcessFailure::Process(error) => {
                            map_render_process_error_for_input(
                                error,
                                input,
                                &self.security,
                                self.remote_includes,
                                self.offline,
                            )
                        }
                    })
            }
            None => self
                .executor
                .execute(&command, RENDER_TIMEOUT)
                .map(|_| ())
                .map_err(|error| {
                    map_render_process_error_for_input(
                        error,
                        input,
                        &self.security,
                        self.remote_includes,
                        self.offline,
                    )
                }),
        }
    }

    fn validate(&self, output: &Path, format: OutputFormat) -> Result<(), ExportRendererError> {
        if self
            .cancellation
            .as_ref()
            .is_some_and(ExportCancellation::is_cancelled)
        {
            return Err(ExportRendererError::environment(
                "export_cancelled",
                "PlantUML export was cancelled",
            ));
        }
        renderer::validate_output(output, renderer_format(format))
            .map_err(|error| ExportRendererError::new("output_type", error.to_string()))
    }

    fn metadata(&self) -> RendererMetadata {
        self.metadata.clone()
    }
}

fn map_render_process_error_for_input(
    error: ProcessFailure,
    input: &Path,
    security: &RenderSecurity,
    remote_includes: RemoteIncludes,
    offline: bool,
) -> ExportRendererError {
    if let ProcessFailure::NonZero { stdout, stderr, .. } = &error {
        let output = ProcessOutput {
            stdout: stdout.clone(),
            stderr: stderr.clone(),
        };
        let diagnostics = diagnostics_from_output(&output);
        if let Some(hint) = remote_policy_hint_from_diagnostics(
            input,
            &diagnostics,
            security,
            remote_includes,
            offline,
        ) {
            return ExportRendererError::new("remote_include_blocked", hint);
        }
    }
    map_render_process_error(error)
}

fn map_render_process_error(error: ProcessFailure) -> ExportRendererError {
    match error {
        ProcessFailure::NonZero { stdout, stderr, .. } => {
            let output = ProcessOutput { stdout, stderr };
            let detail = process_output_detail(&output);
            ExportRendererError::new(
                "render_failed",
                if detail.is_empty() {
                    "PlantUML renderer exited unsuccessfully".to_string()
                } else {
                    detail
                },
            )
        }
        ProcessFailure::Timeout { .. } => ExportRendererError::environment(
            "render_timeout",
            "PlantUML rendering timed out after 120 seconds",
        ),
        other => ExportRendererError::environment("renderer_unavailable", other.to_string()),
    }
}

fn map_export_error(error: ExportError) -> AppError {
    if error.kind == ExportErrorKind::Environment {
        return AppError::environment("renderer_environment", error.message);
    }
    let code = match error.kind {
        ExportErrorKind::Environment => unreachable!("handled above"),
        ExportErrorKind::InputFailure => "input_failure",
        ExportErrorKind::OutputValidation => "output_validation",
        ExportErrorKind::OwnershipConflict => "ownership_conflict",
        ExportErrorKind::UnsafePath => "unsafe_path",
        ExportErrorKind::InvalidManifest => "invalid_manifest",
        ExportErrorKind::Io => "export_io",
    };
    AppError::operation(code, error.message)
}

fn input_identity(input: &DiscoveredInput) -> Result<String, AppError> {
    normalize_relative_path(&input.relative_path)
        .map_err(|error| AppError::operation("invalid_input_identity", error.to_string()))
}

fn diagnostics_from_output(output: &ProcessOutput) -> Vec<CheckDiagnostic> {
    let combined = combined_process_output(output);
    parse_standard_report(&combined)
        .into_iter()
        .map(|diagnostic| CheckDiagnostic {
            line: diagnostic.range.start.line + 1,
            message: diagnostic.message,
        })
        .collect()
}

fn check_failure(
    input: String,
    code: &str,
    message: &str,
    diagnostics: Vec<CheckDiagnostic>,
) -> CheckFailure {
    CheckFailure {
        input,
        code: code.to_string(),
        message: message.to_string(),
        diagnostics,
    }
}

fn combined_process_output(output: &ProcessOutput) -> String {
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    if !combined.is_empty() && !output.stderr.is_empty() {
        combined.push('\n');
    }
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    combined
}

fn process_output_detail(output: &ProcessOutput) -> String {
    let combined = combined_process_output(output);
    let detail = combined.trim();
    if detail.len() <= MAX_ERROR_DETAIL_BYTES {
        return detail.to_string();
    }
    let mut boundary = MAX_ERROR_DETAIL_BYTES;
    while !detail.is_char_boundary(boundary) {
        boundary -= 1;
    }
    format!("{}…", &detail[..boundary])
}

pub(crate) fn remote_include_policy_hint_at_line(
    input: &Path,
    line: usize,
    security: &RenderSecurity,
    remote_includes: RemoteIncludes,
    offline: bool,
) -> Option<String> {
    let source = fs::read_to_string(input).ok()?;
    remote_include_references(&source)
        .into_iter()
        .find(|(candidate_line, _)| *candidate_line == line)
        .and_then(|(_, url)| {
            remote_include_policy_hint_for_url(line, &url, security, remote_includes, offline)
        })
}

fn remote_policy_hint_from_diagnostics(
    input: &Path,
    diagnostics: &[CheckDiagnostic],
    security: &RenderSecurity,
    remote_includes: RemoteIncludes,
    offline: bool,
) -> Option<String> {
    diagnostics
        .iter()
        .filter(|diagnostic| is_remote_access_failure(&diagnostic.message))
        .find_map(|diagnostic| {
            remote_include_policy_hint_at_line(
                input,
                diagnostic.line as usize,
                security,
                remote_includes,
                offline,
            )
        })
}

pub(crate) fn is_remote_access_failure(message: &str) -> bool {
    message.to_ascii_lowercase().contains("cannot open url")
}

fn remote_include_policy_hint_for_url(
    line: usize,
    url: &Url,
    security: &RenderSecurity,
    remote_includes: RemoteIncludes,
    offline: bool,
) -> Option<String> {
    let origin = format!("{}/", url.origin().ascii_serialization());
    if security.allowed_remote_urls.contains(&origin) {
        return None;
    }

    let location = format!(
        "remote include from {} at line {line}",
        origin.trim_end_matches('/')
    );
    if offline {
        return Some(format!(
            "{location} is blocked because offline=true disables all remote includes; remove offline=true from the active trusted configuration only when network access is intended"
        ));
    }
    if remote_includes == RemoteIncludes::Disabled {
        return Some(format!(
            "{location} is blocked because remoteIncludes is disabled; change the trusted user policy if remote access is intended (project and Zed settings cannot grant network access)"
        ));
    }
    if security.profile == SecurityProfile::Allowlist {
        return Some(format!(
            "{location} is not covered by allowedRemoteUrls; add that trusted HTTP(S) origin ending in '/' to allowedRemoteUrls in the user config (this global grant covers the whole origin in every worktree, and project/Zed settings cannot add it)"
        ));
    }
    if internet_profile_blocks(url) {
        return Some(format!(
            "{location} is blocked by PlantUML's INTERNET safety policy; for a trusted private, raw-address, authenticated, or non-standard-port endpoint, add that HTTP(S) origin ending in '/' to allowedRemoteUrls in the user config (this global grant covers the whole origin in every worktree)"
        ));
    }
    None
}

fn remote_include_references(source: &str) -> Vec<(usize, Url)> {
    const DIRECTIVES: [&str; 5] = [
        "!includeurl",
        "!include_once",
        "!include_many",
        "!include",
        "!import",
    ];

    let mut in_block_comment = false;
    source
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let trimmed = line.trim_start();
            if in_block_comment {
                if trimmed.contains("'/") {
                    in_block_comment = false;
                }
                return None;
            }
            if let Some(comment) = trimmed.strip_prefix("/'") {
                in_block_comment = !comment.contains("'/");
                return None;
            }
            if trimmed.starts_with('\'') {
                return None;
            }
            let arguments = DIRECTIVES.iter().find_map(|directive| {
                let arguments = trimmed.strip_prefix(directive)?;
                arguments
                    .chars()
                    .next()
                    .is_some_and(char::is_whitespace)
                    .then_some(arguments)
            })?;
            let start = [arguments.find("https://"), arguments.find("http://")]
                .into_iter()
                .flatten()
                .min()?;
            let candidate = &arguments[start..];
            let end = candidate
                .find(|character: char| {
                    character.is_whitespace()
                        || matches!(character, '>' | '"' | '\'' | ')' | ']' | '}')
                })
                .unwrap_or(candidate.len());
            let candidate = candidate[..end].trim_end_matches([',', ';']);
            Url::parse(candidate).ok().map(|url| (index + 1, url))
        })
        .collect()
}

fn internet_profile_blocks(url: &Url) -> bool {
    if !url.username().is_empty() || url.password().is_some() {
        return true;
    }
    if matches!(url.host(), Some(Host::Ipv4(_)) | Some(Host::Ipv6(_))) {
        return true;
    }
    if url
        .host_str()
        .is_some_and(|host| host.eq_ignore_ascii_case("localhost") || host.ends_with(".localhost"))
    {
        return true;
    }
    !matches!(
        (url.scheme(), url.port()),
        ("http", Some(80)) | ("https", Some(443)) | (_, None)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn config(root: PathBuf) -> ResolvedConfig {
        ResolvedConfig {
            root,
            project_config: None,
            user_config: None,
            renderer: RendererMode::Managed,
            format: OutputFormat::Svg,
            out_dir: PathBuf::from("out"),
            layout: CliLayout::Smetana,
            embed_source_metadata: false,
            include_paths: Vec::new(),
            include: Vec::new(),
            exclude: Vec::new(),
            offline: false,
            remote_includes: RemoteIncludes::Public,
            allowed_remote_urls: Vec::new(),
            java_path: PathBuf::from("java"),
            binary_path: None,
            jar_path: None,
            graphviz_path: PathBuf::from("dot"),
        }
    }

    #[test]
    fn remote_policy_and_offline_mode_map_to_fail_closed_renderer_security() {
        let temp = tempfile::tempdir().unwrap();
        let mut resolved = config(temp.path().to_path_buf());
        resolved.allowed_remote_urls = vec!["http://plantuml.internal:8080/common/".into()];

        let public = resolve_render_security(&resolved);
        assert_eq!(public.profile, SecurityProfile::Internet);
        assert_eq!(public.allowed_remote_urls, resolved.allowed_remote_urls);

        resolved.remote_includes = RemoteIncludes::Allowlist;
        let allowlist = resolve_render_security(&resolved);
        assert_eq!(allowlist.profile, SecurityProfile::Allowlist);
        assert_eq!(allowlist.allowed_remote_urls, resolved.allowed_remote_urls);

        resolved.remote_includes = RemoteIncludes::Disabled;
        let disabled = resolve_render_security(&resolved);
        assert_eq!(disabled.profile, SecurityProfile::Allowlist);
        assert!(disabled.allowed_remote_urls.is_empty());

        resolved.remote_includes = RemoteIncludes::Public;
        resolved.offline = true;
        let offline = resolve_render_security(&resolved);
        assert_eq!(offline.profile, SecurityProfile::Allowlist);
        assert!(offline.allowed_remote_urls.is_empty());
    }

    #[test]
    fn blocked_remote_includes_get_actionable_policy_diagnostics() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("model.puml");
        fs::write(
            &input,
            "@startuml\n!include http://127.0.0.1:8080/common/theme.puml?token=secret\n@enduml\n",
        )
        .unwrap();

        let public = remote_include_policy_hint_at_line(
            &input,
            2,
            &RenderSecurity {
                profile: SecurityProfile::Internet,
                allowed_remote_urls: Vec::new(),
            },
            RemoteIncludes::Public,
            false,
        )
        .unwrap();
        assert!(public.contains("127.0.0.1:8080"));
        assert!(public.contains("allowedRemoteUrls"));
        assert!(public.contains("user config"));
        assert!(!public.contains("token"));
        assert!(!public.contains("secret"));

        let offline = remote_include_policy_hint_at_line(
            &input,
            2,
            &RenderSecurity {
                profile: SecurityProfile::Allowlist,
                allowed_remote_urls: Vec::new(),
            },
            RemoteIncludes::Public,
            true,
        )
        .unwrap();
        assert!(offline.contains("offline=true"));

        let allowlisted = remote_include_policy_hint_at_line(
            &input,
            2,
            &RenderSecurity {
                profile: SecurityProfile::Allowlist,
                allowed_remote_urls: vec!["http://127.0.0.1:8080/".into()],
            },
            RemoteIncludes::Allowlist,
            false,
        );
        assert_eq!(allowlisted, None);
    }

    #[test]
    fn ordinary_public_urls_are_not_misreported_as_policy_blocks() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("model.puml");
        fs::write(
            &input,
            "@startuml\n!include https://example.com/public/theme.puml\n@enduml\n",
        )
        .unwrap();

        assert_eq!(
            remote_include_policy_hint_at_line(
                &input,
                2,
                &RenderSecurity {
                    profile: SecurityProfile::Internet,
                    allowed_remote_urls: Vec::new(),
                },
                RemoteIncludes::Public,
                false,
            ),
            None
        );
    }

    #[test]
    fn renderer_diagnostics_cannot_hide_a_remote_policy_block() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("model.puml");
        fs::write(
            &input,
            "@startuml\n!include http://127.0.0.1:8080/theme.puml\n@enduml\n",
        )
        .unwrap();
        let error = map_render_process_error_for_input(
            ProcessFailure::NonZero {
                program: PathBuf::from("java"),
                code: Some(200),
                stdout: b"protocolVersion=1\nstatus=ERROR\nlineNumber=2\nlabel=Cannot open URL\n"
                    .to_vec(),
                stderr: Vec::new(),
            },
            &input,
            &RenderSecurity {
                profile: SecurityProfile::Internet,
                allowed_remote_urls: Vec::new(),
            },
            RemoteIncludes::Public,
            false,
        );

        assert_eq!(error.code, "remote_include_blocked");
        assert!(error.message.contains("allowedRemoteUrls"));
    }

    #[test]
    fn ordinary_syntax_errors_are_not_reclassified_by_a_later_remote_include() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("model.puml");
        fs::write(
            &input,
            "@startuml\nAlice -x\n!include http://127.0.0.1:8080/theme.puml\n@enduml\n",
        )
        .unwrap();
        let error = map_render_process_error_for_input(
            ProcessFailure::NonZero {
                program: PathBuf::from("java"),
                code: Some(200),
                stdout: b"protocolVersion=1\nstatus=ERROR\nlineNumber=2\nlabel=Syntax Error\n"
                    .to_vec(),
                stderr: Vec::new(),
            },
            &input,
            &RenderSecurity {
                profile: SecurityProfile::Internet,
                allowed_remote_urls: Vec::new(),
            },
            RemoteIncludes::Public,
            false,
        );

        assert_eq!(error.code, "render_failed");
    }

    #[test]
    fn policy_diagnostics_select_the_reported_include_not_the_first_url() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("model.puml");
        fs::write(
            &input,
            "@startuml\n!include https://example.com/theme.puml\n!include http://127.0.0.1:8080/theme.puml\n@enduml\n",
        )
        .unwrap();
        let error = map_render_process_error_for_input(
            ProcessFailure::NonZero {
                program: PathBuf::from("java"),
                code: Some(200),
                stdout: b"protocolVersion=1\nstatus=ERROR\nlineNumber=3\nlabel=Cannot open URL\n"
                    .to_vec(),
                stderr: Vec::new(),
            },
            &input,
            &RenderSecurity {
                profile: SecurityProfile::Internet,
                allowed_remote_urls: Vec::new(),
            },
            RemoteIncludes::Public,
            false,
        );

        assert_eq!(error.code, "remote_include_blocked");
        assert!(error.message.contains("line 3"));
        assert!(error.message.contains("127.0.0.1:8080"));
    }

    #[test]
    fn include_like_identifiers_are_not_treated_as_remote_include_directives() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("model.puml");
        fs::write(
            &input,
            "@startuml\n!includeFoo http://127.0.0.1:8080/theme.puml\n@enduml\n",
        )
        .unwrap();
        let error = map_render_process_error_for_input(
            ProcessFailure::NonZero {
                program: PathBuf::from("java"),
                code: Some(200),
                stdout: b"protocolVersion=1\nstatus=ERROR\nlineNumber=2\nlabel=Cannot open URL\n"
                    .to_vec(),
                stderr: Vec::new(),
            },
            &input,
            &RenderSecurity {
                profile: SecurityProfile::Internet,
                allowed_remote_urls: Vec::new(),
            },
            RemoteIncludes::Public,
            false,
        );

        assert_eq!(error.code, "render_failed");
    }

    #[test]
    fn renderer_mapping_never_falls_back() {
        let temp = tempfile::tempdir().unwrap();
        let mut resolved = config(temp.path().to_path_buf());
        resolved.renderer = RendererMode::Binary;
        let error = renderer_from_config(&resolved, PathBuf::from("cache")).unwrap_err();
        assert_eq!(error.code, "binary_path_required");

        resolved.binary_path = Some(PathBuf::from("chosen-plantuml"));
        assert_eq!(
            renderer_from_config(&resolved, PathBuf::from("cache")).unwrap(),
            Renderer::Binary {
                executable: PathBuf::from("chosen-plantuml")
            }
        );
    }

    #[test]
    fn managed_owns_java_while_explicit_jar_uses_the_configured_java() {
        let temp = tempfile::tempdir().unwrap();
        let cache = temp.path().join("cache");
        let mut resolved = config(temp.path().to_path_buf());
        resolved.java_path = PathBuf::from("user-java");
        let asset = current_managed_java_asset().unwrap();

        assert_eq!(
            renderer_from_config(&resolved, cache.clone()).unwrap(),
            Renderer::Managed {
                java: managed_java_path(&cache, asset),
                cache_dir: cache,
            }
        );

        resolved.renderer = RendererMode::Jar;
        resolved.jar_path = Some(PathBuf::from("chosen.jar"));
        assert_eq!(
            renderer_from_config(&resolved, PathBuf::new()).unwrap(),
            Renderer::Jar {
                java: PathBuf::from("user-java"),
                jar: PathBuf::from("chosen.jar"),
            }
        );
    }

    #[test]
    fn include_paths_are_canonical_and_relative_paths_cannot_escape_the_root() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let inside = root.join("includes");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&inside).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let mut resolved = config(root.clone());
        resolved.include_paths = vec![PathBuf::from("includes")];
        assert_eq!(
            resolve_include_paths(&resolved).unwrap(),
            vec![inside.canonicalize().unwrap()]
        );

        resolved.include_paths = vec![outside.clone()];
        assert_eq!(
            resolve_include_paths(&resolved).unwrap(),
            vec![outside.canonicalize().unwrap()]
        );

        resolved.include_paths = vec![PathBuf::from("../outside")];
        let error = resolve_include_paths(&resolved).unwrap_err();
        assert_eq!(error.code, "include_path_outside_root");
    }

    #[test]
    fn renderer_process_failures_preserve_operation_vs_environment_semantics() {
        let source_failure = map_render_process_error(ProcessFailure::NonZero {
            program: PathBuf::from("plantuml"),
            code: Some(200),
            stdout: Vec::new(),
            stderr: b"syntax error".to_vec(),
        });
        assert_eq!(
            source_failure.kind,
            crate::export::RendererFailureKind::Operation
        );

        for process_failure in [
            ProcessFailure::Timeout {
                program: PathBuf::from("plantuml"),
                timeout: RENDER_TIMEOUT,
            },
            ProcessFailure::Spawn {
                program: PathBuf::from("plantuml"),
                message: "missing".to_string(),
            },
        ] {
            assert_eq!(
                map_render_process_error(process_failure).kind,
                crate::export::RendererFailureKind::Environment
            );
        }
    }
}
