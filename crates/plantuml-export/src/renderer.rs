use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};

pub use crate::process_control::{CommandSpec, ProcessFailure, ProcessOutput};

pub const MANAGED_PLANTUML_VERSION: &str = "1.2026.6";
pub const MANAGED_PLANTUML_URL: &str =
    "https://github.com/plantuml/plantuml/releases/download/v1.2026.6/plantuml.jar";
pub const MANAGED_PLANTUML_SHA256: &str =
    "89948f14c93756c7a3fb7b69078ff37e8489fd79dd430c582b931e2f65358690";
pub const MAX_MANAGED_JAR_BYTES: u64 = 64 * 1024 * 1024;
pub const MANAGED_JAVA_VERSION: &str = "21.0.11+10";
pub const MAX_MANAGED_JAVA_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_MANAGED_JAVA_EXPANDED_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_MANAGED_JAVA_ARCHIVE_ENTRIES: usize = 4096;

const MANAGED_JAVA_MARKER: &str = ".plantuml-export-managed-jre";

const HEALTH_TIMEOUT: Duration = Duration::from_secs(5);
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManagedArchiveFormat {
    TarGz,
    Zip,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ManagedJavaAsset {
    pub url: &'static str,
    pub sha256: &'static str,
    pub archive_format: ManagedArchiveFormat,
    pub java_relative_path: &'static str,
}

pub fn managed_java_asset(os: &str, architecture: &str) -> Option<ManagedJavaAsset> {
    let asset = match (os, architecture) {
        ("macos", "aarch64") => ManagedJavaAsset {
            url: "https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.11%2B10/OpenJDK21U-jre_aarch64_mac_hotspot_21.0.11_10.tar.gz",
            sha256: "4b7a8cd23102c251c8b8be42a9a5f1263fb337cf1037f6f64b25f3070efe4b76",
            archive_format: ManagedArchiveFormat::TarGz,
            java_relative_path: "Contents/Home/bin/java",
        },
        ("macos", "x86_64") => ManagedJavaAsset {
            url: "https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.11%2B10/OpenJDK21U-jre_x64_mac_hotspot_21.0.11_10.tar.gz",
            sha256: "b341fb8ed5b70d49066b98176bc98e30f55082192403deb60e0cd5948b6e7923",
            archive_format: ManagedArchiveFormat::TarGz,
            java_relative_path: "Contents/Home/bin/java",
        },
        ("linux", "aarch64") => ManagedJavaAsset {
            url: "https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.11%2B10/OpenJDK21U-jre_aarch64_linux_hotspot_21.0.11_10.tar.gz",
            sha256: "fa23d9d9945053e67bcc7638410eabf1e17a7672c7c95a24f70cd08b8407d36e",
            archive_format: ManagedArchiveFormat::TarGz,
            java_relative_path: "bin/java",
        },
        ("linux", "x86_64") => ManagedJavaAsset {
            url: "https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.11%2B10/OpenJDK21U-jre_x64_linux_hotspot_21.0.11_10.tar.gz",
            sha256: "e5038aae3ca9ff670bc696496b0728dbd23d280026bad30291cb919221ecfdcb",
            archive_format: ManagedArchiveFormat::TarGz,
            java_relative_path: "bin/java",
        },
        ("windows", "aarch64") => ManagedJavaAsset {
            url: "https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.11%2B10/OpenJDK21U-jre_aarch64_windows_hotspot_21.0.11_10.zip",
            sha256: "22e2c2b83a7dc5653c938c9a49d87ad52a1faa38f7f3d80a96ceb0795ab99637",
            archive_format: ManagedArchiveFormat::Zip,
            java_relative_path: "bin/java.exe",
        },
        ("windows", "x86_64") => ManagedJavaAsset {
            url: "https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.11%2B10/OpenJDK21U-jre_x64_windows_hotspot_21.0.11_10.zip",
            sha256: "be26677aaa20b39a62edcaab4c8857a8b76673b0f45abc0b6143b142b62717e4",
            archive_format: ManagedArchiveFormat::Zip,
            java_relative_path: "bin/java.exe",
        },
        _ => return None,
    };
    Some(asset)
}

pub fn current_managed_java_asset() -> Result<ManagedJavaAsset, RendererError> {
    managed_java_asset(std::env::consts::OS, std::env::consts::ARCH).ok_or_else(|| {
        RendererError::UnsupportedPlatform {
            os: std::env::consts::OS.to_string(),
            architecture: std::env::consts::ARCH.to_string(),
        }
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Format {
    Svg,
    Png,
    Pdf,
}

impl Format {
    fn as_str(self) -> &'static str {
        match self {
            Self::Svg => "svg",
            Self::Png => "png",
            Self::Pdf => "pdf",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Layout {
    Graphviz,
    Smetana,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecurityProfile {
    Internet,
    Allowlist,
}

impl SecurityProfile {
    fn as_str(self) -> &'static str {
        match self {
            Self::Internet => "INTERNET",
            Self::Allowlist => "ALLOWLIST",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderSecurity {
    pub profile: SecurityProfile,
    pub allowed_remote_urls: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Renderer {
    Managed { java: PathBuf, cache_dir: PathBuf },
    Binary { executable: PathBuf },
    Jar { java: PathBuf, jar: PathBuf },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RenderRequest {
    pub input: PathBuf,
    pub output_dir: PathBuf,
    pub worktree_root: PathBuf,
    pub include_paths: Vec<PathBuf>,
    pub security: RenderSecurity,
    pub format: Format,
    pub layout: Layout,
    pub graphviz_path: PathBuf,
    pub embed_source_metadata: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyntaxRequest {
    pub input: PathBuf,
    pub output_dir: PathBuf,
    pub worktree_root: PathBuf,
    pub include_paths: Vec<PathBuf>,
    pub security: RenderSecurity,
}

pub fn create_syntax_output_dir() -> Result<tempfile::TempDir, RendererError> {
    let mut builder = tempfile::Builder::new();
    builder.prefix("plantuml-export-check-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        builder.permissions(fs::Permissions::from_mode(0o700));
    }
    builder.tempdir().map_err(|error| RendererError::Io {
        path: std::env::temp_dir(),
        message: format!("could not create isolated syntax output directory: {error}"),
    })
}

pub fn build_render_command(
    renderer: &Renderer,
    request: &RenderRequest,
) -> Result<CommandSpec, RendererError> {
    let mut command = build_command(
        renderer,
        &request.worktree_root,
        &request.input,
        &request.include_paths,
        &request.security,
        render_args(request),
    )?;
    if request.layout == Layout::Graphviz {
        if !request.graphviz_path.is_absolute() {
            return Err(RendererError::InvalidGraphvizPath {
                path: request.graphviz_path.clone(),
            });
        }
        command.env.push((
            OsString::from("GRAPHVIZ_DOT"),
            request.graphviz_path.as_os_str().to_os_string(),
        ));
    }
    Ok(command)
}

pub fn build_syntax_command(
    renderer: &Renderer,
    request: &SyntaxRequest,
) -> Result<CommandSpec, RendererError> {
    build_command(
        renderer,
        &request.worktree_root,
        &request.input,
        &request.include_paths,
        &request.security,
        vec![
            // PlantUML 1.2026.6's check-only branch returns before standard
            // reports are printed. A no-error diagnostic render preserves the
            // structured lineNumber/label report without exposing an output.
            // Source: https://github.com/plantuml/plantuml/blob/v1.2026.6/src/main/java/net/sourceforge/plantuml/Run.java#L342-L373
            OsString::from("--format"),
            OsString::from("svg"),
            OsString::from("--output-dir"),
            request.output_dir.as_os_str().to_os_string(),
            OsString::from("--stop-on-error"),
            OsString::from("--ignore-startuml-filename"),
            OsString::from("--disable-metadata"),
            OsString::from("--no-error-image"),
            OsString::from("-stdrpt:1"),
            OsString::from("-Playout=smetana"),
            request.input.as_os_str().to_os_string(),
        ],
    )
}

fn build_command(
    renderer: &Renderer,
    worktree_root: &Path,
    input: &Path,
    include_paths: &[PathBuf],
    security: &RenderSecurity,
    operation_args: Vec<OsString>,
) -> Result<CommandSpec, RendererError> {
    let allowlist = local_allowlist(worktree_root, input, include_paths);
    let allowlist =
        std::env::join_paths(&allowlist).map_err(|error| RendererError::InvalidAllowlist {
            message: error.to_string(),
        })?;
    let include_path = if include_paths.is_empty() {
        None
    } else {
        Some(std::env::join_paths(include_paths).map_err(|error| {
            RendererError::InvalidAllowlist {
                message: error.to_string(),
            }
        })?)
    };
    let remote_allowlist = security.allowed_remote_urls.join(";");
    let env_remove = [
        "PLANTUML_SECURITY_PROFILE",
        "plantuml.security.profile",
        "plantuml.allowlist.path",
        "PLANTUML_ALLOWLIST_PATH",
        "plantuml.include.path",
        "PLANTUML_INCLUDE_PATH",
        "plantuml.allowlist.url",
        "PLANTUML_ALLOWLIST_URL",
        "GRAPHVIZ_DOT",
        "JAVA_TOOL_OPTIONS",
        "JDK_JAVA_OPTIONS",
        "_JAVA_OPTIONS",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();

    match renderer {
        Renderer::Managed { java, cache_dir } => build_java_command(
            java,
            &managed_jar_path(cache_dir),
            allowlist,
            include_path,
            security,
            env_remove,
            operation_args,
        ),
        Renderer::Jar { java, jar } => build_java_command(
            java,
            jar,
            allowlist,
            include_path,
            security,
            env_remove,
            operation_args,
        ),
        Renderer::Binary { executable } => {
            let mut env = vec![
                (
                    OsString::from("PLANTUML_SECURITY_PROFILE"),
                    OsString::from(security.profile.as_str()),
                ),
                (OsString::from("PLANTUML_ALLOWLIST_PATH"), allowlist),
            ];
            if let Some(include_path) = include_path {
                env.push((OsString::from("PLANTUML_INCLUDE_PATH"), include_path));
            }
            env.push((
                OsString::from("PLANTUML_ALLOWLIST_URL"),
                OsString::from(remote_allowlist),
            ));
            Ok(CommandSpec {
                program: executable.clone(),
                args: operation_args,
                env,
                env_remove,
            })
        }
    }
}

fn build_java_command(
    java: &Path,
    jar: &Path,
    allowlist: OsString,
    include_path: Option<OsString>,
    security: &RenderSecurity,
    env_remove: Vec<OsString>,
    operation_args: Vec<OsString>,
) -> Result<CommandSpec, RendererError> {
    let mut allowlist_argument = OsString::from("-Dplantuml.allowlist.path=");
    allowlist_argument.push(allowlist);
    let mut security_argument = OsString::from("-DPLANTUML_SECURITY_PROFILE=");
    security_argument.push(security.profile.as_str());
    let mut args = vec![security_argument, allowlist_argument];
    if let Some(include_path) = include_path {
        let mut include_argument = OsString::from("-Dplantuml.include.path=");
        include_argument.push(include_path);
        args.push(include_argument);
    }
    let mut remote_argument = OsString::from("-Dplantuml.allowlist.url=");
    remote_argument.push(security.allowed_remote_urls.join(";"));
    args.push(remote_argument);
    args.push(OsString::from("-jar"));
    args.push(jar.as_os_str().to_os_string());
    args.extend(operation_args);
    Ok(CommandSpec {
        program: java.to_path_buf(),
        args,
        env: Vec::new(),
        env_remove,
    })
}

fn render_args(request: &RenderRequest) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("--format"),
        OsString::from(request.format.as_str()),
        OsString::from("--output-dir"),
        request.output_dir.as_os_str().to_os_string(),
        OsString::from("--stop-on-error"),
        // PlantUML otherwise lets `@startXYZ ../../name` escape --output-dir.
        // This official flag also covers filenames produced by preprocessing
        // and keeps every output under the isolated staging directory.
        OsString::from("--ignore-startuml-filename"),
    ];
    if !request.embed_source_metadata {
        args.push(OsString::from("--disable-metadata"));
    }
    if request.layout == Layout::Smetana {
        args.push(OsString::from("-Playout=smetana"));
    }
    args.push(request.input.as_os_str().to_os_string());
    args
}

fn local_allowlist(worktree_root: &Path, input: &Path, include_paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    push_unique(&mut paths, worktree_root.to_path_buf());
    if let Some(parent) = input.parent() {
        push_unique(&mut paths, parent.to_path_buf());
    }
    for include in include_paths {
        push_unique(&mut paths, include.clone());
    }
    paths
}

fn push_unique(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if !paths.contains(&path) {
        paths.push(path);
    }
}

pub fn minimum_java_major(format: Format) -> u32 {
    match format {
        Format::Svg | Format::Png => 17,
        Format::Pdf => 21,
    }
}

pub fn parse_java_major(output: &str) -> Option<u32> {
    for token in output.split(|character: char| {
        character.is_whitespace() || matches!(character, '"' | '\'' | '(' | ')')
    }) {
        let numeric_prefix: String = token
            .chars()
            .take_while(|character| character.is_ascii_digit() || *character == '.')
            .collect();
        if numeric_prefix.is_empty() {
            continue;
        }
        let mut parts = numeric_prefix.split('.');
        let first = parts.next()?.parse::<u32>().ok()?;
        if first == 1 {
            if let Some(major) = parts.next().and_then(|part| part.parse::<u32>().ok()) {
                return Some(major);
            }
        }
        return Some(first);
    }
    None
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HealthStatus {
    Ready,
    Missing,
    Incompatible,
    NotRequired,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComponentHealth {
    pub status: HealthStatus,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HealthReport {
    pub ready: bool,
    pub renderer: ComponentHealth,
    pub java: ComponentHealth,
    pub graphviz: ComponentHealth,
}

pub fn check_health(
    renderer: &Renderer,
    format: Format,
    layout: Layout,
    graphviz: &Path,
    executor: &dyn CommandExecutor,
) -> HealthReport {
    let renderer_health = renderer_health(renderer, executor);
    let java_health = java_health(renderer, format, executor);
    let graphviz_health = if layout == Layout::Graphviz {
        command_health(
            CommandSpec {
                program: graphviz.to_path_buf(),
                args: vec![OsString::from("-V")],
                env: Vec::new(),
                env_remove: Vec::new(),
            },
            "Graphviz",
            executor,
        )
    } else {
        ComponentHealth {
            status: HealthStatus::NotRequired,
            detail: "Smetana layout does not require Graphviz".to_string(),
        }
    };
    let ready = required_component_ready(&renderer_health)
        && required_component_ready(&java_health)
        && required_component_ready(&graphviz_health);
    HealthReport {
        ready,
        renderer: renderer_health,
        java: java_health,
        graphviz: graphviz_health,
    }
}

fn renderer_health(renderer: &Renderer, executor: &dyn CommandExecutor) -> ComponentHealth {
    match renderer {
        Renderer::Managed { cache_dir, .. } => {
            let jar = managed_jar_path(cache_dir);
            if !jar.is_file() {
                return ComponentHealth {
                    status: HealthStatus::Missing,
                    detail: format!("managed PlantUML jar is missing: {}", jar.display()),
                };
            }
            match Sha256Verifier.sha256(&jar) {
                Ok(actual) if actual.eq_ignore_ascii_case(MANAGED_PLANTUML_SHA256) => {
                    ComponentHealth {
                        status: HealthStatus::Ready,
                        detail: format!("PlantUML {MANAGED_PLANTUML_VERSION}"),
                    }
                }
                Ok(actual) => ComponentHealth {
                    status: HealthStatus::Incompatible,
                    detail: format!("managed PlantUML checksum mismatch: {actual}"),
                },
                Err(error) => ComponentHealth {
                    status: HealthStatus::Missing,
                    detail: error.to_string(),
                },
            }
        }
        Renderer::Jar { jar, .. } => file_health(jar, "PlantUML jar"),
        Renderer::Binary { executable } => command_health(
            CommandSpec {
                program: executable.clone(),
                args: vec![OsString::from("--version")],
                env: Vec::new(),
                env_remove: Vec::new(),
            },
            "PlantUML binary",
            executor,
        ),
    }
}

fn java_health(
    renderer: &Renderer,
    format: Format,
    executor: &dyn CommandExecutor,
) -> ComponentHealth {
    let java = match renderer {
        Renderer::Managed { java, .. } | Renderer::Jar { java, .. } => java,
        Renderer::Binary { .. } => {
            return ComponentHealth {
                status: HealthStatus::NotRequired,
                detail: "binary renderer owns its runtime".to_string(),
            };
        }
    };
    let command = CommandSpec {
        program: java.clone(),
        args: vec![OsString::from("-version")],
        env: Vec::new(),
        env_remove: Vec::new(),
    };
    match executor.execute(&command, HEALTH_TIMEOUT) {
        Ok(output) => {
            let combined = format!(
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            match parse_java_major(&combined) {
                Some(actual) if actual >= minimum_java_major(format) => ComponentHealth {
                    status: HealthStatus::Ready,
                    detail: format!("Java {actual}"),
                },
                Some(actual) => ComponentHealth {
                    status: HealthStatus::Incompatible,
                    detail: format!(
                        "Java {actual} is too old; {} output requires Java {}+",
                        format.as_str(),
                        minimum_java_major(format)
                    ),
                },
                None => ComponentHealth {
                    status: HealthStatus::Incompatible,
                    detail: "could not parse Java version".to_string(),
                },
            }
        }
        Err(error) => ComponentHealth {
            status: HealthStatus::Missing,
            detail: error.to_string(),
        },
    }
}

fn file_health(path: &Path, label: &str) -> ComponentHealth {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() && metadata.len() > 0 => ComponentHealth {
            status: HealthStatus::Ready,
            detail: format!("{label}: {}", path.display()),
        },
        Ok(_) => ComponentHealth {
            status: HealthStatus::Incompatible,
            detail: format!("{label} is empty or not a file: {}", path.display()),
        },
        Err(error) => ComponentHealth {
            status: HealthStatus::Missing,
            detail: format!("{label} not found at {}: {error}", path.display()),
        },
    }
}

fn command_health(
    command: CommandSpec,
    label: &str,
    executor: &dyn CommandExecutor,
) -> ComponentHealth {
    match executor.execute(&command, HEALTH_TIMEOUT) {
        Ok(_) => ComponentHealth {
            status: HealthStatus::Ready,
            detail: format!("{label}: {}", command.program.display()),
        },
        Err(error) => ComponentHealth {
            status: HealthStatus::Missing,
            detail: format!("{label} unavailable: {error}"),
        },
    }
}

fn required_component_ready(component: &ComponentHealth) -> bool {
    matches!(
        component.status,
        HealthStatus::Ready | HealthStatus::NotRequired
    )
}

pub trait CommandExecutor: Send + Sync {
    fn execute(
        &self,
        command: &CommandSpec,
        timeout: Duration,
    ) -> Result<ProcessOutput, ProcessFailure>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StdCommandExecutor;

impl CommandExecutor for StdCommandExecutor {
    fn execute(
        &self,
        command: &CommandSpec,
        timeout: Duration,
    ) -> Result<ProcessOutput, ProcessFailure> {
        crate::process_control::execute(command, timeout)
    }
}

pub fn validate_output(path: &Path, format: Format) -> Result<(), RendererError> {
    let metadata = fs::metadata(path).map_err(|error| RendererError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(RendererError::EmptyOutput {
            path: path.to_path_buf(),
        });
    }
    let mut header = Vec::new();
    File::open(path)
        .and_then(|file| file.take(4096).read_to_end(&mut header))
        .map_err(|error| RendererError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    let valid = match format {
        Format::Png => header.starts_with(b"\x89PNG\r\n\x1a\n"),
        Format::Pdf => header.starts_with(b"%PDF-"),
        Format::Svg => std::str::from_utf8(&header).is_ok_and(|header| {
            let header = header.trim_start_matches('\u{feff}').trim_start();
            header.starts_with("<svg") || (header.starts_with("<?xml") && header.contains("<svg"))
        }),
    };
    if !valid {
        return Err(RendererError::OutputType {
            path: path.to_path_buf(),
            expected: format,
        });
    }
    Ok(())
}

pub fn managed_jar_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join(format!("plantuml-{MANAGED_PLANTUML_VERSION}.jar"))
}

pub fn managed_java_dir(cache_dir: &Path) -> PathBuf {
    cache_dir.join(format!("temurin-jre-{MANAGED_JAVA_VERSION}"))
}

pub fn managed_java_path(cache_dir: &Path, asset: ManagedJavaAsset) -> PathBuf {
    managed_java_dir(cache_dir).join(asset.java_relative_path)
}

fn managed_lock_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join(format!("plantuml-{MANAGED_PLANTUML_VERSION}.jar.lock"))
}

fn managed_java_lock_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join(format!("temurin-jre-{MANAGED_JAVA_VERSION}.lock"))
}

pub trait Downloader: Send + Sync {
    fn download_to(&self, url: &str, destination: &Path) -> Result<(), DownloadError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct UreqDownloader;

impl Downloader for UreqDownloader {
    fn download_to(&self, url: &str, destination: &Path) -> Result<(), DownloadError> {
        let mut response = ureq::get(url).call().map_err(classify_request_error)?;
        if let Some(content_length) = response
            .headers()
            .get("content-length")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
        {
            if content_length > MAX_MANAGED_JAR_BYTES {
                return Err(DownloadError::permanent(format!(
                    "response content length {content_length} exceeds the {MAX_MANAGED_JAR_BYTES} byte limit"
                )));
            }
        }
        let mut destination_file = OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(destination)
            .map_err(|error| {
                DownloadError::permanent(format!(
                    "could not open {} for download: {error}",
                    destination.display()
                ))
            })?;
        let mut limited_reader = response
            .body_mut()
            .as_reader()
            .take(MAX_MANAGED_JAR_BYTES + 1);
        let mut downloaded = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = limited_reader.read(&mut buffer).map_err(|error| {
                DownloadError::retryable(format!("failed while streaming response body: {error}"))
            })?;
            if read == 0 {
                break;
            }
            destination_file
                .write_all(&buffer[..read])
                .map_err(|error| {
                    DownloadError::permanent(format!(
                        "failed to write downloaded file {}: {error}",
                        destination.display()
                    ))
                })?;
            downloaded += read as u64;
        }
        if downloaded > MAX_MANAGED_JAR_BYTES {
            let _ = destination_file.set_len(0);
            return Err(DownloadError::permanent(format!(
                "response exceeds the {MAX_MANAGED_JAR_BYTES} byte limit"
            )));
        }
        destination_file.sync_all().map_err(|error| {
            DownloadError::permanent(format!(
                "failed to flush downloaded file {}: {error}",
                destination.display()
            ))
        })?;
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DownloadError {
    message: String,
    retryable: bool,
}

impl DownloadError {
    pub fn retryable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: true,
        }
    }

    pub fn permanent(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: false,
        }
    }

    fn is_retryable(&self) -> bool {
        self.retryable
    }
}

impl fmt::Display for DownloadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DownloadError {}

fn classify_request_error(error: ureq::Error) -> DownloadError {
    let retryable = match &error {
        ureq::Error::StatusCode(status) => {
            matches!(*status, 408 | 425 | 429) || (500..=599).contains(status)
        }
        ureq::Error::Protocol(_)
        | ureq::Error::Io(_)
        | ureq::Error::Timeout(_)
        | ureq::Error::HostNotFound
        | ureq::Error::ConnectionFailed
        | ureq::Error::Tls(_)
        | ureq::Error::ConnectProxyFailed(_)
        | ureq::Error::Other(_)
        | ureq::Error::BodyStalled => true,
        _ => false,
    };
    let message = format!("HTTP request failed: {error}");
    if retryable {
        DownloadError::retryable(message)
    } else {
        DownloadError::permanent(message)
    }
}

pub trait ChecksumVerifier: Send + Sync {
    fn sha256(&self, path: &Path) -> Result<String, RendererError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Sha256Verifier;

impl ChecksumVerifier for Sha256Verifier {
    fn sha256(&self, path: &Path) -> Result<String, RendererError> {
        let file = File::open(path).map_err(|error| RendererError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
        let mut reader = BufReader::new(file);
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = reader
                .read(&mut buffer)
                .map_err(|error| RendererError::Io {
                    path: path.to_path_buf(),
                    message: error.to_string(),
                })?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        Ok(hex_lower(&hasher.finalize()))
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

#[derive(Clone, Copy, Debug)]
pub struct InstallPolicy {
    pub lock_timeout: Duration,
    pub poll_interval: Duration,
    pub download_attempts: usize,
    pub download_retry_delay: Duration,
}

impl InstallPolicy {
    pub fn for_tests() -> Self {
        Self {
            lock_timeout: Duration::from_millis(25),
            poll_interval: Duration::from_millis(2),
            download_attempts: 3,
            download_retry_delay: Duration::ZERO,
        }
    }
}

impl Default for InstallPolicy {
    fn default() -> Self {
        Self {
            lock_timeout: Duration::from_secs(30),
            poll_interval: Duration::from_millis(50),
            download_attempts: 3,
            download_retry_delay: Duration::from_millis(250),
        }
    }
}

fn download_with_retry(
    downloader: &dyn Downloader,
    url: &str,
    destination: &Path,
    policy: InstallPolicy,
) -> Result<(), DownloadError> {
    let attempts = policy.download_attempts.max(1);
    for attempt in 1..=attempts {
        match downloader.download_to(url, destination) {
            Ok(()) => return Ok(()),
            Err(error) if error.is_retryable() && attempt < attempts => {
                OpenOptions::new()
                    .write(true)
                    .truncate(true)
                    .open(destination)
                    .map_err(|reset_error| {
                        DownloadError::permanent(format!(
                            "failed to reset partial download {}: {reset_error}",
                            destination.display()
                        ))
                    })?;
                thread::sleep(policy.download_retry_delay.saturating_mul(attempt as u32));
            }
            Err(error) if error.is_retryable() => {
                return Err(DownloadError::permanent(format!(
                    "failed after {attempts} attempts: {error}"
                )));
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("download attempt loop always returns")
}

pub fn ensure_managed_jar(cache_dir: &Path, offline: bool) -> Result<PathBuf, RendererError> {
    ensure_managed_jar_with(
        cache_dir,
        offline,
        &UreqDownloader,
        &Sha256Verifier,
        InstallPolicy::default(),
    )
}

pub fn ensure_managed_jar_with(
    cache_dir: &Path,
    offline: bool,
    downloader: &dyn Downloader,
    verifier: &dyn ChecksumVerifier,
    policy: InstallPolicy,
) -> Result<PathBuf, RendererError> {
    let jar = managed_jar_path(cache_dir);
    if file_matches(&jar, verifier)? {
        return Ok(jar);
    }
    if offline {
        return Err(RendererError::Offline {
            path: jar,
            component: "managed PlantUML jar",
        });
    }

    fs::create_dir_all(cache_dir).map_err(|error| RendererError::Io {
        path: cache_dir.to_path_buf(),
        message: error.to_string(),
    })?;
    let lock_path = managed_lock_path(cache_dir);
    let lock = acquire_lock(&lock_path, || file_matches(&jar, verifier), policy)?;
    let Some(_lock) = lock else {
        return Ok(jar);
    };
    if file_matches(&jar, verifier)? {
        return Ok(jar);
    }

    let temporary = reserve_temporary_path(cache_dir)?;
    let mut temporary_guard = RemoveOnDrop::new(temporary.clone());
    download_with_retry(downloader, MANAGED_PLANTUML_URL, &temporary, policy).map_err(|error| {
        RendererError::Download {
            url: MANAGED_PLANTUML_URL.to_string(),
            message: error.to_string(),
        }
    })?;
    let downloaded_size = fs::metadata(&temporary)
        .map_err(|error| RendererError::Io {
            path: temporary.clone(),
            message: error.to_string(),
        })?
        .len();
    if downloaded_size > MAX_MANAGED_JAR_BYTES {
        return Err(RendererError::DownloadTooLarge {
            path: temporary,
            limit: MAX_MANAGED_JAR_BYTES,
            actual: downloaded_size,
        });
    }
    OpenOptions::new()
        .write(true)
        .open(&temporary)
        .and_then(|file| file.sync_all())
        .map_err(|error| RendererError::Io {
            path: temporary.clone(),
            message: error.to_string(),
        })?;
    let actual = verifier.sha256(&temporary)?;
    if !actual.eq_ignore_ascii_case(MANAGED_PLANTUML_SHA256) {
        return Err(RendererError::Checksum {
            path: temporary,
            expected: MANAGED_PLANTUML_SHA256.to_string(),
            actual,
        });
    }

    atomic_install(&temporary, &jar)?;
    temporary_guard.disarm();
    Ok(jar)
}

pub fn ensure_managed_java(cache_dir: &Path, offline: bool) -> Result<PathBuf, RendererError> {
    let asset = current_managed_java_asset()?;
    ensure_managed_java_with(
        cache_dir,
        offline,
        asset,
        &UreqDownloader,
        &Sha256Verifier,
        InstallPolicy::default(),
    )
}

pub fn ensure_managed_java_with(
    cache_dir: &Path,
    offline: bool,
    asset: ManagedJavaAsset,
    downloader: &dyn Downloader,
    verifier: &dyn ChecksumVerifier,
    policy: InstallPolicy,
) -> Result<PathBuf, RendererError> {
    let java = managed_java_path(cache_dir, asset);
    if managed_java_ready(cache_dir, asset)? {
        return Ok(java);
    }
    if offline {
        return Err(RendererError::Offline {
            path: java,
            component: "managed Java runtime",
        });
    }

    fs::create_dir_all(cache_dir).map_err(|error| RendererError::Io {
        path: cache_dir.to_path_buf(),
        message: error.to_string(),
    })?;
    let lock_path = managed_java_lock_path(cache_dir);
    let lock = acquire_lock(&lock_path, || managed_java_ready(cache_dir, asset), policy)?;
    let Some(_lock) = lock else {
        return Ok(java);
    };
    if managed_java_ready(cache_dir, asset)? {
        return Ok(java);
    }

    let archive = reserve_temporary_named_path(cache_dir, "temurin-jre.archive")?;
    let _archive_guard = RemoveOnDrop::new(archive.clone());
    download_with_retry(downloader, asset.url, &archive, policy).map_err(|error| {
        RendererError::Download {
            url: asset.url.to_string(),
            message: error.to_string(),
        }
    })?;
    let downloaded_size = fs::metadata(&archive)
        .map_err(|error| RendererError::Io {
            path: archive.clone(),
            message: error.to_string(),
        })?
        .len();
    if downloaded_size > MAX_MANAGED_JAVA_ARCHIVE_BYTES {
        return Err(RendererError::DownloadTooLarge {
            path: archive,
            limit: MAX_MANAGED_JAVA_ARCHIVE_BYTES,
            actual: downloaded_size,
        });
    }
    let actual = verifier.sha256(&archive)?;
    if !actual.eq_ignore_ascii_case(asset.sha256) {
        return Err(RendererError::Checksum {
            path: archive,
            expected: asset.sha256.to_string(),
            actual,
        });
    }

    let extraction = tempfile::Builder::new()
        .prefix("temurin-jre.extract-")
        .tempdir_in(cache_dir)
        .map_err(|error| RendererError::Io {
            path: cache_dir.to_path_buf(),
            message: format!("could not create managed Java staging directory: {error}"),
        })?;
    extract_managed_java_archive(&archive, extraction.path(), asset.archive_format)?;
    let staged_root = single_archive_root(extraction.path())?;
    let staged_java = staged_root.join(asset.java_relative_path);
    if !managed_java_executable(&staged_java) {
        return Err(RendererError::Archive {
            path: archive,
            message: format!(
                "archive does not contain an executable Java runtime at {}",
                asset.java_relative_path
            ),
        });
    }
    let marker = staged_root.join(MANAGED_JAVA_MARKER);
    fs::write(&marker, managed_java_marker(asset)).map_err(|error| RendererError::Io {
        path: marker.clone(),
        message: error.to_string(),
    })?;
    OpenOptions::new()
        .write(true)
        .open(&marker)
        .and_then(|file| file.sync_all())
        .map_err(|error| RendererError::Io {
            path: marker,
            message: error.to_string(),
        })?;

    atomic_install_directory(&staged_root, &managed_java_dir(cache_dir))?;
    extraction.close().map_err(|error| RendererError::Io {
        path: cache_dir.to_path_buf(),
        message: format!("could not clean managed Java staging directory: {error}"),
    })?;
    if !managed_java_ready(cache_dir, asset)? {
        return Err(RendererError::Archive {
            path: managed_java_dir(cache_dir),
            message: "installed managed Java runtime failed its integrity marker check".to_string(),
        });
    }
    Ok(java)
}

fn managed_java_marker(asset: ManagedJavaAsset) -> String {
    format!(
        "version={MANAGED_JAVA_VERSION}\narchive-sha256={}\n",
        asset.sha256
    )
}

fn managed_java_ready(cache_dir: &Path, asset: ManagedJavaAsset) -> Result<bool, RendererError> {
    let root = managed_java_dir(cache_dir);
    let root_metadata = match fs::symlink_metadata(&root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(RendererError::Io {
                path: root,
                message: error.to_string(),
            });
        }
    };
    if !root_metadata.file_type().is_dir() || root_metadata.file_type().is_symlink() {
        return Ok(false);
    }
    let marker = root.join(MANAGED_JAVA_MARKER);
    let marker_metadata = match fs::symlink_metadata(&marker) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(RendererError::Io {
                path: marker,
                message: error.to_string(),
            });
        }
    };
    if !marker_metadata.file_type().is_file() || marker_metadata.file_type().is_symlink() {
        return Ok(false);
    }
    let marker_contents = match fs::read_to_string(&marker) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(RendererError::Io {
                path: marker,
                message: error.to_string(),
            });
        }
    };
    Ok(marker_contents == managed_java_marker(asset)
        && managed_java_executable(&managed_java_path(cache_dir, asset)))
}

#[cfg(unix)]
fn managed_java_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    fs::symlink_metadata(path).is_ok_and(|metadata| {
        metadata.file_type().is_file()
            && !metadata.file_type().is_symlink()
            && metadata.permissions().mode() & 0o111 != 0
    })
}

#[cfg(not(unix))]
fn managed_java_executable(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.file_type().is_file() && !metadata.file_type().is_symlink())
}

pub fn extract_managed_java_archive(
    archive_path: &Path,
    destination: &Path,
    archive_format: ManagedArchiveFormat,
) -> Result<(), RendererError> {
    extract_managed_java_archive_with_limits(
        archive_path,
        destination,
        archive_format,
        MAX_MANAGED_JAVA_EXPANDED_BYTES,
        MAX_MANAGED_JAVA_ARCHIVE_ENTRIES,
    )
}

pub fn extract_managed_java_archive_with_limits(
    archive_path: &Path,
    destination: &Path,
    archive_format: ManagedArchiveFormat,
    max_expanded_bytes: u64,
    max_entries: usize,
) -> Result<(), RendererError> {
    fs::create_dir_all(destination).map_err(|error| RendererError::Io {
        path: destination.to_path_buf(),
        message: error.to_string(),
    })?;
    match archive_format {
        ManagedArchiveFormat::TarGz => {
            extract_tar_gz(archive_path, destination, max_expanded_bytes, max_entries)
        }
        ManagedArchiveFormat::Zip => {
            extract_zip(archive_path, destination, max_expanded_bytes, max_entries)
        }
    }
}

fn extract_tar_gz(
    archive_path: &Path,
    destination: &Path,
    max_expanded_bytes: u64,
    max_entries: usize,
) -> Result<(), RendererError> {
    let file = File::open(archive_path).map_err(|error| RendererError::Io {
        path: archive_path.to_path_buf(),
        message: error.to_string(),
    })?;
    let mut archive = tar::Archive::new(GzDecoder::new(BufReader::new(file)));
    let mut expanded = 0_u64;
    let mut seen = BTreeSet::new();
    let mut links = Vec::new();
    let entries = archive.entries().map_err(|error| RendererError::Archive {
        path: archive_path.to_path_buf(),
        message: error.to_string(),
    })?;
    for (index, entry) in entries.enumerate() {
        if index >= max_entries {
            return Err(RendererError::Archive {
                path: archive_path.to_path_buf(),
                message: format!("archive exceeds the {max_entries} entry limit"),
            });
        }
        let mut entry = entry.map_err(|error| RendererError::Archive {
            path: archive_path.to_path_buf(),
            message: error.to_string(),
        })?;
        let relative =
            safe_archive_path(&entry.path().map_err(|error| RendererError::Archive {
                path: archive_path.to_path_buf(),
                message: error.to_string(),
            })?)
            .ok_or_else(|| RendererError::Archive {
                path: archive_path.to_path_buf(),
                message: "archive contains an unsafe path".to_string(),
            })?;
        if !seen.insert(relative.clone()) {
            return Err(RendererError::Archive {
                path: archive_path.to_path_buf(),
                message: format!("archive contains duplicate path {}", relative.display()),
            });
        }
        let kind = entry.header().entry_type();
        if kind.is_symlink() {
            let target = entry
                .link_name()
                .map_err(|error| RendererError::Archive {
                    path: archive_path.to_path_buf(),
                    message: error.to_string(),
                })?
                .and_then(|target| resolve_archive_link(&relative, &target))
                .ok_or_else(|| RendererError::Archive {
                    path: archive_path.to_path_buf(),
                    message: format!(
                        "archive link {} escapes its top-level runtime",
                        relative.display()
                    ),
                })?;
            links.push((relative, target));
            continue;
        }
        if !kind.is_file() && !kind.is_dir() {
            return Err(RendererError::Archive {
                path: archive_path.to_path_buf(),
                message: format!(
                    "archive entry {} is a hard link or unsupported special file",
                    relative.display()
                ),
            });
        }
        expanded = expanded
            .checked_add(entry.size())
            .ok_or_else(|| RendererError::Archive {
                path: archive_path.to_path_buf(),
                message: "archive expanded size overflow".to_string(),
            })?;
        if expanded > max_expanded_bytes {
            return Err(RendererError::Archive {
                path: archive_path.to_path_buf(),
                message: format!("archive expands beyond the {max_expanded_bytes} byte limit"),
            });
        }
        let unpacked = entry
            .unpack_in(destination)
            .map_err(|error| RendererError::Archive {
                path: archive_path.to_path_buf(),
                message: error.to_string(),
            })?;
        if !unpacked {
            return Err(RendererError::Archive {
                path: archive_path.to_path_buf(),
                message: format!("archive path escaped staging: {}", relative.display()),
            });
        }
    }
    for (relative, target) in links {
        let source = destination.join(&target);
        let source_metadata =
            fs::symlink_metadata(&source).map_err(|error| RendererError::Archive {
                path: archive_path.to_path_buf(),
                message: format!(
                    "archive link {} has an unavailable target {}: {error}",
                    relative.display(),
                    target.display()
                ),
            })?;
        if !source_metadata.file_type().is_file() || source_metadata.file_type().is_symlink() {
            return Err(RendererError::Archive {
                path: archive_path.to_path_buf(),
                message: format!(
                    "archive link {} does not target a regular file",
                    relative.display()
                ),
            });
        }
        expanded =
            expanded
                .checked_add(source_metadata.len())
                .ok_or_else(|| RendererError::Archive {
                    path: archive_path.to_path_buf(),
                    message: "archive materialized-link size overflow".to_string(),
                })?;
        if expanded > max_expanded_bytes {
            return Err(RendererError::Archive {
                path: archive_path.to_path_buf(),
                message: format!("archive expands beyond the {max_expanded_bytes} byte limit"),
            });
        }
        let output = destination.join(&relative);
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(|error| RendererError::Io {
                path: parent.to_path_buf(),
                message: error.to_string(),
            })?;
        }
        fs::copy(&source, &output).map_err(|error| RendererError::Io {
            path: output,
            message: error.to_string(),
        })?;
    }
    Ok(())
}

fn extract_zip(
    archive_path: &Path,
    destination: &Path,
    max_expanded_bytes: u64,
    max_entries: usize,
) -> Result<(), RendererError> {
    let file = File::open(archive_path).map_err(|error| RendererError::Io {
        path: archive_path.to_path_buf(),
        message: error.to_string(),
    })?;
    let mut archive =
        zip::ZipArchive::new(BufReader::new(file)).map_err(|error| RendererError::Archive {
            path: archive_path.to_path_buf(),
            message: error.to_string(),
        })?;
    if archive.len() > max_entries {
        return Err(RendererError::Archive {
            path: archive_path.to_path_buf(),
            message: format!("archive exceeds the {max_entries} entry limit"),
        });
    }
    let mut expanded = 0_u64;
    let mut seen = BTreeSet::new();
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| RendererError::Archive {
                path: archive_path.to_path_buf(),
                message: error.to_string(),
            })?;
        let relative = entry
            .enclosed_name()
            .as_deref()
            .and_then(safe_archive_path)
            .ok_or_else(|| RendererError::Archive {
                path: archive_path.to_path_buf(),
                message: "archive contains an unsafe path".to_string(),
            })?;
        if !seen.insert(relative.clone()) {
            return Err(RendererError::Archive {
                path: archive_path.to_path_buf(),
                message: format!("archive contains duplicate path {}", relative.display()),
            });
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(RendererError::Archive {
                path: archive_path.to_path_buf(),
                message: format!("archive entry {} is a symbolic link", relative.display()),
            });
        }
        expanded = expanded
            .checked_add(entry.size())
            .ok_or_else(|| RendererError::Archive {
                path: archive_path.to_path_buf(),
                message: "archive expanded size overflow".to_string(),
            })?;
        if expanded > max_expanded_bytes {
            return Err(RendererError::Archive {
                path: archive_path.to_path_buf(),
                message: format!("archive expands beyond the {max_expanded_bytes} byte limit"),
            });
        }

        let output = destination.join(&relative);
        if entry.is_dir() {
            fs::create_dir_all(&output).map_err(|error| RendererError::Io {
                path: output,
                message: error.to_string(),
            })?;
            continue;
        }
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(|error| RendererError::Io {
                path: parent.to_path_buf(),
                message: error.to_string(),
            })?;
        }
        let mut output_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)
            .map_err(|error| RendererError::Io {
                path: output.clone(),
                message: error.to_string(),
            })?;
        std::io::copy(&mut entry, &mut output_file).map_err(|error| RendererError::Io {
            path: output.clone(),
            message: error.to_string(),
        })?;
        output_file.sync_all().map_err(|error| RendererError::Io {
            path: output.clone(),
            message: error.to_string(),
        })?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&output, fs::Permissions::from_mode(mode & 0o777)).map_err(
                |error| RendererError::Io {
                    path: output,
                    message: error.to_string(),
                },
            )?;
        }
    }
    Ok(())
}

fn safe_archive_path(path: &Path) -> Option<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!normalized.as_os_str().is_empty()).then_some(normalized)
}

fn resolve_archive_link(link_path: &Path, target: &Path) -> Option<PathBuf> {
    if target.is_absolute() {
        return None;
    }
    let top_level = link_path.components().next()?;
    let mut normalized = link_path.parent()?.to_path_buf();
    for component in target.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (normalized.components().next() == Some(top_level)).then_some(normalized)
}

fn single_archive_root(staging: &Path) -> Result<PathBuf, RendererError> {
    let mut roots = fs::read_dir(staging)
        .map_err(|error| RendererError::Io {
            path: staging.to_path_buf(),
            message: error.to_string(),
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| RendererError::Io {
            path: staging.to_path_buf(),
            message: error.to_string(),
        })?;
    if roots.len() != 1 {
        return Err(RendererError::Archive {
            path: staging.to_path_buf(),
            message: "managed Java archive must contain exactly one top-level directory"
                .to_string(),
        });
    }
    let root = roots.pop().expect("one root was checked").path();
    if !root.is_dir() {
        return Err(RendererError::Archive {
            path: staging.to_path_buf(),
            message: "managed Java archive top-level entry is not a directory".to_string(),
        });
    }
    Ok(root)
}

fn file_matches(path: &Path, verifier: &dyn ChecksumVerifier) -> Result<bool, RendererError> {
    if !path.is_file() {
        return Ok(false);
    }
    verifier
        .sha256(path)
        .map(|actual| actual.eq_ignore_ascii_case(MANAGED_PLANTUML_SHA256))
}

fn acquire_lock<F>(
    lock_path: &Path,
    mut installed: F,
    policy: InstallPolicy,
) -> Result<Option<ManagedInstallLock>, RendererError>
where
    F: FnMut() -> Result<bool, RendererError>,
{
    let mut lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .map_err(|error| RendererError::Io {
            path: lock_path.to_path_buf(),
            message: error.to_string(),
        })?;
    let started = Instant::now();
    loop {
        match lock.try_lock() {
            Ok(()) => {
                // The lock file is deliberately persistent. The operating-system lock is
                // attached to this handle and is released even if the process crashes.
                let _ = lock.set_len(0);
                let _ = writeln!(lock, "{}", std::process::id());
                let _ = lock.sync_data();
                return Ok(Some(ManagedInstallLock { _file: lock }));
            }
            Err(TryLockError::WouldBlock) => {
                if installed()? {
                    return Ok(None);
                }
                if started.elapsed() >= policy.lock_timeout {
                    return Err(RendererError::LockTimeout {
                        path: lock_path.to_path_buf(),
                        timeout: policy.lock_timeout,
                    });
                }
                if policy.poll_interval.is_zero() {
                    thread::yield_now();
                } else {
                    thread::sleep(policy.poll_interval);
                }
            }
            Err(TryLockError::Error(error)) => {
                return Err(RendererError::Io {
                    path: lock_path.to_path_buf(),
                    message: error.to_string(),
                });
            }
        }
    }
}

struct ManagedInstallLock {
    _file: File,
}

fn reserve_temporary_path(cache_dir: &Path) -> Result<PathBuf, RendererError> {
    reserve_temporary_named_path(
        cache_dir,
        &format!("plantuml-{MANAGED_PLANTUML_VERSION}.jar"),
    )
}

fn reserve_temporary_named_path(cache_dir: &Path, name: &str) -> Result<PathBuf, RendererError> {
    for _ in 0..100 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = cache_dir.join(format!("{name}.tmp-{}-{sequence}", std::process::id()));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(RendererError::Io {
                    path,
                    message: error.to_string(),
                });
            }
        }
    }
    Err(RendererError::Io {
        path: cache_dir.to_path_buf(),
        message: "could not reserve a unique temporary file".to_string(),
    })
}

fn atomic_install(temporary: &Path, destination: &Path) -> Result<(), RendererError> {
    match fs::rename(temporary, destination) {
        Ok(()) => Ok(()),
        Err(first_error) if destination.exists() => {
            fs::remove_file(destination).map_err(|error| RendererError::Io {
                path: destination.to_path_buf(),
                message: format!("failed to replace existing managed jar: {error}"),
            })?;
            fs::rename(temporary, destination).map_err(|error| RendererError::Io {
                path: destination.to_path_buf(),
                message: format!("atomic install failed after {first_error}: {error}"),
            })
        }
        Err(error) => Err(RendererError::Io {
            path: destination.to_path_buf(),
            message: format!("atomic install failed: {error}"),
        }),
    }
}

fn atomic_install_directory(staged: &Path, destination: &Path) -> Result<(), RendererError> {
    let file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| RendererError::Io {
            path: destination.to_path_buf(),
            message: "managed Java destination has an invalid file name".to_string(),
        })?;
    let backup = destination.with_file_name(format!("{file_name}.previous"));
    remove_managed_path_if_present(&backup)?;
    let had_destination = fs::symlink_metadata(destination).is_ok();
    if had_destination {
        fs::rename(destination, &backup).map_err(|error| RendererError::Io {
            path: destination.to_path_buf(),
            message: format!("failed to stage existing managed Java runtime: {error}"),
        })?;
    }
    if let Err(error) = fs::rename(staged, destination) {
        if had_destination {
            let _ = fs::rename(&backup, destination);
        }
        return Err(RendererError::Io {
            path: destination.to_path_buf(),
            message: format!("failed to install managed Java runtime atomically: {error}"),
        });
    }
    if had_destination {
        remove_managed_path_if_present(&backup)?;
    }
    Ok(())
}

fn remove_managed_path_if_present(path: &Path) -> Result<(), RendererError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(RendererError::Io {
                path: path.to_path_buf(),
                message: error.to_string(),
            });
        }
    };
    let result = if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    result.map_err(|error| RendererError::Io {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

struct RemoveOnDrop {
    path: PathBuf,
    armed: bool,
}

impl RemoveOnDrop {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RendererError {
    InvalidAllowlist {
        message: String,
    },
    InvalidGraphvizPath {
        path: PathBuf,
    },
    Io {
        path: PathBuf,
        message: String,
    },
    EmptyOutput {
        path: PathBuf,
    },
    OutputType {
        path: PathBuf,
        expected: Format,
    },
    Offline {
        path: PathBuf,
        component: &'static str,
    },
    LockTimeout {
        path: PathBuf,
        timeout: Duration,
    },
    Download {
        url: String,
        message: String,
    },
    DownloadTooLarge {
        path: PathBuf,
        limit: u64,
        actual: u64,
    },
    Checksum {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    Archive {
        path: PathBuf,
        message: String,
    },
    UnsupportedPlatform {
        os: String,
        architecture: String,
    },
}

impl fmt::Display for RendererError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAllowlist { message } => {
                write!(formatter, "invalid local allowlist path: {message}")
            }
            Self::InvalidGraphvizPath { path } => write!(
                formatter,
                "Graphviz executable path must be absolute after resolution: {}",
                path.display()
            ),
            Self::Io { path, message } => write!(formatter, "{}: {message}", path.display()),
            Self::EmptyOutput { path } => {
                write!(
                    formatter,
                    "renderer produced an empty output: {}",
                    path.display()
                )
            }
            Self::OutputType { path, expected } => write!(
                formatter,
                "renderer output {} is not valid {} data",
                path.display(),
                expected.as_str()
            ),
            Self::Offline { path, component } => write!(
                formatter,
                "{component} is unavailable at {} and offline mode forbids download",
                path.display()
            ),
            Self::LockTimeout { path, timeout } => write!(
                formatter,
                "timed out after {} ms waiting for managed renderer lock {}",
                timeout.as_millis(),
                path.display()
            ),
            Self::Download { url, message } => {
                write!(formatter, "failed to download {url}: {message}")
            }
            Self::DownloadTooLarge {
                path,
                limit,
                actual,
            } => write!(
                formatter,
                "downloaded file {} is {actual} bytes, exceeding the {limit} byte limit",
                path.display()
            ),
            Self::Checksum {
                path,
                expected,
                actual,
            } => write!(
                formatter,
                "checksum mismatch for {}: expected {expected}, got {actual}",
                path.display()
            ),
            Self::Archive { path, message } => {
                write!(
                    formatter,
                    "invalid managed runtime archive {}: {message}",
                    path.display()
                )
            }
            Self::UnsupportedPlatform { os, architecture } => write!(
                formatter,
                "managed Java runtime is unavailable for {os}/{architecture}"
            ),
        }
    }
}

impl std::error::Error for RendererError {}
