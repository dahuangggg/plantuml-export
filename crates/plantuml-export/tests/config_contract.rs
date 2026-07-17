use std::fs;
use std::path::{Path, PathBuf};

use plantuml_export::cli::{Layout, OutputFormat, RendererMode, SecurityProfile};
use plantuml_export::config::{resolve, ConfigOverrides, ConfigRequest};
use tempfile::TempDir;

#[test]
fn defaults_are_managed_svg_graphviz_allowlist_without_metadata() {
    let fixture = RepoFixture::new();

    let resolved = resolve(fixture.request(ConfigOverrides::default())).unwrap();

    assert_eq!(resolved.root, fixture.root());
    assert_eq!(resolved.renderer, RendererMode::Managed);
    assert_eq!(resolved.format, OutputFormat::Svg);
    assert_eq!(resolved.out_dir, PathBuf::from("out/plantuml"));
    assert_eq!(resolved.layout, Layout::Graphviz);
    assert_eq!(resolved.security, SecurityProfile::Allowlist);
    assert!(!resolved.embed_source_metadata);
}

#[test]
fn cli_overrides_project_which_overrides_user_which_overrides_defaults() {
    let fixture = RepoFixture::new();
    fixture.write_user(
        r#"
format = "png"
out_dir = "user-output"
layout = "smetana"
java_path = "/user/bin/java"
"#,
    );
    fixture.write_project(
        r#"
format = "pdf"
out_dir = "project-output"
layout = "graphviz"
"#,
    );
    let overrides = ConfigOverrides {
        format: Some(OutputFormat::Svg),
        out_dir: Some(PathBuf::from("cli-output")),
        include: Some(vec!["cli/**/*.puml".to_string()]),
        ..ConfigOverrides::default()
    };

    let resolved = resolve(fixture.request(overrides)).unwrap();

    assert_eq!(resolved.format, OutputFormat::Svg);
    assert_eq!(resolved.out_dir, PathBuf::from("cli-output"));
    assert_eq!(resolved.layout, Layout::Graphviz);
    assert_eq!(resolved.java_path, PathBuf::from("/user/bin/java"));
    assert_eq!(resolved.include, vec!["cli/**/*.puml"]);
}

#[test]
fn include_file_globs_are_distinct_from_local_include_paths() {
    let fixture = RepoFixture::new();
    fixture.write_user(
        r#"
include = ["user/**/*.puml"]
includePaths = ["/user/includes"]
"#,
    );
    fixture.write_project(
        r#"
include = ["docs/**/*.puml"]
includePaths = ["shared/includes"]
"#,
    );

    let resolved = resolve(fixture.request(ConfigOverrides::default())).unwrap();

    assert_eq!(resolved.include, vec!["docs/**/*.puml"]);
    assert_eq!(
        resolved.include_paths,
        vec![PathBuf::from("shared/includes")]
    );
}

#[test]
fn discovers_exactly_one_project_config_at_the_git_root() {
    let fixture = RepoFixture::new();
    let nested = fixture.root().join("docs/diagrams");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("plantuml-export.toml"), "format = \"png\"\n").unwrap();
    fixture.write_project("format = \"pdf\"\n");
    let mut request = fixture.request(ConfigOverrides::default());
    request.cwd = nested;

    let resolved = resolve(request).unwrap();

    assert_eq!(resolved.root, fixture.root());
    assert_eq!(resolved.format, OutputFormat::Pdf);
    assert_eq!(
        resolved.project_config,
        Some(fixture.root().join("plantuml-export.toml"))
    );
}

#[test]
fn explicit_root_and_config_are_resolved_relative_to_the_current_directory() {
    let fixture = RepoFixture::new();
    let cwd = fixture.temp.path().join("shell");
    let root = fixture.temp.path().join("selected-root");
    fs::create_dir_all(&cwd).unwrap();
    fs::create_dir_all(&root).unwrap();
    fs::write(cwd.join("selected.toml"), "format = \"png\"\n").unwrap();
    let mut request = fixture.request(ConfigOverrides::default());
    request.cwd = cwd.clone();
    request.root = Some(PathBuf::from("../selected-root"));
    request.config = Some(PathBuf::from("selected.toml"));

    let resolved = resolve(request).unwrap();

    assert_eq!(resolved.root, root.canonicalize().unwrap());
    assert_eq!(resolved.format, OutputFormat::Png);
    assert_eq!(
        resolved.project_config,
        Some(cwd.join("selected.toml").canonicalize().unwrap())
    );
}

#[test]
fn project_config_rejects_executable_jar_and_download_url_keys() {
    for forbidden in [
        "java_path = \"/usr/bin/java\"",
        "binary_path = \"/usr/bin/plantuml\"",
        "jar_path = \"vendor/plantuml.jar\"",
        "download_url = \"https://example.test/plantuml.jar\"",
    ] {
        let fixture = RepoFixture::new();
        fixture.write_project(forbidden);

        let error = resolve(fixture.request(ConfigOverrides::default())).unwrap_err();

        assert_eq!(error.exit_code(), 2);
        assert!(error.to_string().contains("user config or CLI"));
    }
}

#[test]
fn legacy_unreleased_keys_return_explicit_migration_errors() {
    for legacy in [
        "renderer = \"auto\"",
        "autoDownloadJar = true",
        "defaultFormat = \"png\"",
    ] {
        let fixture = RepoFixture::new();
        fixture.write_project(legacy);

        let error = resolve(fixture.request(ConfigOverrides::default())).unwrap_err();

        assert_eq!(error.exit_code(), 2);
        assert!(error
            .to_string()
            .contains("legacy unreleased configuration"));
        assert!(error.to_string().contains("plantuml-export.toml"));
    }
}

struct RepoFixture {
    temp: TempDir,
    user_config: PathBuf,
}

impl RepoFixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("repo/.git")).unwrap();
        Self {
            user_config: temp.path().join("user/config.toml"),
            temp,
        }
    }

    fn root(&self) -> PathBuf {
        self.temp.path().join("repo").canonicalize().unwrap()
    }

    fn write_project(&self, contents: &str) {
        fs::write(self.temp.path().join("repo/plantuml-export.toml"), contents).unwrap();
    }

    fn write_user(&self, contents: &str) {
        fs::create_dir_all(self.user_config.parent().unwrap()).unwrap();
        fs::write(&self.user_config, contents).unwrap();
    }

    fn request(&self, overrides: ConfigOverrides) -> ConfigRequest {
        ConfigRequest {
            cwd: self.root(),
            root: None,
            config: None,
            user_config: Some(self.user_config.clone()),
            overrides,
        }
    }
}

#[allow(dead_code)]
fn assert_path(_: &Path) {}
