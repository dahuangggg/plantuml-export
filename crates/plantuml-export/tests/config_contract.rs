use std::fs;
use std::path::{Path, PathBuf};

use clap::Parser;
use plantuml_export::cli::{Cli, Layout, OutputFormat, RendererMode};
use plantuml_export::config::{resolve, ConfigOverrides, ConfigRequest, RemoteIncludes};
use tempfile::TempDir;

#[test]
fn defaults_are_managed_svg_smetana_with_public_remote_includes() {
    let fixture = RepoFixture::new();

    let resolved = resolve(fixture.request(ConfigOverrides::default())).unwrap();

    assert_eq!(resolved.root, fixture.root());
    assert_eq!(resolved.renderer, RendererMode::Managed);
    assert_eq!(resolved.format, OutputFormat::Svg);
    assert_eq!(resolved.out_dir, PathBuf::from("out"));
    assert_eq!(resolved.layout, Layout::Smetana);
    assert_eq!(resolved.remote_includes, RemoteIncludes::Public);
    assert!(resolved.allowed_remote_urls.is_empty());
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
fn local_include_paths_merge_across_user_project_and_cli_scopes() {
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

    let cli = Cli::try_parse_from([
        "plantuml-export",
        "export",
        "--include-path",
        "/cli/includes",
    ])
    .unwrap();
    let resolved = resolve(fixture.request(ConfigOverrides::from_cli(&cli))).unwrap();

    assert_eq!(resolved.include, vec!["docs/**/*.puml"]);
    assert_eq!(
        resolved.include_paths,
        vec![
            PathBuf::from("/user/includes"),
            PathBuf::from("shared/includes"),
            PathBuf::from("/cli/includes"),
        ]
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
fn project_config_rejects_machine_and_network_trust_roots() {
    for forbidden in [
        "java_path = \"/usr/bin/java\"",
        "binary_path = \"/usr/bin/plantuml\"",
        "jar_path = \"vendor/plantuml.jar\"",
        "graphviz_path = \"/usr/bin/dot\"",
        "download_url = \"https://example.test/plantuml.jar\"",
        "allowedRemoteUrls = [\"http://plantuml.internal:8080/\"]",
    ] {
        let fixture = RepoFixture::new();
        fixture.write_project(forbidden);

        let error = resolve(fixture.request(ConfigOverrides::default())).unwrap_err();

        assert_eq!(error.exit_code(), 2);
        assert!(error.to_string().contains("user config or CLI"));
    }
}

#[test]
fn project_config_can_tighten_but_never_widen_machine_network_policy() {
    let fixture = RepoFixture::new();
    fixture.write_user("remoteIncludes = \"disabled\"\noffline = true\n");
    fixture.write_project("remoteIncludes = \"public\"\noffline = false\n");

    let resolved = resolve(fixture.request(ConfigOverrides::default())).unwrap();

    assert_eq!(resolved.remote_includes, RemoteIncludes::Disabled);
    assert!(resolved.offline);

    let fixture = RepoFixture::new();
    fixture.write_project("remoteIncludes = \"allowlist\"\noffline = true\n");
    let resolved = resolve(fixture.request(ConfigOverrides::default())).unwrap();
    assert_eq!(resolved.remote_includes, RemoteIncludes::Allowlist);
    assert!(resolved.offline);
}

#[test]
fn project_paths_are_portable_relative_and_never_traverse() {
    for contents in [
        "outDir = \"/tmp/generated\"\n",
        "outDir = \"../generated\"\n",
        "outDir = \"C:\\\\generated\"\n",
        "includePaths = [\"../shared\"]\n",
        "includePaths = [\"/tmp/shared\"]\n",
        "includePaths = [\"C:\\\\shared\"]\n",
    ] {
        let fixture = RepoFixture::new();
        fixture.write_project(contents);

        let error = resolve(fixture.request(ConfigOverrides::default())).unwrap_err();

        assert_eq!(error.code, "nonportable_project_path", "{contents}");
    }

    let fixture = RepoFixture::new();
    fixture.write_project("outDir = \"build/diagrams\"\nincludePaths = [\"docs/includes\"]\n");
    let resolved = resolve(fixture.request(ConfigOverrides::default())).unwrap();
    assert_eq!(resolved.out_dir, PathBuf::from("build/diagrams"));
    assert_eq!(resolved.include_paths, vec![PathBuf::from("docs/includes")]);
}

#[test]
fn graphviz_path_is_user_only_and_cli_has_highest_precedence() {
    let fixture = RepoFixture::new();
    fixture.write_user("graphvizPath = \"user-dot\"\n");
    let overrides = ConfigOverrides {
        graphviz_path: Some(PathBuf::from("cli-dot")),
        ..ConfigOverrides::default()
    };

    let resolved = resolve(fixture.request(overrides)).unwrap();

    assert_eq!(resolved.graphviz_path, PathBuf::from("cli-dot"));
}

#[test]
fn user_config_can_select_remote_policy_and_trusted_url_roots() {
    let fixture = RepoFixture::new();
    fixture.write_user(
        r#"
remoteIncludes = "allowlist"
allowedRemoteUrls = [
  "HTTP://PLANTUML.INTERNAL:8080/",
  "https://example.com"
]
"#,
    );

    let resolved = resolve(fixture.request(ConfigOverrides::default())).unwrap();

    assert_eq!(resolved.remote_includes, RemoteIncludes::Allowlist);
    assert_eq!(
        resolved.allowed_remote_urls,
        ["http://plantuml.internal:8080/", "https://example.com/",]
    );
}

#[test]
fn trusted_remote_url_roots_are_validated_as_whole_origins() {
    for invalid in [
        "relative/common/",
        "file:///tmp/common/",
        "https://user@example.com/common/",
        "https://example.com/common",
        "https://example.com/common/",
        "https://example.com/common/?token=secret",
        "https://example.com/common/#fragment",
        "https://example.com/common/;https://evil.test/",
    ] {
        let fixture = RepoFixture::new();
        fixture.write_user(&format!("allowedRemoteUrls = [{invalid:?}]\n"));

        let error = resolve(fixture.request(ConfigOverrides::default())).unwrap_err();

        assert_eq!(error.code, "invalid_remote_url", "{invalid}");
    }

    let fixture = RepoFixture::new();
    fixture
        .write_user("allowedRemoteUrls = [\"https://user:secret@example.com/?token=private\"]\n");
    let error = resolve(fixture.request(ConfigOverrides::default())).unwrap_err();
    assert!(!error.message.contains("secret"));
    assert!(!error.message.contains("private"));
}

#[test]
fn unreleased_boolean_remote_includes_and_security_keys_are_not_migrated() {
    for stale in ["remoteIncludes = true", "security = \"allowlist\""] {
        let fixture = RepoFixture::new();
        fixture.write_user(stale);

        let error = resolve(fixture.request(ConfigOverrides::default())).unwrap_err();

        assert_eq!(error.code, "config_schema", "{stale}");
    }
}

#[test]
fn unknown_configuration_keys_are_rejected_by_the_current_schema() {
    for unknown in [
        "renderer = \"auto\"",
        "autoDownloadJar = true",
        "defaultFormat = \"png\"",
    ] {
        let fixture = RepoFixture::new();
        fixture.write_project(unknown);

        let error = resolve(fixture.request(ConfigOverrides::default())).unwrap_err();

        assert_eq!(error.exit_code(), 2);
        assert_eq!(error.code, "config_schema");
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
