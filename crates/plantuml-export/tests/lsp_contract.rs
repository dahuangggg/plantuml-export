use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use lsp_types::{
    CodeActionProviderCapability, Diagnostic, DiagnosticSeverity, Position, Range,
    TextDocumentSyncCapability,
};
use plantuml_export::cli::{Layout, OutputFormat, RendererMode};
use plantuml_export::config::{RemoteIncludes, ResolvedConfig};
use plantuml_export::lsp::{
    apply_initialization_options, decode_export_command, export_commands_for_document,
    initialize_result, CancellationToken, LspState, SyntaxCheck, SyntaxCheckResult,
    CHANGE_DEBOUNCE, EXPORT_COMMAND, SAVE_CHECK_TIMEOUT,
};
use url::Url;

#[test]
fn initialize_advertises_full_sync_and_plugin_managed_export_actions() {
    let result = initialize_result();

    assert_eq!(
        result.capabilities.text_document_sync,
        Some(TextDocumentSyncCapability::Kind(
            lsp_types::TextDocumentSyncKind::FULL
        ))
    );
    assert!(result.capabilities.completion_provider.is_none());
    assert!(result.capabilities.hover_provider.is_none());
    assert!(result.capabilities.definition_provider.is_none());
    assert!(result.capabilities.rename_provider.is_none());
    assert!(result.capabilities.document_formatting_provider.is_none());
    assert_eq!(
        result.capabilities.code_action_provider,
        Some(CodeActionProviderCapability::Simple(true))
    );
    assert_eq!(
        result
            .capabilities
            .execute_command_provider
            .expect("plugin-managed exports must be executable through the LSP")
            .commands,
        ["plantuml-export.export"]
    );
}

#[test]
fn zed_initialization_settings_override_safe_values_and_merge_include_paths() {
    let temp = tempfile::tempdir().unwrap();
    let mut config = fixture_config(temp.path());
    config.include_paths = vec![PathBuf::from("user/includes")];
    let params = serde_json::json!({
        "processId": null,
        "rootUri": Url::from_directory_path(temp.path()).unwrap(),
        "capabilities": {},
        "initializationOptions": {
            "includePaths": ["docs/includes"],
            "remoteIncludes": "disabled"
        }
    });

    let resolved = apply_initialization_options(config, &params).unwrap();

    assert_eq!(resolved.remote_includes, RemoteIncludes::Disabled);
    assert_eq!(
        resolved.include_paths,
        [
            PathBuf::from("user/includes"),
            PathBuf::from("docs/includes")
        ]
    );
}

#[test]
fn zed_workspace_settings_cannot_grant_remote_or_machine_access() {
    for settings in [
        serde_json::json!({"allowedRemoteUrls": ["http://localhost:8080/"]}),
        serde_json::json!({"javaPath": "/tmp/java"}),
        serde_json::json!({"offline": false}),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let params = serde_json::json!({
            "processId": null,
            "rootUri": Url::from_directory_path(temp.path()).unwrap(),
            "capabilities": {},
            "initializationOptions": settings
        });

        let error = apply_initialization_options(fixture_config(temp.path()), &params).unwrap_err();

        assert_eq!(error.code, "untrusted_lsp_setting");
    }
}

#[test]
fn missing_initialization_options_preserve_the_resolved_configuration() {
    let temp = tempfile::tempdir().unwrap();
    let config = fixture_config(temp.path());
    let params = serde_json::json!({
        "processId": null,
        "rootUri": Url::from_directory_path(temp.path()).unwrap(),
        "capabilities": {}
    });

    assert_eq!(
        apply_initialization_options(config.clone(), &params).unwrap(),
        config
    );
}

#[test]
fn zed_initialization_include_paths_must_be_portable_and_project_relative() {
    for include_path in ["../shared", "/tmp/shared", "C:\\shared"] {
        let temp = tempfile::tempdir().unwrap();
        let params = serde_json::json!({
            "processId": null,
            "rootUri": Url::from_directory_path(temp.path()).unwrap(),
            "capabilities": {},
            "initializationOptions": {"includePaths": [include_path]}
        });

        let error = apply_initialization_options(fixture_config(temp.path()), &params).unwrap_err();

        assert_eq!(error.code, "nonportable_project_path", "{include_path}");
    }
}

#[test]
fn unknown_zed_initialization_options_fail_closed() {
    let temp = tempfile::tempdir().unwrap();
    let params = serde_json::json!({
        "processId": null,
        "rootUri": Url::from_directory_path(temp.path()).unwrap(),
        "capabilities": {},
        "initializationOptions": {"security": "insecure"}
    });

    let error = apply_initialization_options(fixture_config(temp.path()), &params).unwrap_err();

    assert_eq!(error.code, "invalid_lsp_initialization_options");
}

#[test]
fn saved_plantuml_documents_offer_svg_png_and_pdf_exports_through_the_lsp_helper() {
    let uri = Url::parse("file:///workspace/model.puml").unwrap();

    let commands = export_commands_for_document(&uri);

    assert_eq!(commands.len(), 3);
    assert_eq!(
        commands
            .iter()
            .map(|command| command.title.as_str())
            .collect::<Vec<_>>(),
        [
            "Export PlantUML to SVG",
            "Export PlantUML to PNG",
            "Export PlantUML to PDF",
        ]
    );
    for (command, format) in commands.iter().zip(["svg", "png", "pdf"]) {
        assert_eq!(command.command, EXPORT_COMMAND);
        assert_eq!(
            command.arguments.as_deref(),
            Some(
                &[serde_json::json!({
                    "uri": uri,
                    "format": format,
                })][..]
            )
        );
    }
}

#[test]
fn non_file_documents_do_not_offer_native_export_actions() {
    let uri = Url::parse("untitled:PlantUML-1").unwrap();

    assert!(export_commands_for_document(&uri).is_empty());
}

#[test]
fn export_command_accepts_exactly_one_typed_uri_and_format_argument() {
    let uri = Url::parse("file:///workspace/model.puml").unwrap();

    let request = decode_export_command(
        EXPORT_COMMAND,
        &[serde_json::json!({"uri": uri, "format": "png"})],
    )
    .unwrap();

    assert_eq!(request.uri, uri);
    assert_eq!(request.format, OutputFormat::Png);

    for arguments in [
        vec![],
        vec![serde_json::json!({"uri": uri, "format": "gif"})],
        vec![serde_json::json!({
            "uri": uri,
            "format": "svg",
            "outDir": "/tmp/escape"
        })],
    ] {
        assert!(decode_export_command(EXPORT_COMMAND, &arguments).is_err());
    }
    assert!(decode_export_command("plantuml-export.delete", &[]).is_err());
}

#[test]
fn export_uses_the_saved_document_inside_the_worktree_and_rejects_dirty_content() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("repo");
    std::fs::create_dir(&root).unwrap();
    let path = root.join("model.puml");
    let saved = "@startuml\nAlice -> Bob\n@enduml\n";
    std::fs::write(&path, saved).unwrap();
    let uri = Url::from_file_path(&path).unwrap();
    let request = decode_export_command(
        EXPORT_COMMAND,
        &[serde_json::json!({"uri": uri, "format": "svg"})],
    )
    .unwrap();
    let mut state = LspState::default();
    state.did_open(uri.clone(), saved.into(), 1);

    assert_eq!(
        state.saved_export_input(&request, &root).unwrap(),
        path.canonicalize().unwrap()
    );

    state.did_change(uri, "@startuml\nAlice -> Carol\n@enduml\n".into(), 2);
    let error = state.saved_export_input(&request, &root).unwrap_err();
    assert_eq!(error.code, "document_not_saved");
    assert!(error.message.contains("save"));
}

#[test]
fn export_rejects_a_document_outside_the_worktree() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(outside.path(), "@startuml\n@enduml\n").unwrap();
    let uri = Url::from_file_path(outside.path()).unwrap();
    let request = decode_export_command(
        EXPORT_COMMAND,
        &[serde_json::json!({"uri": uri, "format": "svg"})],
    )
    .unwrap();
    let mut state = LspState::default();
    state.did_open(uri, "@startuml\n@enduml\n".into(), 1);

    let error = state.saved_export_input(&request, root.path()).unwrap_err();

    assert_eq!(error.code, "input_outside_root");
}

#[test]
fn change_diagnostics_are_debounced_and_only_the_latest_generation_publishes() {
    let mut state = LspState::default();
    let uri = Url::parse("file:///workspace/model.puml").unwrap();
    let stale = state.did_open(uri.clone(), "@startuml\n".into(), 1);
    let latest = state.did_change(uri.clone(), "@startuml\n@enduml\n".into(), 2);

    assert_eq!(stale.delay, CHANGE_DEBOUNCE);
    assert_eq!(latest.delay, Duration::from_millis(250));
    assert!(state.finish_structural(stale).is_none());
    let publish = state.finish_structural(latest).unwrap();
    assert_eq!(publish.uri, uri);
    assert_eq!(publish.version, Some(2));
    assert_eq!(publish.diagnostics.len(), 1);
    assert_eq!(
        publish.diagnostics[0].message,
        "PlantUML diagram has no body."
    );
}

#[test]
fn save_check_receives_the_real_document_path_and_ten_second_timeout() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("model.puml");
    std::fs::write(&path, "@startuml\nAlice -> Bob\n@enduml\n").unwrap();
    let uri = Url::from_file_path(&path).unwrap();
    let mut state = LspState::default();
    state.did_open(uri.clone(), "@startuml\nAlice -> Bob\n@enduml\n".into(), 1);
    let job = state.did_save(&uri).unwrap();
    let checker = RecordingChecker::default();

    let publish = state
        .run_save_check(job, &checker, &fixture_config(temp.path()))
        .unwrap();

    assert_eq!(
        *checker.path.lock().unwrap(),
        Some(path.canonicalize().unwrap())
    );
    assert_eq!(*checker.timeout.lock().unwrap(), Some(SAVE_CHECK_TIMEOUT));
    assert!(publish.diagnostics.is_empty());
}

#[test]
fn environment_failures_never_become_fake_source_diagnostics() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("model.puml");
    std::fs::write(&path, "@startuml\nAlice -> Bob\n@enduml\n").unwrap();
    let uri = Url::from_file_path(&path).unwrap();
    let mut state = LspState::default();
    state.did_open(uri.clone(), "@startuml\nAlice -> Bob\n@enduml\n".into(), 1);
    let job = state.did_save(&uri).unwrap();

    let publish = state
        .run_save_check(
            job,
            &RecordingChecker::default(),
            &fixture_config(temp.path()),
        )
        .unwrap();

    assert!(publish.diagnostics.is_empty());
}

#[test]
fn a_save_result_is_discarded_after_a_newer_edit() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("model.puml");
    std::fs::write(&path, "@startuml\nAlice -> Bob\n@enduml\n").unwrap();
    let uri = Url::from_file_path(&path).unwrap();
    let mut state = LspState::default();
    state.did_open(uri.clone(), "@startuml\n@enduml\n".into(), 1);
    let save = state.did_save(&uri).unwrap();
    state.did_change(uri, "@startuml\nAlice -> Bob\n@enduml\n".into(), 2);
    let checker = DiagnosticChecker;

    assert!(state
        .run_save_check(save, &checker, &fixture_config(temp.path()))
        .is_none());
}

#[test]
fn a_newer_edit_actively_cancels_the_previous_save_check() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("model.puml");
    std::fs::write(&path, "@startuml\nAlice -> Bob\n@enduml\n").unwrap();
    let uri = Url::from_file_path(&path).unwrap();
    let mut state = LspState::default();
    state.did_open(uri.clone(), "@startuml\n@enduml\n".into(), 1);
    let save = state.did_save(&uri).unwrap();
    state.did_change(uri, "@startuml\nAlice -> Bob\n@enduml\n".into(), 2);
    let checker = CancellationRecordingChecker::default();

    assert!(state
        .run_save_check(save, &checker, &fixture_config(temp.path()))
        .is_none());
    assert!(checker.observed.load(Ordering::Acquire));
}

#[derive(Default)]
struct RecordingChecker {
    path: Mutex<Option<PathBuf>>,
    timeout: Mutex<Option<Duration>>,
}

impl SyntaxCheck for RecordingChecker {
    fn check(&self, path: &Path, _config: &ResolvedConfig, timeout: Duration) -> SyntaxCheckResult {
        *self.path.lock().unwrap() = Some(path.to_path_buf());
        *self.timeout.lock().unwrap() = Some(timeout);
        SyntaxCheckResult::Environment("renderer is unavailable".into())
    }
}

struct DiagnosticChecker;

impl SyntaxCheck for DiagnosticChecker {
    fn check(
        &self,
        _path: &Path,
        _config: &ResolvedConfig,
        _timeout: Duration,
    ) -> SyntaxCheckResult {
        SyntaxCheckResult::Diagnostics(vec![Diagnostic {
            range: Range::new(Position::new(0, 0), Position::new(0, 1)),
            severity: Some(DiagnosticSeverity::ERROR),
            source: Some("plantuml".into()),
            message: "stale".into(),
            ..Diagnostic::default()
        }])
    }
}

#[derive(Default)]
struct CancellationRecordingChecker {
    observed: AtomicBool,
}

impl SyntaxCheck for CancellationRecordingChecker {
    fn check(
        &self,
        _path: &Path,
        _config: &ResolvedConfig,
        _timeout: Duration,
    ) -> SyntaxCheckResult {
        panic!("the cancellable entrypoint must be used")
    }

    fn check_cancellable(
        &self,
        _path: &Path,
        _config: &ResolvedConfig,
        _timeout: Duration,
        cancellation: &CancellationToken,
    ) -> SyntaxCheckResult {
        self.observed
            .store(cancellation.is_cancelled(), Ordering::Release);
        SyntaxCheckResult::Cancelled
    }
}

fn fixture_config(root: &Path) -> ResolvedConfig {
    ResolvedConfig {
        root: root.to_path_buf(),
        project_config: None,
        user_config: None,
        renderer: RendererMode::Managed,
        format: OutputFormat::Svg,
        out_dir: PathBuf::from("out"),
        layout: Layout::Graphviz,
        embed_source_metadata: false,
        include_paths: vec![],
        include: vec![],
        exclude: vec![],
        offline: false,
        remote_includes: RemoteIncludes::Public,
        allowed_remote_urls: vec![],
        java_path: PathBuf::from("java"),
        binary_path: None,
        jar_path: None,
        graphviz_path: PathBuf::from("dot"),
    }
}
