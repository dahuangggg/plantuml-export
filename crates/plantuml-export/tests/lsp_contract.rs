use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range, TextDocumentSyncCapability};
use plantuml_export::cli::{Layout, OutputFormat, RendererMode, SecurityProfile};
use plantuml_export::config::ResolvedConfig;
use plantuml_export::lsp::{
    initialize_result, LspState, SyntaxCheck, SyntaxCheckResult, CHANGE_DEBOUNCE,
    SAVE_CHECK_TIMEOUT,
};
use url::Url;

#[test]
fn initialize_advertises_only_full_text_sync() {
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
    assert!(result.capabilities.code_action_provider.is_none());
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

fn fixture_config(root: &Path) -> ResolvedConfig {
    ResolvedConfig {
        root: root.to_path_buf(),
        project_config: None,
        user_config: None,
        renderer: RendererMode::Managed,
        format: OutputFormat::Svg,
        out_dir: PathBuf::from("out/plantuml"),
        layout: Layout::Graphviz,
        security: SecurityProfile::Allowlist,
        embed_source_metadata: false,
        include_paths: vec![],
        include: vec![],
        exclude: vec![],
        offline: false,
        remote_includes: false,
        java_path: PathBuf::from("java"),
        binary_path: None,
        jar_path: None,
    }
}
