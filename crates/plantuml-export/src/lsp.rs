use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use lsp_server::{Connection, ErrorCode, Message, Notification, Response};
use lsp_types::{
    Diagnostic, DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DidSaveTextDocumentParams, InitializeResult, PublishDiagnosticsParams, ServerCapabilities,
    ServerInfo, TextDocumentSyncCapability, TextDocumentSyncKind, Url,
};

use crate::cli::RendererMode;
use crate::config::ResolvedConfig;
use crate::diagnostics::{parse_standard_report, structural_diagnostics};
use crate::AppError;

pub const CHANGE_DEBOUNCE: Duration = Duration::from_millis(250);
pub const SAVE_CHECK_TIMEOUT: Duration = Duration::from_secs(10);

pub fn initialize_result() -> InitializeResult {
    InitializeResult {
        capabilities: ServerCapabilities {
            text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
            ..ServerCapabilities::default()
        },
        server_info: Some(ServerInfo {
            name: "plantuml-export".to_string(),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
        }),
    }
}

#[derive(Clone, Debug)]
struct Document {
    text: String,
    version: i32,
    generation: u64,
}

#[derive(Debug, Default)]
pub struct LspState {
    documents: HashMap<Url, Document>,
}

impl LspState {
    pub fn did_open(&mut self, uri: Url, text: String, version: i32) -> StructuralJob {
        self.update(uri, text, version)
    }

    pub fn did_change(&mut self, uri: Url, text: String, version: i32) -> StructuralJob {
        self.update(uri, text, version)
    }

    pub fn did_save(&mut self, uri: &Url) -> Option<SaveCheckJob> {
        let document = self.documents.get_mut(uri)?;
        document.generation = document.generation.saturating_add(1);
        let path = uri.to_file_path().ok()?;
        let path = path.canonicalize().unwrap_or(path);
        Some(SaveCheckJob {
            uri: uri.clone(),
            version: document.version,
            generation: document.generation,
            text: document.text.clone(),
            path,
            timeout: SAVE_CHECK_TIMEOUT,
        })
    }

    pub fn did_close(&mut self, uri: &Url) -> bool {
        self.documents.remove(uri).is_some()
    }

    pub fn finish_structural(&self, job: StructuralJob) -> Option<Publish> {
        self.is_latest(&job.uri, job.generation).then(|| Publish {
            uri: job.uri,
            version: Some(job.version),
            diagnostics: structural_diagnostics(&job.text),
        })
    }

    pub fn finish_save(&self, result: SaveCheckCompletion) -> Option<Publish> {
        self.is_latest(&result.uri, result.generation)
            .then_some(Publish {
                uri: result.uri,
                version: Some(result.version),
                diagnostics: result.diagnostics,
            })
    }

    pub fn run_save_check(
        &self,
        job: SaveCheckJob,
        checker: &dyn SyntaxCheck,
        config: &ResolvedConfig,
    ) -> Option<Publish> {
        self.finish_save(job.execute(checker, config))
    }

    fn update(&mut self, uri: Url, text: String, version: i32) -> StructuralJob {
        let generation = self
            .documents
            .get(&uri)
            .map_or(1, |document| document.generation.saturating_add(1));
        self.documents.insert(
            uri.clone(),
            Document {
                text: text.clone(),
                version,
                generation,
            },
        );
        StructuralJob {
            uri,
            version,
            generation,
            text,
            delay: CHANGE_DEBOUNCE,
        }
    }

    fn is_latest(&self, uri: &Url, generation: u64) -> bool {
        self.documents
            .get(uri)
            .is_some_and(|document| document.generation == generation)
    }
}

#[derive(Clone, Debug)]
pub struct StructuralJob {
    pub uri: Url,
    pub version: i32,
    pub generation: u64,
    pub text: String,
    pub delay: Duration,
}

#[derive(Clone, Debug)]
pub struct SaveCheckJob {
    pub uri: Url,
    pub version: i32,
    pub generation: u64,
    pub text: String,
    pub path: PathBuf,
    pub timeout: Duration,
}

impl SaveCheckJob {
    pub fn execute(
        self,
        checker: &dyn SyntaxCheck,
        config: &ResolvedConfig,
    ) -> SaveCheckCompletion {
        let mut diagnostics = structural_diagnostics(&self.text);
        let mut environment = None;
        match checker.check(&self.path, config, self.timeout) {
            SyntaxCheckResult::Valid => {}
            SyntaxCheckResult::Diagnostics(mut syntax) => diagnostics.append(&mut syntax),
            SyntaxCheckResult::Environment(message) => environment = Some(message),
            SyntaxCheckResult::TimedOut => {
                environment = Some(format!(
                    "PlantUML syntax check timed out after {} seconds",
                    self.timeout.as_secs()
                ));
            }
        }
        SaveCheckCompletion {
            uri: self.uri,
            version: self.version,
            generation: self.generation,
            diagnostics,
            environment,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SaveCheckCompletion {
    uri: Url,
    version: i32,
    generation: u64,
    diagnostics: Vec<Diagnostic>,
    environment: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Publish {
    pub uri: Url,
    pub version: Option<i32>,
    pub diagnostics: Vec<Diagnostic>,
}

pub trait SyntaxCheck: Send + Sync {
    fn check(&self, path: &Path, config: &ResolvedConfig, timeout: Duration) -> SyntaxCheckResult;
}

#[derive(Clone, Debug)]
pub enum SyntaxCheckResult {
    Valid,
    Diagnostics(Vec<Diagnostic>),
    Environment(String),
    TimedOut,
}

#[derive(Debug, Default)]
pub struct PlantUmlSyntaxChecker;

impl SyntaxCheck for PlantUmlSyntaxChecker {
    fn check(&self, path: &Path, config: &ResolvedConfig, timeout: Duration) -> SyntaxCheckResult {
        let (program, args) = match syntax_command(path, config) {
            Ok(command) => command,
            Err(message) => return SyntaxCheckResult::Environment(message),
        };

        run_syntax_command(&program, &args, timeout)
    }
}

fn syntax_command(path: &Path, config: &ResolvedConfig) -> Result<(PathBuf, Vec<String>), String> {
    let source = path.to_string_lossy().into_owned();
    let check_args = || {
        vec![
            "--check-syntax".to_string(),
            "--stop-on-error".to_string(),
            "-stdrpt:1".to_string(),
            source.clone(),
        ]
    };

    match config.renderer {
        RendererMode::Managed => Err(
            "managed PlantUML renderer resolution is not available in this build phase".to_string(),
        ),
        RendererMode::Binary => Ok((
            config
                .binary_path
                .clone()
                .unwrap_or_else(|| PathBuf::from("plantuml")),
            check_args(),
        )),
        RendererMode::Jar => {
            let jar = config.jar_path.as_ref().ok_or_else(|| {
                "jar renderer mode requires jar_path in user config or --jar".to_string()
            })?;
            if !jar.is_file() {
                return Err(format!("PlantUML jar not found: {}", jar.display()));
            }
            let mut args = vec!["-jar".to_string(), jar.to_string_lossy().into_owned()];
            args.extend(check_args());
            Ok((config.java_path.clone(), args))
        }
    }
}

fn run_syntax_command(program: &Path, args: &[String], timeout: Duration) -> SyntaxCheckResult {
    let mut child = match ProcessCommand::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            return SyntaxCheckResult::Environment(format!(
                "failed to start {}: {error}",
                program.display()
            ));
        }
    };

    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let output = match child.wait_with_output() {
                    Ok(output) => output,
                    Err(error) => {
                        return SyntaxCheckResult::Environment(format!(
                            "failed to collect PlantUML syntax output: {error}"
                        ));
                    }
                };
                let report = format!(
                    "{}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                let diagnostics = parse_standard_report(&report);
                if !diagnostics.is_empty() {
                    return SyntaxCheckResult::Diagnostics(diagnostics);
                }
                if output.status.success() {
                    return SyntaxCheckResult::Valid;
                }
                return SyntaxCheckResult::Environment(if report.trim().is_empty() {
                    format!("PlantUML syntax check exited with {}", output.status)
                } else {
                    report.trim().to_string()
                });
            }
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return SyntaxCheckResult::TimedOut;
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return SyntaxCheckResult::Environment(format!(
                    "failed while waiting for PlantUML syntax check: {error}"
                ));
            }
        }
    }
}

pub fn run_stdio(config: ResolvedConfig) -> Result<(), AppError> {
    let (connection, io_threads) = Connection::stdio();
    let (initialize_id, _) = connection.initialize_start().map_err(protocol_error)?;
    let initialization = serde_json::to_value(initialize_result()).map_err(|error| {
        AppError::environment(
            "lsp_initialize",
            format!("failed to serialize LSP capabilities: {error}"),
        )
    })?;
    connection
        .initialize_finish(initialize_id, initialization)
        .map_err(protocol_error)?;

    let state = Arc::new(Mutex::new(LspState::default()));
    let checker: Arc<dyn SyntaxCheck> = Arc::new(PlantUmlSyntaxChecker);

    for message in &connection.receiver {
        match message {
            Message::Request(request) => {
                if connection
                    .handle_shutdown(&request)
                    .map_err(protocol_error)?
                {
                    break;
                }
                let response = Response::new_err(
                    request.id,
                    ErrorCode::MethodNotFound as i32,
                    format!("unknown request: {}", request.method),
                );
                let _ = connection.sender.send(Message::Response(response));
            }
            Message::Notification(notification) => {
                handle_notification(notification, &state, &checker, &config, &connection)
            }
            Message::Response(_) => {}
        }
    }

    drop(connection);
    io_threads
        .join()
        .map_err(|error| AppError::environment("lsp_stdio", format!("LSP stdio failed: {error}")))
}

fn handle_notification(
    notification: Notification,
    state: &Arc<Mutex<LspState>>,
    checker: &Arc<dyn SyntaxCheck>,
    config: &ResolvedConfig,
    connection: &Connection,
) {
    match notification.method.as_str() {
        "textDocument/didOpen" => {
            if let Ok(params) =
                serde_json::from_value::<DidOpenTextDocumentParams>(notification.params)
            {
                let document = params.text_document;
                let job = state.lock().expect("LSP state poisoned").did_open(
                    document.uri,
                    document.text,
                    document.version,
                );
                spawn_structural(job, Arc::clone(state), connection.sender.clone());
            }
        }
        "textDocument/didChange" => {
            if let Ok(params) =
                serde_json::from_value::<DidChangeTextDocumentParams>(notification.params)
            {
                if let Some(change) = params.content_changes.into_iter().last() {
                    let job = state.lock().expect("LSP state poisoned").did_change(
                        params.text_document.uri,
                        change.text,
                        params.text_document.version,
                    );
                    spawn_structural(job, Arc::clone(state), connection.sender.clone());
                }
            }
        }
        "textDocument/didSave" => {
            if let Ok(params) =
                serde_json::from_value::<DidSaveTextDocumentParams>(notification.params)
            {
                let job = state
                    .lock()
                    .expect("LSP state poisoned")
                    .did_save(&params.text_document.uri);
                if let Some(job) = job {
                    spawn_save(
                        job,
                        Arc::clone(state),
                        Arc::clone(checker),
                        config.clone(),
                        connection.sender.clone(),
                    );
                }
            }
        }
        "textDocument/didClose" => {
            if let Ok(params) =
                serde_json::from_value::<DidCloseTextDocumentParams>(notification.params)
            {
                let uri = params.text_document.uri;
                if state.lock().expect("LSP state poisoned").did_close(&uri) {
                    send_publish(
                        &connection.sender,
                        Publish {
                            uri,
                            version: None,
                            diagnostics: Vec::new(),
                        },
                    );
                }
            }
        }
        _ => {}
    }
}

fn spawn_structural(
    job: StructuralJob,
    state: Arc<Mutex<LspState>>,
    sender: crossbeam_channel::Sender<Message>,
) {
    thread::spawn(move || {
        thread::sleep(job.delay);
        let publish = state
            .lock()
            .expect("LSP state poisoned")
            .finish_structural(job);
        if let Some(publish) = publish {
            send_publish(&sender, publish);
        }
    });
}

fn spawn_save(
    job: SaveCheckJob,
    state: Arc<Mutex<LspState>>,
    checker: Arc<dyn SyntaxCheck>,
    config: ResolvedConfig,
    sender: crossbeam_channel::Sender<Message>,
) {
    thread::spawn(move || {
        let completion = job.execute(checker.as_ref(), &config);
        if let Some(message) = &completion.environment {
            eprintln!("plantuml-export lsp: {message}");
        }
        let publish = state
            .lock()
            .expect("LSP state poisoned")
            .finish_save(completion);
        if let Some(publish) = publish {
            send_publish(&sender, publish);
        }
    });
}

fn send_publish(sender: &crossbeam_channel::Sender<Message>, publish: Publish) {
    let params = PublishDiagnosticsParams::new(publish.uri, publish.diagnostics, publish.version);
    let notification = Notification::new("textDocument/publishDiagnostics".to_string(), params);
    let _ = sender.send(Message::Notification(notification));
}

fn protocol_error(error: impl std::fmt::Display) -> AppError {
    AppError::environment("lsp_protocol", format!("LSP protocol error: {error}"))
}
