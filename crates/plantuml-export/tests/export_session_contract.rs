use std::fs;
use std::path::{Path, PathBuf};

use plantuml_export::cli::OutputFormat;
use plantuml_export::discovery::{discover_inputs, DiscoveryOptions};
use plantuml_export::export::{
    EnvironmentMetadata, ExportErrorKind, ExportRequest, ExportSession, Renderer, RendererError,
    RendererMetadata,
};
use tempfile::TempDir;

#[derive(Default)]
struct FakeRenderer;

impl Renderer for FakeRenderer {
    fn render(
        &self,
        input: &Path,
        staging_dir: &Path,
        format: OutputFormat,
    ) -> Result<(), RendererError> {
        let source = fs::read_to_string(input).map_err(|error| {
            RendererError::new("fake_read", format!("failed to read fake input: {error}"))
        })?;
        if source.contains("RENDER_FAIL") {
            return Err(RendererError::new("fake_failure", "requested fake failure"));
        }
        if source.contains("ENVIRONMENT_FAIL") {
            return Err(RendererError::environment(
                "fake_environment",
                "requested environment failure",
            ));
        }

        let stem = input.file_stem().expect("input stem").to_string_lossy();
        let extension = format.extension();
        let first = staging_dir.join(format!("{stem}.{extension}"));
        fs::create_dir_all(staging_dir).expect("fake staging dir");
        if source.contains("ZERO_BYTE") {
            fs::write(first, []).expect("zero-byte output");
            return Ok(());
        }

        fs::write(&first, fake_bytes(format)).expect("fake output");
        if source.contains("MULTI") {
            fs::write(
                staging_dir.join(format!("{stem}_001.{extension}")),
                fake_bytes(format),
            )
            .expect("second fake output");
        }
        Ok(())
    }

    fn validate(&self, output: &Path, format: OutputFormat) -> Result<(), RendererError> {
        let bytes = fs::read(output).map_err(|error| {
            RendererError::new("fake_validate_read", format!("failed to validate: {error}"))
        })?;
        if bytes != fake_bytes(format) {
            return Err(RendererError::new(
                "fake_type",
                "unexpected fake output type",
            ));
        }
        Ok(())
    }

    fn metadata(&self) -> RendererMetadata {
        RendererMetadata {
            mode: "fake".into(),
            version: "test-1".into(),
        }
    }
}

struct ProvenanceRenderer {
    metadata: RendererMetadata,
    marker: &'static str,
}

struct StagingBoundaryRenderer {
    out_dir: PathBuf,
    state_dir: PathBuf,
}

impl Renderer for StagingBoundaryRenderer {
    fn render(
        &self,
        input: &Path,
        staging_dir: &Path,
        format: OutputFormat,
    ) -> Result<(), RendererError> {
        let out_dir = fs::canonicalize(&self.out_dir).expect("canonical output directory");
        let state_dir = fs::canonicalize(&self.state_dir).expect("canonical state directory");
        assert!(staging_dir.starts_with(&state_dir));
        assert!(!staging_dir.starts_with(&out_dir));
        FakeRenderer.render(input, staging_dir, format)
    }

    fn validate(&self, output: &Path, format: OutputFormat) -> Result<(), RendererError> {
        FakeRenderer.validate(output, format)
    }

    fn metadata(&self) -> RendererMetadata {
        FakeRenderer.metadata()
    }
}

impl Renderer for ProvenanceRenderer {
    fn render(
        &self,
        input: &Path,
        staging_dir: &Path,
        format: OutputFormat,
    ) -> Result<(), RendererError> {
        FakeRenderer.render(input, staging_dir, format)?;
        for entry in fs::read_dir(staging_dir).expect("provenance staging directory") {
            let path = entry.expect("provenance staged entry").path();
            fs::write(path, provenance_bytes(format, self.marker))
                .expect("provenance-marked output");
        }
        Ok(())
    }

    fn validate(&self, output: &Path, format: OutputFormat) -> Result<(), RendererError> {
        let bytes = fs::read(output).map_err(|error| {
            RendererError::new("provenance_read", format!("failed to validate: {error}"))
        })?;
        if bytes != provenance_bytes(format, self.marker) {
            return Err(RendererError::new(
                "provenance_type",
                "unexpected provenance-marked output",
            ));
        }
        Ok(())
    }

    fn metadata(&self) -> RendererMetadata {
        self.metadata.clone()
    }
}

fn provenance_bytes(format: OutputFormat, marker: &str) -> Vec<u8> {
    match format {
        OutputFormat::Svg => {
            format!(r#"<svg xmlns="http://www.w3.org/2000/svg"><!-- {marker} --></svg>"#)
                .into_bytes()
        }
        OutputFormat::Png => [b"\x89PNG\r\n\x1a\n".as_slice(), marker.as_bytes()].concat(),
        OutputFormat::Pdf => [b"%PDF-1.7\n".as_slice(), marker.as_bytes()].concat(),
    }
}

fn fake_bytes(format: OutputFormat) -> &'static [u8] {
    match format {
        OutputFormat::Svg => b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>",
        OutputFormat::Png => b"\x89PNG\r\n\x1a\nfixture",
        OutputFormat::Pdf => b"%PDF-1.7\nfixture",
    }
}

fn assert_manifest_outputs(value: &serde_json::Value, expected_paths: &[&str]) {
    let outputs = value.as_array().expect("manifest output array");
    assert_eq!(outputs.len(), expected_paths.len());
    for (output, expected_path) in outputs.iter().zip(expected_paths) {
        assert_eq!(output["path"], *expected_path);
        let sha256 = output["sha256"].as_str().expect("output SHA-256");
        assert_eq!(sha256.len(), 64);
        assert!(sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
    }
}

fn write(path: impl Into<PathBuf>, contents: impl AsRef<[u8]>) {
    let path = path.into();
    fs::create_dir_all(path.parent().expect("fixture parent")).expect("fixture parent");
    fs::write(path, contents).expect("write fixture");
}

fn discovered(
    project: &TempDir,
    paths: &[&str],
) -> Vec<plantuml_export::discovery::DiscoveredInput> {
    let mut options = DiscoveryOptions::new(project.path());
    options.inputs = paths.iter().map(PathBuf::from).collect();
    discover_inputs(&options).expect("discover explicit inputs")
}

fn request(project: &TempDir, paths: &[&str]) -> ExportRequest {
    ExportRequest {
        root: project.path().to_path_buf(),
        out_dir: PathBuf::from("out"),
        state_dir: project.path().join(".test-export-state"),
        inputs: discovered(project, paths),
        format: OutputFormat::Svg,
        keep_going: false,
        tool_version: "0.1.0-test".into(),
        environment: EnvironmentMetadata {
            java_version: Some("21-test".into()),
            graphviz_version: Some("test".into()),
            os: "test-os".into(),
            architecture: "test-arch".into(),
        },
    }
}

#[test]
fn validates_and_commits_each_supported_output_format() {
    for format in [OutputFormat::Svg, OutputFormat::Png, OutputFormat::Pdf] {
        let project = TempDir::new().expect("project");
        write(project.path().join("diagram.puml"), "diagram");
        let mut export = request(&project, &["diagram.puml"]);
        export.format = format;

        ExportSession::new(FakeRenderer)
            .run(export)
            .expect("format export");

        assert!(project
            .path()
            .join(format!("out/diagram.{}", format.extension()))
            .is_file());
    }
}

#[test]
fn preserves_outputs_and_provenance_from_other_formats_for_the_same_input() {
    let project = TempDir::new().expect("project");
    write(project.path().join("diagram.puml"), "diagram");

    let svg_renderer = ProvenanceRenderer {
        metadata: RendererMetadata {
            mode: "renderer-svg".into(),
            version: "1-svg".into(),
        },
        marker: "render-svg",
    };
    let mut svg = request(&project, &["diagram.puml"]);
    svg.tool_version = "tool-svg".into();
    svg.environment.java_version = Some("java-svg".into());
    ExportSession::new(svg_renderer)
        .run(svg)
        .expect("SVG export");

    let png_renderer = ProvenanceRenderer {
        metadata: RendererMetadata {
            mode: "renderer-png".into(),
            version: "1-png".into(),
        },
        marker: "render-png",
    };
    let mut png = request(&project, &["diagram.puml"]);
    png.format = OutputFormat::Png;
    png.tool_version = "tool-png".into();
    png.environment.java_version = Some("java-png".into());
    ExportSession::new(png_renderer)
        .run(png)
        .expect("PNG export");

    assert_eq!(
        fs::read(project.path().join("out/diagram.svg")).expect("preserved SVG"),
        provenance_bytes(OutputFormat::Svg, "render-svg")
    );
    assert_eq!(
        fs::read(project.path().join("out/diagram.png")).expect("new PNG"),
        provenance_bytes(OutputFormat::Png, "render-png")
    );

    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(project.path().join(".test-export-state/manifest.json")).expect("manifest"),
    )
    .expect("manifest JSON");
    let formats = &manifest["inputs"]["diagram.puml"]["formats"];
    assert_manifest_outputs(&formats["svg"]["outputs"], &["out/diagram.svg"]);
    assert_eq!(formats["svg"]["toolVersion"], "tool-svg");
    assert_eq!(formats["svg"]["renderer"]["version"], "1-svg");
    assert_eq!(formats["svg"]["environment"]["javaVersion"], "java-svg");
    assert_manifest_outputs(&formats["png"]["outputs"], &["out/diagram.png"]);
    assert_eq!(formats["png"]["toolVersion"], "tool-png");
    assert_eq!(formats["png"]["renderer"]["version"], "1-png");
    assert_eq!(formats["png"]["environment"]["javaVersion"], "java-png");
}

#[test]
fn mirrors_unicode_paths_and_records_every_multi_output_in_the_manifest() {
    let project = TempDir::new().expect("project");
    write(project.path().join("文档/登录流程.puml"), "MULTI");

    let report = ExportSession::new(FakeRenderer)
        .run(request(&project, &["文档/登录流程.puml"]))
        .expect("multi-output export");

    assert_eq!(report.succeeded.len(), 1);
    assert_eq!(
        report.succeeded[0].outputs,
        ["out/文档/登录流程.svg", "out/文档/登录流程_001.svg"]
    );
    assert!(project.path().join("out/文档/登录流程.svg").is_file());
    assert!(project.path().join("out/文档/登录流程_001.svg").is_file());

    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(project.path().join(".test-export-state/manifest.json")).expect("manifest"),
    )
    .expect("manifest JSON");
    assert!(manifest.get("toolVersion").is_none());
    assert!(manifest.get("renderer").is_none());
    assert!(manifest.get("environment").is_none());
    assert!(manifest.get("format").is_none());
    let svg = &manifest["inputs"]["文档/登录流程.puml"]["formats"]["svg"];
    assert_eq!(svg["toolVersion"], "0.1.0-test");
    assert_eq!(svg["renderer"]["version"], "test-1");
    assert_eq!(svg["environment"]["javaVersion"], "21-test");
    assert_eq!(svg["environment"]["os"], "test-os");
    assert_manifest_outputs(
        &svg["outputs"],
        &["out/文档/登录流程.svg", "out/文档/登录流程_001.svg"],
    );
}

#[test]
fn default_session_rolls_back_every_input_when_one_renderer_fails() {
    let project = TempDir::new().expect("project");
    write(project.path().join("ok.puml"), "ok");
    write(project.path().join("bad.puml"), "RENDER_FAIL");
    write(
        project.path().join("out/notes.txt"),
        b"unrelated user artifact",
    );

    let error = ExportSession::new(FakeRenderer)
        .run(request(&project, &["ok.puml", "bad.puml"]))
        .expect_err("batch must fail");
    assert_eq!(error.kind, ExportErrorKind::InputFailure);
    assert_eq!(
        fs::read(project.path().join("out/notes.txt")).expect("unrelated file"),
        b"unrelated user artifact"
    );
    assert!(!project.path().join("out/ok.svg").exists());
    assert!(!project.path().join("out/bad.svg").exists());
    assert!(!project
        .path()
        .join(".test-export-state/manifest.json")
        .exists());
}

#[test]
fn environment_failure_aborts_the_batch_even_with_keep_going() {
    let project = TempDir::new().expect("project");
    write(project.path().join("ok.puml"), "ok");
    write(project.path().join("environment.puml"), "ENVIRONMENT_FAIL");
    let mut export = request(&project, &["ok.puml", "environment.puml"]);
    export.keep_going = true;

    let error = ExportSession::new(FakeRenderer)
        .run(export)
        .expect_err("shared renderer environment failure must abort the batch");

    assert_eq!(error.kind, ExportErrorKind::Environment);
    assert!(error.message.contains("requested environment failure"));
    assert!(!project.path().join("out/ok.svg").exists());
    assert!(!project
        .path()
        .join(".test-export-state/manifest.json")
        .exists());
}

#[test]
fn default_failure_preserves_the_previous_outputs_and_manifest_byte_for_byte() {
    let project = TempDir::new().expect("project");
    write(project.path().join("diagram.puml"), "MULTI");
    let session = ExportSession::new(FakeRenderer);
    session
        .run(request(&project, &["diagram.puml"]))
        .expect("initial export");
    let manifest_path = project.path().join(".test-export-state/manifest.json");
    let previous_manifest = fs::read(&manifest_path).expect("previous manifest");
    let previous_primary =
        fs::read(project.path().join("out/diagram.svg")).expect("previous primary");
    let previous_secondary =
        fs::read(project.path().join("out/diagram_001.svg")).expect("previous secondary");

    write(project.path().join("diagram.puml"), "changed");
    write(project.path().join("bad.puml"), "RENDER_FAIL");
    let error = session
        .run(request(&project, &["diagram.puml", "bad.puml"]))
        .expect_err("batch must fail without replacing prior state");

    assert_eq!(error.kind, ExportErrorKind::InputFailure);
    assert_eq!(
        fs::read(manifest_path).expect("preserved manifest"),
        previous_manifest
    );
    assert_eq!(
        fs::read(project.path().join("out/diagram.svg")).expect("preserved primary"),
        previous_primary
    );
    assert_eq!(
        fs::read(project.path().join("out/diagram_001.svg")).expect("preserved secondary"),
        previous_secondary
    );
}

#[test]
fn rejects_zero_byte_output_even_when_renderer_returns_success() {
    let project = TempDir::new().expect("project");
    write(project.path().join("empty.puml"), "ZERO_BYTE");

    let error = ExportSession::new(FakeRenderer)
        .run(request(&project, &["empty.puml"]))
        .expect_err("zero-byte output must fail");
    assert_eq!(error.kind, ExportErrorKind::OutputValidation);
    assert!(!project.path().join("out/empty.svg").exists());
}

#[test]
fn reports_colliding_sources_without_overwriting_anything() {
    let project = TempDir::new().expect("project");
    write(project.path().join("same.puml"), "first");
    write(project.path().join("same.wsd"), "second");

    let error = ExportSession::new(FakeRenderer)
        .run(request(&project, &["same.puml", "same.wsd"]))
        .expect_err("same output target must conflict");
    assert_eq!(error.kind, ExportErrorKind::OwnershipConflict);
    assert!(!project.path().join("out/same.svg").exists());
}

#[test]
fn refuses_to_overwrite_an_unmanaged_file_at_an_intended_target() {
    let project = TempDir::new().expect("project");
    write(project.path().join("diagram.puml"), "diagram");
    write(project.path().join("out/diagram.svg"), b"user-owned output");

    let error = ExportSession::new(FakeRenderer)
        .run(request(&project, &["diagram.puml"]))
        .expect_err("unmanaged target must conflict");
    assert_eq!(error.kind, ExportErrorKind::OwnershipConflict);
    assert_eq!(
        fs::read(project.path().join("out/diagram.svg")).expect("user-owned output"),
        b"user-owned output"
    );
}

#[test]
fn stale_external_manifest_cannot_overwrite_a_user_replacement_after_output_recreation() {
    let project = TempDir::new().expect("project");
    write(project.path().join("diagram.puml"), "diagram");
    let session = ExportSession::new(FakeRenderer);
    session
        .run(request(&project, &["diagram.puml"]))
        .expect("initial export");

    fs::remove_dir_all(project.path().join("out")).expect("remove old output tree");
    write(
        project.path().join("out/diagram.svg"),
        b"user-created replacement",
    );

    let error = session
        .run(request(&project, &["diagram.puml"]))
        .expect_err("stale external ownership must not overwrite replacement bytes");

    assert_eq!(error.kind, ExportErrorKind::OwnershipConflict);
    assert_eq!(
        fs::read(project.path().join("out/diagram.svg")).expect("replacement output"),
        b"user-created replacement"
    );
}

#[test]
fn removes_only_stale_outputs_owned_by_the_same_input() {
    let project = TempDir::new().expect("project");
    write(project.path().join("diagram.puml"), "MULTI");
    let session = ExportSession::new(FakeRenderer);
    session
        .run(request(&project, &["diagram.puml"]))
        .expect("initial multi export");
    write(
        project.path().join("out/notes.txt"),
        "unrelated user artifact",
    );

    write(project.path().join("diagram.puml"), "single");
    session
        .run(request(&project, &["diagram.puml"]))
        .expect("second single export");

    assert!(project.path().join("out/diagram.svg").is_file());
    assert!(!project.path().join("out/diagram_001.svg").exists());
    assert_eq!(
        fs::read_to_string(project.path().join("out/notes.txt")).expect("unrelated file"),
        "unrelated user artifact"
    );
}

#[test]
fn stale_cleanup_is_scoped_to_the_format_being_updated() {
    let project = TempDir::new().expect("project");
    write(project.path().join("diagram.puml"), "MULTI");
    let session = ExportSession::new(FakeRenderer);

    session
        .run(request(&project, &["diagram.puml"]))
        .expect("initial SVG export");
    let mut png = request(&project, &["diagram.puml"]);
    png.format = OutputFormat::Png;
    session.run(png).expect("initial PNG export");

    write(project.path().join("diagram.puml"), "single");
    session
        .run(request(&project, &["diagram.puml"]))
        .expect("updated SVG export");

    assert!(project.path().join("out/diagram.svg").is_file());
    assert!(!project.path().join("out/diagram_001.svg").exists());
    assert!(project.path().join("out/diagram.png").is_file());
    assert!(project.path().join("out/diagram_001.png").is_file());

    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(project.path().join(".test-export-state/manifest.json")).expect("manifest"),
    )
    .expect("manifest JSON");
    let formats = &manifest["inputs"]["diagram.puml"]["formats"];
    assert_manifest_outputs(&formats["svg"]["outputs"], &["out/diagram.svg"]);
    assert_manifest_outputs(
        &formats["png"]["outputs"],
        &["out/diagram.png", "out/diagram_001.png"],
    );
}

#[test]
fn partial_export_preserves_untouched_inputs_and_other_format_variants() {
    let project = TempDir::new().expect("project");
    write(project.path().join("a.puml"), "single");
    write(project.path().join("b.puml"), "MULTI");

    let renderer_a = ProvenanceRenderer {
        metadata: RendererMetadata {
            mode: "renderer-a".into(),
            version: "1-a".into(),
        },
        marker: "render-a",
    };
    let mut first = request(&project, &["a.puml", "b.puml"]);
    first.tool_version = "tool-a".into();
    first.environment = EnvironmentMetadata {
        java_version: Some("java-a".into()),
        graphviz_version: Some("graphviz-a".into()),
        os: "os-a".into(),
        architecture: "arch-a".into(),
    };
    ExportSession::new(renderer_a)
        .run(first)
        .expect("initial A/B export");

    let a_output = project.path().join("out/a.svg");
    let a_bytes = fs::read(&a_output).expect("initial A output");
    assert!(project.path().join("out/b.svg").is_file());
    assert!(project.path().join("out/b_001.svg").is_file());

    write(project.path().join("b.puml"), "single");
    let renderer_b = ProvenanceRenderer {
        metadata: RendererMetadata {
            mode: "renderer-b".into(),
            version: "2-b".into(),
        },
        marker: "render-b",
    };
    let mut second = request(&project, &["b.puml"]);
    second.format = OutputFormat::Png;
    second.tool_version = "tool-b".into();
    second.environment = EnvironmentMetadata {
        java_version: Some("java-b".into()),
        graphviz_version: None,
        os: "os-b".into(),
        architecture: "arch-b".into(),
    };
    ExportSession::new(renderer_b)
        .run(second)
        .expect("partial B export");

    assert_eq!(fs::read(a_output).expect("untouched A output"), a_bytes);
    assert_eq!(
        fs::read(project.path().join("out/b.svg")).expect("preserved B SVG"),
        provenance_bytes(OutputFormat::Svg, "render-a")
    );
    assert_eq!(
        fs::read(project.path().join("out/b_001.svg")).expect("preserved B secondary SVG"),
        provenance_bytes(OutputFormat::Svg, "render-a")
    );
    assert_eq!(
        fs::read(project.path().join("out/b.png")).expect("updated B output"),
        provenance_bytes(OutputFormat::Png, "render-b")
    );

    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(project.path().join(".test-export-state/manifest.json")).expect("manifest"),
    )
    .expect("manifest JSON");
    assert_eq!(manifest["schemaVersion"], 2);
    assert!(manifest.get("toolVersion").is_none());
    assert!(manifest.get("renderer").is_none());
    assert!(manifest.get("environment").is_none());
    assert!(manifest.get("format").is_none());

    let input_a = &manifest["inputs"]["a.puml"]["formats"]["svg"];
    assert_manifest_outputs(&input_a["outputs"], &["out/a.svg"]);
    assert_eq!(input_a["toolVersion"], "tool-a");
    assert_eq!(input_a["renderer"]["mode"], "renderer-a");
    assert_eq!(input_a["renderer"]["version"], "1-a");
    assert_eq!(input_a["environment"]["javaVersion"], "java-a");
    assert_eq!(input_a["environment"]["graphvizVersion"], "graphviz-a");
    assert_eq!(input_a["environment"]["os"], "os-a");
    assert_eq!(input_a["environment"]["architecture"], "arch-a");
    let input_b_svg = &manifest["inputs"]["b.puml"]["formats"]["svg"];
    assert_manifest_outputs(&input_b_svg["outputs"], &["out/b.svg", "out/b_001.svg"]);
    assert_eq!(input_b_svg["toolVersion"], "tool-a");
    assert_eq!(input_b_svg["renderer"]["version"], "1-a");

    let input_b_png = &manifest["inputs"]["b.puml"]["formats"]["png"];
    assert_manifest_outputs(&input_b_png["outputs"], &["out/b.png"]);
    assert_eq!(input_b_png["toolVersion"], "tool-b");
    assert_eq!(input_b_png["renderer"]["mode"], "renderer-b");
    assert_eq!(input_b_png["renderer"]["version"], "2-b");
    assert_eq!(input_b_png["environment"]["javaVersion"], "java-b");
    assert_eq!(
        input_b_png["environment"]["graphvizVersion"],
        serde_json::Value::Null
    );
    assert_eq!(input_b_png["environment"]["os"], "os-b");
    assert_eq!(input_b_png["environment"]["architecture"], "arch-b");
}

#[test]
fn keep_going_commits_independent_successes_and_reports_failures() {
    let project = TempDir::new().expect("project");
    write(project.path().join("good.puml"), "good");
    write(project.path().join("bad.puml"), "RENDER_FAIL");
    let mut export = request(&project, &["good.puml", "bad.puml"]);
    export.keep_going = true;

    let report = ExportSession::new(FakeRenderer)
        .run(export)
        .expect("partial result is reportable");
    assert!(report.is_partial());
    assert_eq!(report.succeeded.len(), 1);
    assert_eq!(report.failures.len(), 1);
    assert!(project.path().join("out/good.svg").is_file());
    assert!(!project.path().join("out/bad.svg").exists());
}

#[cfg(unix)]
#[test]
fn rejects_output_directory_symlinks_that_escape_the_root() {
    use std::os::unix::fs::symlink;

    let project = TempDir::new().expect("project");
    let outside = TempDir::new().expect("outside");
    write(project.path().join("diagram.puml"), "diagram");
    symlink(outside.path(), project.path().join("escaped-output")).expect("output symlink");
    let mut export = request(&project, &["diagram.puml"]);
    export.out_dir = PathBuf::from("escaped-output");

    let error = ExportSession::new(FakeRenderer)
        .run(export)
        .expect_err("escaped output must be rejected");
    assert_eq!(error.kind, ExportErrorKind::UnsafePath);
    assert!(fs::read_dir(outside.path())
        .expect("outside dir")
        .next()
        .is_none());
}

#[test]
fn rejects_lexical_relative_and_absolute_output_escapes() {
    let project = TempDir::new().expect("project");
    let outside = TempDir::new().expect("outside");
    write(project.path().join("diagram.puml"), "diagram");

    let mut relative_escape = request(&project, &["diagram.puml"]);
    relative_escape.out_dir = PathBuf::from("../escaped-output");
    let error = ExportSession::new(FakeRenderer)
        .run(relative_escape)
        .expect_err("relative traversal must be rejected");
    assert_eq!(error.kind, ExportErrorKind::UnsafePath);

    let mut absolute_escape = request(&project, &["diagram.puml"]);
    absolute_escape.out_dir = outside.path().join("absolute-output");
    let error = ExportSession::new(FakeRenderer)
        .run(absolute_escape)
        .expect_err("absolute outside path must be rejected");
    assert_eq!(error.kind, ExportErrorKind::UnsafePath);
    assert!(fs::read_dir(outside.path())
        .expect("outside dir")
        .next()
        .is_none());
}

#[test]
fn rejects_a_manifest_that_claims_files_outside_the_output_directory() {
    let project = TempDir::new().expect("project");
    write(project.path().join("diagram.puml"), "diagram");
    write(project.path().join("do-not-delete.txt"), "user-owned");
    write(
        project.path().join(".test-export-state/manifest.json"),
        br#"{
          "schemaVersion": 2,
          "inputs": {
            "diagram.puml": {"formats": {"svg": {
                "outputs": [{
                  "path": "do-not-delete.txt",
                  "sha256": "0000000000000000000000000000000000000000000000000000000000000000"
                }],
                "toolVersion": "forged",
                "renderer": {"mode": "fake", "version": "forged"},
                "environment": {
                  "javaVersion": null,
                  "graphvizVersion": null,
                  "os": "forged",
                  "architecture": "forged"
                }
            }}}
          }
        }"#,
    );

    let error = ExportSession::new(FakeRenderer)
        .run(request(&project, &["diagram.puml"]))
        .expect_err("unsafe manifest ownership must be rejected");

    assert_eq!(error.kind, ExportErrorKind::InvalidManifest);
    assert_eq!(
        fs::read_to_string(project.path().join("do-not-delete.txt")).expect("user file"),
        "user-owned"
    );
}

#[test]
fn rejects_removed_global_provenance_fields_in_schema_v2() {
    let project = TempDir::new().expect("project");
    write(project.path().join("diagram.puml"), "diagram");
    write(
        project.path().join(".test-export-state/manifest.json"),
        br#"{
          "schemaVersion": 2,
          "toolVersion": "misleading-global-value",
          "renderer": {"mode": "fake", "version": "forged"},
          "environment": {
            "javaVersion": null,
            "graphvizVersion": null,
            "os": "forged",
            "architecture": "forged"
          },
          "format": "svg",
          "inputs": {}
        }"#,
    );

    let error = ExportSession::new(FakeRenderer)
        .run(request(&project, &["diagram.puml"]))
        .expect_err("schema v2 must reject global provenance");

    assert_eq!(error.kind, ExportErrorKind::InvalidManifest);
    assert!(error.message.contains("unknown field `toolVersion`"));
}

#[test]
fn rejects_schema_v1_without_migration() {
    let project = TempDir::new().expect("project");
    write(project.path().join("diagram.puml"), "diagram");
    write(
        project.path().join(".test-export-state/manifest.json"),
        br#"{"schemaVersion": 1, "inputs": {}}"#,
    );

    let error = ExportSession::new(FakeRenderer)
        .run(request(&project, &["diagram.puml"]))
        .expect_err("development schema v1 must not be migrated");

    assert_eq!(error.kind, ExportErrorKind::InvalidManifest);
    assert!(error
        .message
        .contains("unsupported export manifest schemaVersion 1; expected 2"));
}

#[test]
fn rejects_unknown_one_format_manifest_fields() {
    let project = TempDir::new().expect("project");
    write(project.path().join("diagram.puml"), "diagram");
    write(
        project.path().join(".test-export-state/manifest.json"),
        br#"{
          "schemaVersion": 2,
          "inputs": {
            "diagram.puml": {
              "outputs": ["out/diagram.svg"],
              "toolVersion": "development-only",
              "renderer": {"mode": "fake", "version": "development-only"},
              "environment": {
                "javaVersion": null,
                "graphvizVersion": null,
                "os": "development-only",
                "architecture": "development-only"
              },
              "format": "svg"
            }
          }
        }"#,
    );

    let error = ExportSession::new(FakeRenderer)
        .run(request(&project, &["diagram.puml"]))
        .expect_err("unknown manifest fields must be rejected");

    assert_eq!(error.kind, ExportErrorKind::InvalidManifest);
    assert!(error.message.contains("unknown field `outputs`"));
}

#[test]
fn rejects_outputs_claimed_by_the_wrong_format_variant() {
    let project = TempDir::new().expect("project");
    write(project.path().join("diagram.puml"), "diagram");
    let manifest = serde_json::json!({
        "schemaVersion": 2,
        "inputs": {"diagram.puml": {"formats": {"png": {
            "outputs": [{
                "path": "out/diagram.svg",
                "sha256": "0".repeat(64)
            }],
            "toolVersion": "forged",
            "renderer": {"mode": "fake", "version": "forged"},
            "environment": {
                "javaVersion": null,
                "graphvizVersion": null,
                "os": "forged",
                "architecture": "forged"
            }
        }}}}
    });
    write(
        project.path().join(".test-export-state/manifest.json"),
        serde_json::to_vec_pretty(&manifest).expect("manifest JSON"),
    );

    let error = ExportSession::new(FakeRenderer)
        .run(request(&project, &["diagram.puml"]))
        .expect_err("cross-format ownership must fail closed");

    assert_eq!(error.kind, ExportErrorKind::InvalidManifest);
    assert!(error
        .message
        .contains("does not match its png format entry"));
}

#[test]
fn rejects_empty_per_input_provenance() {
    let project = TempDir::new().expect("project");
    write(project.path().join("diagram.puml"), "diagram");
    let manifest = serde_json::json!({
        "schemaVersion": 2,
        "inputs": {"diagram.puml": {"formats": {"svg": {
                "outputs": [{
                    "path": "out/diagram.svg",
                    "sha256": "0".repeat(64)
                }],
                "toolVersion": "",
                "renderer": {"mode": "fake", "version": "forged"},
                "environment": {
                    "javaVersion": null,
                    "graphvizVersion": null,
                    "os": "forged",
                    "architecture": "forged"
                }
        }}}}
    });
    write(
        project.path().join(".test-export-state/manifest.json"),
        serde_json::to_vec_pretty(&manifest).expect("manifest JSON"),
    );

    let error = ExportSession::new(FakeRenderer)
        .run(request(&project, &["diagram.puml"]))
        .expect_err("empty provenance must fail closed");

    assert_eq!(error.kind, ExportErrorKind::InvalidManifest);
    assert!(error.message.contains("empty toolVersion"));
}

#[test]
fn output_tree_contains_only_final_artifacts() {
    let project = TempDir::new().expect("project");
    write(project.path().join("diagram.puml"), "diagram");

    let session = ExportSession::new(StagingBoundaryRenderer {
        out_dir: project.path().join("out"),
        state_dir: project.path().join(".test-export-state"),
    });
    for format in [OutputFormat::Svg, OutputFormat::Png, OutputFormat::Pdf] {
        let mut export = request(&project, &["diagram.puml"]);
        export.format = format;
        session.run(export).expect("format export");
    }

    let mut visible = fs::read_dir(project.path().join("out"))
        .expect("output directory")
        .map(|entry| entry.expect("output entry").file_name())
        .collect::<Vec<_>>();
    visible.sort();
    assert_eq!(
        visible,
        ["diagram.pdf", "diagram.png", "diagram.svg"].map(std::ffi::OsString::from)
    );
    assert!(project
        .path()
        .join(".test-export-state/manifest.json")
        .is_file());
    assert!(project
        .path()
        .join(".test-export-state/export.lock")
        .is_file());
    assert!(
        fs::read_dir(project.path().join(".test-export-state/transactions"))
            .expect("transaction directory")
            .next()
            .is_none()
    );
}

#[test]
fn former_internal_names_are_ordinary_source_names_now() {
    let project = TempDir::new().expect("project");
    let input = ".plantuml-export-staging-notes/diagram.puml";
    write(project.path().join(input), "diagram");

    ExportSession::new(FakeRenderer)
        .run(request(&project, &[input]))
        .expect("no output namespace is reserved for engine internals");

    assert!(project
        .path()
        .join("out/.plantuml-export-staging-notes/diagram.svg")
        .is_file());
}

#[test]
fn rejects_state_directories_inside_the_output_tree() {
    let project = TempDir::new().expect("project");
    write(project.path().join("diagram.puml"), "diagram");
    let mut export = request(&project, &["diagram.puml"]);
    export.state_dir = project.path().join("out/.plantuml-export-state");

    let error = ExportSession::new(FakeRenderer)
        .run(export)
        .expect_err("internal state must never be placed below output");

    assert_eq!(error.kind, ExportErrorKind::UnsafePath);
    assert!(!project.path().join("out/.plantuml-export-state").exists());
}
