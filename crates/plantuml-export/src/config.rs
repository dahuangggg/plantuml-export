use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use url::Url;

use crate::cli::{Cli, Command, Layout, OutputFormat, RendererMode};
use crate::AppError;

pub const PROJECT_CONFIG_NAME: &str = "plantuml-export.toml";

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConfigOverrides {
    pub renderer: Option<RendererMode>,
    pub format: Option<OutputFormat>,
    pub out_dir: Option<PathBuf>,
    pub layout: Option<Layout>,
    pub embed_source_metadata: Option<bool>,
    pub include_paths: Option<Vec<PathBuf>>,
    pub include: Option<Vec<String>>,
    pub exclude: Option<Vec<String>>,
    pub offline: Option<bool>,
    pub java_path: Option<PathBuf>,
    pub binary_path: Option<PathBuf>,
    pub jar_path: Option<PathBuf>,
    pub graphviz_path: Option<PathBuf>,
}

impl ConfigOverrides {
    pub fn from_cli(cli: &Cli) -> Self {
        match &cli.command {
            Command::Export(export) => Self {
                renderer: export.renderer,
                format: export.format,
                out_dir: export.out_dir.clone(),
                layout: export.layout,
                embed_source_metadata: export
                    .embed_source_metadata
                    .then_some(true)
                    .or_else(|| export.disable_metadata.then_some(false)),
                include_paths: (!export.include_paths.is_empty())
                    .then(|| export.include_paths.clone()),
                include: (!export.include.is_empty()).then(|| export.include.clone()),
                exclude: (!export.exclude.is_empty()).then(|| export.exclude.clone()),
                offline: export.offline.then_some(true),
                java_path: export.java_path.clone(),
                binary_path: export.binary_path.clone(),
                jar_path: export.jar_path.clone(),
                graphviz_path: export.graphviz_path.clone(),
            },
            Command::Check(check) => Self {
                include_paths: (!check.include_paths.is_empty())
                    .then(|| check.include_paths.clone()),
                ..Self::default()
            },
            _ => Self::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct ConfigRequest {
    pub cwd: PathBuf,
    pub root: Option<PathBuf>,
    pub config: Option<PathBuf>,
    pub user_config: Option<PathBuf>,
    pub overrides: ConfigOverrides,
}

impl ConfigRequest {
    pub fn from_cli(cli: &Cli) -> Result<Self, AppError> {
        let cwd = env::current_dir().map_err(|error| {
            AppError::usage(
                "current_directory",
                format!("failed to read current directory: {error}"),
            )
        })?;

        Ok(Self {
            cwd,
            root: cli.root.clone(),
            config: cli.config.clone(),
            user_config: default_user_config_path(),
            overrides: ConfigOverrides::from_cli(cli),
        })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RemoteIncludes {
    Public,
    Allowlist,
    Disabled,
}

impl RemoteIncludes {
    fn restrict(self, requested: Self) -> Self {
        self.max(requested)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedConfig {
    pub root: PathBuf,
    pub project_config: Option<PathBuf>,
    pub user_config: Option<PathBuf>,
    pub renderer: RendererMode,
    pub format: OutputFormat,
    pub out_dir: PathBuf,
    pub layout: Layout,
    pub embed_source_metadata: bool,
    pub include_paths: Vec<PathBuf>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub offline: bool,
    pub remote_includes: RemoteIncludes,
    pub allowed_remote_urls: Vec<String>,
    pub java_path: PathBuf,
    pub binary_path: Option<PathBuf>,
    pub jar_path: Option<PathBuf>,
    pub graphviz_path: PathBuf,
}

impl ResolvedConfig {
    fn defaults(root: PathBuf) -> Self {
        Self {
            root,
            project_config: None,
            user_config: None,
            renderer: RendererMode::Managed,
            format: OutputFormat::Svg,
            out_dir: PathBuf::from("out"),
            layout: Layout::Smetana,
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

    fn apply(&mut self, input: RawConfig, allow_machine_settings: bool) {
        apply_option(&mut self.renderer, input.renderer);
        apply_option(&mut self.format, input.format);
        apply_option(&mut self.out_dir, input.out_dir);
        apply_option(&mut self.layout, input.layout);
        apply_option(&mut self.embed_source_metadata, input.embed_source_metadata);
        extend_unique(&mut self.include_paths, input.include_paths);
        apply_option(&mut self.include, input.include);
        apply_option(&mut self.exclude, input.exclude);
        if input.offline == Some(true) {
            self.offline = true;
        }
        if let Some(remote_includes) = input.remote_includes {
            self.remote_includes = self.remote_includes.restrict(remote_includes);
        }

        if allow_machine_settings {
            apply_option(&mut self.allowed_remote_urls, input.allowed_remote_urls);
            apply_option(&mut self.java_path, input.java_path);
            apply_option(&mut self.binary_path, input.binary_path.map(Some));
            apply_option(&mut self.jar_path, input.jar_path.map(Some));
            apply_option(&mut self.graphviz_path, input.graphviz_path);
        }
    }

    fn apply_overrides(&mut self, input: ConfigOverrides) {
        apply_option(&mut self.renderer, input.renderer);
        apply_option(&mut self.format, input.format);
        apply_option(&mut self.out_dir, input.out_dir);
        apply_option(&mut self.layout, input.layout);
        apply_option(&mut self.embed_source_metadata, input.embed_source_metadata);
        extend_unique(&mut self.include_paths, input.include_paths);
        apply_option(&mut self.include, input.include);
        apply_option(&mut self.exclude, input.exclude);
        apply_option(&mut self.offline, input.offline);
        apply_option(&mut self.java_path, input.java_path);
        apply_option(&mut self.binary_path, input.binary_path.map(Some));
        apply_option(&mut self.jar_path, input.jar_path.map(Some));
        apply_option(&mut self.graphviz_path, input.graphviz_path);
    }

    pub fn apply_workspace_options(
        &mut self,
        include_paths: Vec<PathBuf>,
        remote_includes: Option<RemoteIncludes>,
    ) {
        extend_unique(&mut self.include_paths, Some(include_paths));
        if let Some(remote_includes) = remote_includes {
            self.remote_includes = self.remote_includes.restrict(remote_includes);
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    renderer: Option<RendererMode>,
    format: Option<OutputFormat>,
    #[serde(rename = "outDir", alias = "out_dir")]
    out_dir: Option<PathBuf>,
    layout: Option<Layout>,
    #[serde(rename = "embedSourceMetadata", alias = "embed_source_metadata")]
    embed_source_metadata: Option<bool>,
    #[serde(rename = "includePaths", alias = "include_paths")]
    include_paths: Option<Vec<PathBuf>>,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    offline: Option<bool>,
    #[serde(rename = "remoteIncludes", alias = "remote_includes")]
    remote_includes: Option<RemoteIncludes>,
    #[serde(rename = "allowedRemoteUrls", alias = "allowed_remote_urls")]
    allowed_remote_urls: Option<Vec<String>>,
    #[serde(rename = "javaPath", alias = "java_path")]
    java_path: Option<PathBuf>,
    #[serde(rename = "binaryPath", alias = "binary_path")]
    binary_path: Option<PathBuf>,
    #[serde(rename = "jarPath", alias = "jar_path")]
    jar_path: Option<PathBuf>,
    #[serde(rename = "graphvizPath", alias = "graphviz_path")]
    graphviz_path: Option<PathBuf>,
}

pub fn resolve(request: ConfigRequest) -> Result<ResolvedConfig, AppError> {
    let cwd = canonical_directory(&request.cwd, "current directory")?;
    let root = match request.root {
        Some(root) => canonical_directory(&resolve_from(&cwd, &root), "--root")?,
        None => find_git_root(&cwd).unwrap_or_else(|| cwd.clone()),
    };

    let mut resolved = ResolvedConfig::defaults(root.clone());

    if let Some(user_path) = request.user_config {
        let user_path = resolve_from(&cwd, &user_path);
        if user_path.exists() {
            let user_path = canonical_file(&user_path, "user config")?;
            let user = read_config(&user_path, ConfigScope::User)?;
            resolved.apply(user, true);
            resolved.user_config = Some(user_path);
        }
    }

    let project_path = match request.config {
        Some(config) => Some(canonical_file(&resolve_from(&cwd, &config), "--config")?),
        None => {
            let default = root.join(PROJECT_CONFIG_NAME);
            default.exists().then_some(default)
        }
    };

    if let Some(project_path) = project_path {
        let project_path = canonical_file(&project_path, "project config")?;
        let project = read_config(&project_path, ConfigScope::Project)?;
        resolved.apply(project, false);
        resolved.project_config = Some(project_path);
    }

    resolved.apply_overrides(request.overrides);
    resolved.allowed_remote_urls = normalize_remote_urls(resolved.allowed_remote_urls)?;
    Ok(resolved)
}

fn apply_option<T>(target: &mut T, value: Option<T>) {
    if let Some(value) = value {
        *target = value;
    }
}

fn extend_unique<T: PartialEq>(target: &mut Vec<T>, values: Option<Vec<T>>) {
    for value in values.into_iter().flatten() {
        if !target.contains(&value) {
            target.push(value);
        }
    }
}

fn normalize_remote_urls(values: Vec<String>) -> Result<Vec<String>, AppError> {
    let mut normalized = Vec::new();
    for (index, raw) in values.into_iter().enumerate() {
        let value = raw.trim();
        let parsed = Url::parse(value).map_err(|error| {
            AppError::usage(
                "invalid_remote_url",
                format!(
                    "allowedRemoteUrls entry #{} is not a valid HTTP(S) origin: {error}",
                    index + 1
                ),
            )
        })?;
        let valid_scheme = matches!(parsed.scheme(), "http" | "https");
        let valid_authority = parsed.host_str().is_some()
            && parsed.username().is_empty()
            && parsed.password().is_none();
        let valid_origin = parsed.path() == "/"
            && parsed.query().is_none()
            && parsed.fragment().is_none()
            && !value.contains(';');
        if !valid_scheme || !valid_authority || !valid_origin {
            return Err(AppError::usage(
                "invalid_remote_url",
                format!(
                    "allowedRemoteUrls entry #{} must be an HTTP(S) origin without a non-root path, credentials, query, fragment, or `;`",
                    index + 1
                ),
            ));
        }
        let value = parsed.to_string();
        if !normalized.contains(&value) {
            normalized.push(value);
        }
    }
    Ok(normalized)
}

#[derive(Clone, Copy)]
enum ConfigScope {
    Project,
    User,
}

fn read_config(path: &Path, scope: ConfigScope) -> Result<RawConfig, AppError> {
    let contents = fs::read_to_string(path).map_err(|error| {
        AppError::usage(
            "config_read",
            format!("failed to read {}: {error}", path.display()),
        )
    })?;
    let value: toml::Value = toml::from_str(&contents).map_err(|error| {
        AppError::usage(
            "config_parse",
            format!("invalid {}: {error}", path.display()),
        )
    })?;

    if matches!(scope, ConfigScope::Project) {
        if let Some(key) = find_forbidden_project_key(&value) {
            return Err(AppError::usage(
                "nonportable_project_config",
                format!(
                    "`{key}` in {} is machine-specific; executable, jar, and download URL settings belong in user config or CLI",
                    path.display()
                ),
            ));
        }
    }

    let config = value.try_into::<RawConfig>().map_err(|error| {
        AppError::usage(
            "config_schema",
            format!("invalid {}: {error}", path.display()),
        )
    })?;
    if matches!(scope, ConfigScope::Project) {
        validate_project_paths(path, &config)?;
    }
    Ok(config)
}

fn validate_project_paths(path: &Path, config: &RawConfig) -> Result<(), AppError> {
    if let Some(out_dir) = &config.out_dir {
        validate_portable_project_path(path, "outDir", out_dir)?;
    }
    if let Some(include_paths) = &config.include_paths {
        for include_path in include_paths {
            validate_portable_project_path(path, "includePaths", include_path)?;
        }
    }
    Ok(())
}

fn validate_portable_project_path(
    config_path: &Path,
    key: &str,
    value: &Path,
) -> Result<(), AppError> {
    use std::path::Component;

    let display = value.to_string_lossy();
    let has_windows_prefix = display.as_bytes().get(1) == Some(&b':')
        && display
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphabetic);
    let unsafe_component = value.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    });
    if display.is_empty()
        || value.is_absolute()
        || has_windows_prefix
        || display.contains('\\')
        || unsafe_component
    {
        return Err(AppError::usage(
            "nonportable_project_path",
            format!(
                "`{key}` in {} must be a portable project-relative path without `..`: {}",
                config_path.display(),
                value.display()
            ),
        ));
    }
    Ok(())
}

pub fn validate_portable_workspace_paths(
    label: &str,
    key: &str,
    values: &[PathBuf],
) -> Result<(), AppError> {
    for value in values {
        validate_portable_project_path(Path::new(label), key, value)?;
    }
    Ok(())
}

fn find_forbidden_project_key(value: &toml::Value) -> Option<String> {
    find_key(value, &|key, _| {
        let normalized = normalize_key(key);
        matches!(
            normalized.as_str(),
            "javapath"
                | "javabinary"
                | "binarypath"
                | "plantumlbinary"
                | "jarpath"
                | "plantumljar"
                | "graphvizpath"
                | "graphvizdot"
                | "downloadurl"
                | "rendererurl"
                | "plantumlurl"
                | "allowedremoteurls"
        )
        .then(|| key.to_string())
    })
}

fn find_key(
    value: &toml::Value,
    predicate: &impl Fn(&str, &toml::Value) -> Option<String>,
) -> Option<String> {
    match value {
        toml::Value::Table(table) => table
            .iter()
            .find_map(|(key, value)| predicate(key, value).or_else(|| find_key(value, predicate))),
        toml::Value::Array(values) => values.iter().find_map(|value| find_key(value, predicate)),
        _ => None,
    }
}

fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(|character| !matches!(character, '_' | '-'))
        .flat_map(char::to_lowercase)
        .collect()
}

fn resolve_from(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn canonical_directory(path: &Path, label: &str) -> Result<PathBuf, AppError> {
    let canonical = path.canonicalize().map_err(|error| {
        AppError::usage(
            "path_not_found",
            format!("{label} does not exist at {}: {error}", path.display()),
        )
    })?;
    if !canonical.is_dir() {
        return Err(AppError::usage(
            "path_not_directory",
            format!("{label} is not a directory: {}", canonical.display()),
        ));
    }
    Ok(canonical)
}

fn canonical_file(path: &Path, label: &str) -> Result<PathBuf, AppError> {
    let canonical = path.canonicalize().map_err(|error| {
        AppError::usage(
            "config_not_found",
            format!("{label} does not exist at {}: {error}", path.display()),
        )
    })?;
    if !canonical.is_file() {
        return Err(AppError::usage(
            "config_not_file",
            format!("{label} is not a file: {}", canonical.display()),
        ));
    }
    Ok(canonical)
}

fn find_git_root(cwd: &Path) -> Option<PathBuf> {
    cwd.ancestors()
        .find(|candidate| candidate.join(".git").exists())
        .map(Path::to_path_buf)
}

pub fn default_user_config_path() -> Option<PathBuf> {
    if cfg!(windows) {
        env::var_os("APPDATA")
            .map(PathBuf::from)
            .map(|path| path.join("plantuml-export/config.toml"))
    } else {
        env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .map(|path| path.join("plantuml-export/config.toml"))
    }
}
