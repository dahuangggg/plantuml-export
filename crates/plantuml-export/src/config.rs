use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cli::{Cli, Command, Layout, OutputFormat, RendererMode, SecurityProfile};
use crate::AppError;

pub const PROJECT_CONFIG_NAME: &str = "plantuml-export.toml";

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConfigOverrides {
    pub renderer: Option<RendererMode>,
    pub format: Option<OutputFormat>,
    pub out_dir: Option<PathBuf>,
    pub layout: Option<Layout>,
    pub security: Option<SecurityProfile>,
    pub embed_source_metadata: Option<bool>,
    pub include_paths: Option<Vec<PathBuf>>,
    pub include: Option<Vec<String>>,
    pub exclude: Option<Vec<String>>,
    pub offline: Option<bool>,
    pub java_path: Option<PathBuf>,
    pub binary_path: Option<PathBuf>,
    pub jar_path: Option<PathBuf>,
}

impl ConfigOverrides {
    pub fn from_cli(cli: &Cli) -> Self {
        let Command::Export(export) = &cli.command else {
            return Self::default();
        };

        Self {
            renderer: export.renderer,
            format: export.format,
            out_dir: export.out_dir.clone(),
            layout: export.layout,
            security: export.security,
            embed_source_metadata: export
                .embed_source_metadata
                .then_some(true)
                .or_else(|| export.disable_metadata.then_some(false)),
            include_paths: (!export.include_paths.is_empty()).then(|| export.include_paths.clone()),
            include: (!export.include.is_empty()).then(|| export.include.clone()),
            exclude: (!export.exclude.is_empty()).then(|| export.exclude.clone()),
            offline: export.offline.then_some(true),
            java_path: export.java_path.clone(),
            binary_path: export.binary_path.clone(),
            jar_path: export.jar_path.clone(),
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
    pub security: SecurityProfile,
    pub embed_source_metadata: bool,
    pub include_paths: Vec<PathBuf>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub offline: bool,
    pub remote_includes: bool,
    pub java_path: PathBuf,
    pub binary_path: Option<PathBuf>,
    pub jar_path: Option<PathBuf>,
}

impl ResolvedConfig {
    fn defaults(root: PathBuf) -> Self {
        Self {
            root,
            project_config: None,
            user_config: None,
            renderer: RendererMode::Managed,
            format: OutputFormat::Svg,
            out_dir: PathBuf::from("out/plantuml"),
            layout: Layout::Graphviz,
            security: SecurityProfile::Allowlist,
            embed_source_metadata: false,
            include_paths: Vec::new(),
            include: Vec::new(),
            exclude: Vec::new(),
            offline: false,
            remote_includes: false,
            java_path: PathBuf::from("java"),
            binary_path: None,
            jar_path: None,
        }
    }

    fn apply(&mut self, input: RawConfig, allow_local_tools: bool) {
        apply_option(&mut self.renderer, input.renderer);
        apply_option(&mut self.format, input.format);
        apply_option(&mut self.out_dir, input.out_dir);
        apply_option(&mut self.layout, input.layout);
        apply_option(&mut self.security, input.security);
        apply_option(&mut self.embed_source_metadata, input.embed_source_metadata);
        apply_option(&mut self.include_paths, input.include_paths);
        apply_option(&mut self.include, input.include);
        apply_option(&mut self.exclude, input.exclude);
        apply_option(&mut self.offline, input.offline);
        apply_option(&mut self.remote_includes, input.remote_includes);

        if allow_local_tools {
            apply_option(&mut self.java_path, input.java_path);
            apply_option(&mut self.binary_path, input.binary_path.map(Some));
            apply_option(&mut self.jar_path, input.jar_path.map(Some));
        }
    }

    fn apply_overrides(&mut self, input: ConfigOverrides) {
        apply_option(&mut self.renderer, input.renderer);
        apply_option(&mut self.format, input.format);
        apply_option(&mut self.out_dir, input.out_dir);
        apply_option(&mut self.layout, input.layout);
        apply_option(&mut self.security, input.security);
        apply_option(&mut self.embed_source_metadata, input.embed_source_metadata);
        apply_option(&mut self.include_paths, input.include_paths);
        apply_option(&mut self.include, input.include);
        apply_option(&mut self.exclude, input.exclude);
        apply_option(&mut self.offline, input.offline);
        apply_option(&mut self.java_path, input.java_path);
        apply_option(&mut self.binary_path, input.binary_path.map(Some));
        apply_option(&mut self.jar_path, input.jar_path.map(Some));
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
    security: Option<SecurityProfile>,
    #[serde(rename = "embedSourceMetadata", alias = "embed_source_metadata")]
    embed_source_metadata: Option<bool>,
    #[serde(rename = "includePaths", alias = "include_paths")]
    include_paths: Option<Vec<PathBuf>>,
    include: Option<Vec<String>>,
    exclude: Option<Vec<String>>,
    offline: Option<bool>,
    #[serde(rename = "remoteIncludes", alias = "remote_includes")]
    remote_includes: Option<bool>,
    #[serde(rename = "javaPath", alias = "java_path")]
    java_path: Option<PathBuf>,
    #[serde(rename = "binaryPath", alias = "binary_path")]
    binary_path: Option<PathBuf>,
    #[serde(rename = "jarPath", alias = "jar_path")]
    jar_path: Option<PathBuf>,
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
    Ok(resolved)
}

fn apply_option<T>(target: &mut T, value: Option<T>) {
    if let Some(value) = value {
        *target = value;
    }
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

    if let Some(key) = find_legacy_key(&value) {
        return Err(AppError::usage(
            "legacy_config",
            format!(
                "legacy unreleased configuration `{key}` in {}; migrate to the v0.1 plantuml-export.toml schema",
                path.display()
            ),
        ));
    }

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

    value.try_into::<RawConfig>().map_err(|error| {
        AppError::usage(
            "config_schema",
            format!("invalid {}: {error}", path.display()),
        )
    })
}

fn find_legacy_key(value: &toml::Value) -> Option<String> {
    find_key(value, &|key, value| {
        let normalized = normalize_key(key);
        if matches!(
            normalized.as_str(),
            "autodownloadjar" | "defaultformat" | "plantumlversion"
        ) || (normalized == "renderer" && value.as_str() == Some("auto"))
        {
            Some(key.to_string())
        } else {
            None
        }
    })
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
                | "downloadurl"
                | "rendererurl"
                | "plantumlurl"
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
