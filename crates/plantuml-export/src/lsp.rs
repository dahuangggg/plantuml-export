use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use lsp_server::{Connection, ErrorCode, Message, Notification, Request, RequestId, Response};
use lsp_types::{
    CodeActionOrCommand, CodeActionParams, CodeActionProviderCapability, Command as LspCommand,
    Diagnostic, DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DidSaveTextDocumentParams, ExecuteCommandOptions, ExecuteCommandParams, InitializeParams,
    InitializeResult, MessageType, PublishDiagnosticsParams, ServerCapabilities, ServerInfo,
    ShowMessageParams, TextDocumentSyncCapability, TextDocumentSyncKind, Url,
};
use serde::Deserialize;

use crate::cli::OutputFormat;
use crate::config::{validate_portable_workspace_paths, RemoteIncludes, ResolvedConfig};
use crate::diagnostics::{parse_standard_report, structural_diagnostics};
use crate::process_control::{execute_cancellable, ControlledProcessFailure, ProcessFailure};
use crate::renderer::{build_syntax_command, create_syntax_output_dir, CommandSpec, SyntaxRequest};
use crate::AppError;

pub const CHANGE_DEBOUNCE: Duration = Duration::from_millis(250);
pub const SAVE_CHECK_TIMEOUT: Duration = Duration::from_secs(10);
pub const EXPORT_COMMAND: &str = "plantuml-export.export";
const EXPORT_QUEUE_CAPACITY: usize = 1;

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InitializationOptions {
    #[serde(default)]
    include_paths: Vec<PathBuf>,
    remote_includes: Option<RemoteIncludes>,
}

pub fn apply_initialization_options(
    mut config: ResolvedConfig,
    params: &serde_json::Value,
) -> Result<ResolvedConfig, AppError> {
    let params: InitializeParams = serde_json::from_value(params.clone()).map_err(|error| {
        AppError::usage(
            "invalid_lsp_initialize",
            format!("invalid LSP initialize parameters: {error}"),
        )
    })?;
    let Some(value) = params.initialization_options else {
        return Ok(config);
    };
    if value.is_null() {
        return Ok(config);
    }
    if let Some(key) = untrusted_initialization_key(&value) {
        return Err(AppError::usage(
            "untrusted_lsp_setting",
            format!(
                "`{key}` cannot be granted by worktree LSP settings; machine tools, offline policy, and trusted remote origins belong in the user config"
            ),
        ));
    }
    let options: InitializationOptions = serde_json::from_value(value).map_err(|error| {
        AppError::usage(
            "invalid_lsp_initialization_options",
            format!("invalid PlantUML LSP initialization options: {error}"),
        )
    })?;
    validate_portable_workspace_paths(
        "Zed initialization options",
        "includePaths",
        &options.include_paths,
    )?;
    config.apply_workspace_options(options.include_paths, options.remote_includes);
    Ok(config)
}

fn untrusted_initialization_key(value: &serde_json::Value) -> Option<String> {
    let object = value.as_object()?;
    object.keys().find_map(|key| {
        let normalized = key
            .chars()
            .filter(|character| !matches!(character, '_' | '-'))
            .flat_map(char::to_lowercase)
            .collect::<String>();
        matches!(
            normalized.as_str(),
            "allowedremoteurls"
                | "javapath"
                | "binarypath"
                | "jarpath"
                | "graphvizpath"
                | "offline"
        )
        .then(|| key.clone())
    })
}

struct ExportJob {
    id: RequestId,
    input: PathBuf,
    format: OutputFormat,
}

struct ExportWorker {
    jobs: crossbeam_channel::Sender<ExportJob>,
    cancellation: crate::runtime::ExportCancellation,
    thread: thread::JoinHandle<()>,
}

impl ExportWorker {
    fn spawn(config: ResolvedConfig, sender: crossbeam_channel::Sender<Message>) -> Self {
        let (jobs, receiver) = crossbeam_channel::bounded(EXPORT_QUEUE_CAPACITY);
        let cancellation = crate::runtime::ExportCancellation::default();
        let thread = spawn_export_worker(config, receiver, sender, cancellation.clone());
        Self {
            jobs,
            cancellation,
            thread,
        }
    }

    fn submit(&self, job: ExportJob) -> Result<(), crossbeam_channel::TrySendError<ExportJob>> {
        self.jobs.try_send(job)
    }

    fn shutdown(self) -> Result<(), AppError> {
        self.cancellation.cancel();
        drop(self.jobs);
        self.thread.join().map_err(|_| {
            AppError::environment("lsp_export_worker", "PlantUML export worker panicked")
        })
    }
}

struct StructuralWorker {
    jobs: crossbeam_channel::Sender<StructuralJob>,
    pending: crossbeam_channel::Receiver<StructuralJob>,
    thread: thread::JoinHandle<()>,
}

impl StructuralWorker {
    fn spawn(state: Arc<Mutex<LspState>>, sender: crossbeam_channel::Sender<Message>) -> Self {
        let (jobs, receiver) = crossbeam_channel::bounded::<StructuralJob>(1);
        let pending = receiver.clone();
        let thread = thread::spawn(move || {
            while let Ok(mut job) = receiver.recv() {
                loop {
                    match receiver.recv_timeout(job.delay) {
                        Ok(latest) => job = latest,
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                            let publish = state
                                .lock()
                                .expect("LSP state poisoned")
                                .finish_structural(job);
                            if let Some(publish) = publish {
                                send_publish(&sender, publish);
                            }
                            break;
                        }
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return,
                    }
                }
            }
        });
        Self {
            jobs,
            pending,
            thread,
        }
    }

    fn submit(&self, mut job: StructuralJob) {
        loop {
            match self.jobs.try_send(job) {
                Ok(()) => return,
                Err(crossbeam_channel::TrySendError::Full(returned)) => {
                    job = returned;
                    match self.pending.try_recv() {
                        Ok(_) | Err(crossbeam_channel::TryRecvError::Empty) => {}
                        Err(crossbeam_channel::TryRecvError::Disconnected) => return,
                    }
                }
                Err(crossbeam_channel::TrySendError::Disconnected(_)) => return,
            }
        }
    }

    fn shutdown(self) -> Result<(), AppError> {
        let Self {
            jobs,
            pending,
            thread,
        } = self;
        drop(jobs);
        drop(pending);
        thread.join().map_err(|_| {
            AppError::environment("lsp_structural_worker", "LSP structural worker panicked")
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportCommandRequest {
    pub uri: Url,
    pub format: OutputFormat,
}

pub fn decode_export_command(
    command: &str,
    arguments: &[serde_json::Value],
) -> Result<ExportCommandRequest, AppError> {
    if command != EXPORT_COMMAND {
        return Err(AppError::usage(
            "unknown_lsp_command",
            format!("unknown PlantUML LSP command: {command}"),
        ));
    }
    if arguments.len() != 1 {
        return Err(AppError::usage(
            "invalid_lsp_export_arguments",
            "PlantUML export requires exactly one URI and format argument",
        ));
    }
    serde_json::from_value(arguments[0].clone()).map_err(|error| {
        AppError::usage(
            "invalid_lsp_export_arguments",
            format!("invalid PlantUML export argument: {error}"),
        )
    })
}

pub fn export_commands_for_document(uri: &Url) -> Vec<LspCommand> {
    if uri.to_file_path().is_err() {
        return Vec::new();
    }

    [("SVG", "svg"), ("PNG", "png"), ("PDF", "pdf")]
        .into_iter()
        .map(|(label, format)| LspCommand {
            title: format!("Export PlantUML to {label}"),
            command: EXPORT_COMMAND.to_string(),
            arguments: Some(vec![serde_json::json!({
                "uri": uri,
                "format": format,
            })]),
        })
        .collect()
}

pub fn initialize_result() -> InitializeResult {
    InitializeResult {
        capabilities: ServerCapabilities {
            text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
            // Zed filters out LSP actions whose commands are not advertised here.
            // Source: https://github.com/zed-industries/zed/blob/dde45ff09276331eb58419c3245d4a3ccb7534f6/crates/project/src/lsp_command.rs#L3013-L3037
            code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
            execute_command_provider: Some(ExecuteCommandOptions {
                commands: vec![EXPORT_COMMAND.to_string()],
                ..ExecuteCommandOptions::default()
            }),
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
    cancellation: CancellationToken,
}

#[derive(Clone, Debug, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

#[derive(Debug, Default)]
pub struct LspState {
    documents: HashMap<Url, Document>,
}

impl LspState {
    pub fn saved_export_input(
        &self,
        request: &ExportCommandRequest,
        root: &Path,
    ) -> Result<PathBuf, AppError> {
        let document = self.documents.get(&request.uri).ok_or_else(|| {
            AppError::operation(
                "document_not_open",
                "open the PlantUML document in Zed before exporting it",
            )
        })?;
        let root = root.canonicalize().map_err(|error| {
            AppError::usage(
                "invalid_root",
                format!(
                    "failed to resolve worktree root {}: {error}",
                    root.display()
                ),
            )
        })?;
        let input = request.uri.to_file_path().map_err(|_| {
            AppError::usage(
                "invalid_document_uri",
                "PlantUML export requires a saved local file",
            )
        })?;
        let input = input.canonicalize().map_err(|error| {
            AppError::operation(
                "document_not_saved",
                format!("save the PlantUML document before exporting: {error}"),
            )
        })?;
        if !input.is_file() {
            return Err(AppError::usage(
                "input_not_file",
                format!("PlantUML export input is not a file: {}", input.display()),
            ));
        }
        if !input.starts_with(&root) {
            return Err(AppError::usage(
                "input_outside_root",
                format!(
                    "PlantUML export input must stay inside the worktree root: {}",
                    input.display()
                ),
            ));
        }
        let saved = std::fs::read_to_string(&input).map_err(|error| {
            AppError::operation(
                "document_not_saved",
                format!("save the PlantUML document before exporting: {error}"),
            )
        })?;
        if saved != document.text {
            return Err(AppError::operation(
                "document_not_saved",
                "save the PlantUML document before exporting",
            ));
        }
        Ok(input)
    }

    pub fn did_open(&mut self, uri: Url, text: String, version: i32) -> StructuralJob {
        self.update(uri, text, version)
    }

    pub fn did_change(&mut self, uri: Url, text: String, version: i32) -> StructuralJob {
        self.update(uri, text, version)
    }

    pub fn did_save(&mut self, uri: &Url) -> Option<SaveCheckJob> {
        let document = self.documents.get_mut(uri)?;
        document.cancellation.cancel();
        document.cancellation = CancellationToken::default();
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
            cancellation: document.cancellation.clone(),
        })
    }

    pub fn did_close(&mut self, uri: &Url) -> bool {
        if let Some(document) = self.documents.remove(uri) {
            document.cancellation.cancel();
            true
        } else {
            false
        }
    }

    pub fn cancel_all(&self) {
        for document in self.documents.values() {
            document.cancellation.cancel();
        }
    }

    pub fn finish_structural(&self, job: StructuralJob) -> Option<Publish> {
        self.is_latest(&job.uri, job.generation).then(|| Publish {
            uri: job.uri,
            version: Some(job.version),
            diagnostics: structural_diagnostics(&job.text),
        })
    }

    pub fn finish_save(&self, result: SaveCheckCompletion) -> Option<Publish> {
        (!result.cancelled && self.is_latest(&result.uri, result.generation)).then_some(Publish {
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
        if let Some(document) = self.documents.get(&uri) {
            document.cancellation.cancel();
        }
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
                cancellation: CancellationToken::default(),
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
    pub cancellation: CancellationToken,
}

impl SaveCheckJob {
    pub fn execute(
        self,
        checker: &dyn SyntaxCheck,
        config: &ResolvedConfig,
    ) -> SaveCheckCompletion {
        let mut diagnostics = structural_diagnostics(&self.text);
        let mut environment = None;
        let mut cancelled = false;
        match checker.check_cancellable(&self.path, config, self.timeout, &self.cancellation) {
            SyntaxCheckResult::Valid => {}
            SyntaxCheckResult::Diagnostics(mut syntax) => diagnostics.append(&mut syntax),
            SyntaxCheckResult::Environment(message) => environment = Some(message),
            SyntaxCheckResult::TimedOut => {
                environment = Some(format!(
                    "PlantUML syntax check timed out after {} seconds",
                    self.timeout.as_secs()
                ));
            }
            SyntaxCheckResult::Cancelled => cancelled = true,
        }
        SaveCheckCompletion {
            uri: self.uri,
            version: self.version,
            generation: self.generation,
            diagnostics,
            environment,
            cancelled,
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
    cancelled: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Publish {
    pub uri: Url,
    pub version: Option<i32>,
    pub diagnostics: Vec<Diagnostic>,
}

pub trait SyntaxCheck: Send + Sync {
    fn check(&self, path: &Path, config: &ResolvedConfig, timeout: Duration) -> SyntaxCheckResult;

    fn check_cancellable(
        &self,
        path: &Path,
        config: &ResolvedConfig,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> SyntaxCheckResult {
        if cancellation.is_cancelled() {
            SyntaxCheckResult::Cancelled
        } else {
            self.check(path, config, timeout)
        }
    }
}

#[derive(Clone, Debug)]
pub enum SyntaxCheckResult {
    Valid,
    Diagnostics(Vec<Diagnostic>),
    Environment(String),
    TimedOut,
    Cancelled,
}

#[derive(Debug, Default)]
pub struct PlantUmlSyntaxChecker;

impl SyntaxCheck for PlantUmlSyntaxChecker {
    fn check(&self, path: &Path, config: &ResolvedConfig, timeout: Duration) -> SyntaxCheckResult {
        self.check_cancellable(path, config, timeout, &CancellationToken::default())
    }

    fn check_cancellable(
        &self,
        path: &Path,
        config: &ResolvedConfig,
        timeout: Duration,
        cancellation: &CancellationToken,
    ) -> SyntaxCheckResult {
        let (syntax_output, command) = match syntax_command(path, config) {
            Ok(command) => command,
            Err(message) => return SyntaxCheckResult::Environment(message),
        };

        let result = apply_remote_policy_hint(
            path,
            config,
            run_syntax_command(&command, timeout, cancellation),
        );
        let scratch_path = syntax_output.path().to_path_buf();
        if let Err(error) = syntax_output.close() {
            return SyntaxCheckResult::Environment(format!(
                "could not clean isolated syntax output directory {}: {error}",
                scratch_path.display()
            ));
        }
        result
    }
}

fn apply_remote_policy_hint(
    path: &Path,
    config: &ResolvedConfig,
    result: SyntaxCheckResult,
) -> SyntaxCheckResult {
    let mut diagnostics = match result {
        SyntaxCheckResult::Diagnostics(diagnostics) => diagnostics,
        result => return result,
    };
    let security = crate::runtime::resolve_render_security(config);
    for diagnostic in &mut diagnostics {
        if !crate::runtime::is_remote_access_failure(&diagnostic.message) {
            continue;
        }
        if let Some(hint) = crate::runtime::remote_include_policy_hint_at_line(
            path,
            diagnostic.range.start.line as usize + 1,
            &security,
            config.remote_includes,
            config.offline,
        ) {
            diagnostic.message = format!("Cannot open URL: {hint}");
        }
    }
    SyntaxCheckResult::Diagnostics(diagnostics)
}

fn syntax_command(
    path: &Path,
    config: &ResolvedConfig,
) -> Result<(tempfile::TempDir, CommandSpec), String> {
    let renderer =
        crate::runtime::configured_renderer(config).map_err(|error| error.to_string())?;
    let include_paths =
        crate::runtime::resolve_include_paths(config).map_err(|error| error.to_string())?;
    let syntax_output = create_syntax_output_dir().map_err(|error| error.to_string())?;
    let command = build_syntax_command(
        &renderer,
        &SyntaxRequest {
            input: path.to_path_buf(),
            output_dir: syntax_output.path().to_path_buf(),
            worktree_root: config.root.clone(),
            include_paths,
            security: crate::runtime::resolve_render_security(config),
        },
    )
    .map_err(|error| error.to_string())?;
    Ok((syntax_output, command))
}

fn run_syntax_command(
    command: &CommandSpec,
    timeout: Duration,
    cancellation: &CancellationToken,
) -> SyntaxCheckResult {
    match execute_cancellable(command, timeout, || cancellation.is_cancelled()) {
        Ok(output) => syntax_result(&output.stdout, &output.stderr, None),
        Err(ControlledProcessFailure::Cancelled) => SyntaxCheckResult::Cancelled,
        Err(ControlledProcessFailure::Process(ProcessFailure::Timeout { .. })) => {
            SyntaxCheckResult::TimedOut
        }
        Err(ControlledProcessFailure::Process(ProcessFailure::NonZero {
            code,
            stdout,
            stderr,
            ..
        })) => syntax_result(&stdout, &stderr, Some(code)),
        Err(ControlledProcessFailure::Process(error)) => {
            SyntaxCheckResult::Environment(error.to_string())
        }
    }
}

fn syntax_result(
    stdout: &[u8],
    stderr: &[u8],
    exit_code: Option<Option<i32>>,
) -> SyntaxCheckResult {
    let report = format!(
        "{}{}",
        String::from_utf8_lossy(stdout),
        String::from_utf8_lossy(stderr)
    );
    let diagnostics = parse_standard_report(&report);
    if !diagnostics.is_empty() {
        return SyntaxCheckResult::Diagnostics(diagnostics);
    }
    if exit_code.is_none() {
        return SyntaxCheckResult::Valid;
    }
    SyntaxCheckResult::Environment(if report.trim().is_empty() {
        let status = exit_code
            .flatten()
            .map_or_else(|| "unknown".to_string(), |status| status.to_string());
        format!("PlantUML syntax check exited with status {status}")
    } else {
        report.trim().to_string()
    })
}

pub fn run_stdio(config: ResolvedConfig) -> Result<(), AppError> {
    let (connection, io_threads) = Connection::stdio();
    let (initialize_id, initialize_params) =
        connection.initialize_start().map_err(protocol_error)?;
    let config = apply_initialization_options(config, &initialize_params)?;
    let initialization = serde_json::to_value(initialize_result()).map_err(|error| {
        AppError::environment(
            "lsp_initialize",
            format!("failed to serialize LSP capabilities: {error}"),
        )
    })?;
    connection
        .initialize_finish(initialize_id, initialization)
        .map_err(protocol_error)?;

    // A first managed install downloads both PlantUML and a pinned JRE. Finish
    // the LSP handshake before doing that work so the editor does not mistake
    // a legitimate first-use download for a language-server startup timeout.
    // Requests sent meanwhile remain queued until the renderer is ready.
    if let Err(error) = crate::runtime::prepare_lsp_renderer(&config) {
        drop(connection);
        let _ = io_threads.join();
        return Err(error);
    }

    let state = Arc::new(Mutex::new(LspState::default()));
    let checker: Arc<dyn SyntaxCheck> = Arc::new(PlantUmlSyntaxChecker);
    let structural_worker = StructuralWorker::spawn(Arc::clone(&state), connection.sender.clone());
    let export_worker = ExportWorker::spawn(config.clone(), connection.sender.clone());

    for message in &connection.receiver {
        match message {
            Message::Request(request) => {
                if connection
                    .handle_shutdown(&request)
                    .map_err(protocol_error)?
                {
                    break;
                }
                handle_request(request, &state, &config, &export_worker, &connection.sender);
            }
            Message::Notification(notification) => handle_notification(
                notification,
                &state,
                &checker,
                &config,
                &structural_worker,
                &connection,
            ),
            Message::Response(_) => {}
        }
    }

    state.lock().expect("LSP state poisoned").cancel_all();
    structural_worker.shutdown()?;
    export_worker.shutdown()?;
    drop(connection);
    io_threads
        .join()
        .map_err(|error| AppError::environment("lsp_stdio", format!("LSP stdio failed: {error}")))
}

fn handle_request(
    request: Request,
    state: &Arc<Mutex<LspState>>,
    config: &ResolvedConfig,
    export_worker: &ExportWorker,
    sender: &crossbeam_channel::Sender<Message>,
) {
    let Request { id, method, params } = request;
    match method.as_str() {
        "textDocument/codeAction" => match serde_json::from_value::<CodeActionParams>(params) {
            Ok(params) => {
                let actions = export_commands_for_document(&params.text_document.uri)
                    .into_iter()
                    .map(CodeActionOrCommand::Command)
                    .collect::<Vec<_>>();
                send_response(sender, Response::new_ok(id, actions));
            }
            Err(error) => send_response(
                sender,
                Response::new_err(
                    id,
                    ErrorCode::InvalidParams as i32,
                    format!("invalid textDocument/codeAction parameters: {error}"),
                ),
            ),
        },
        "workspace/executeCommand" => {
            let params = match serde_json::from_value::<ExecuteCommandParams>(params) {
                Ok(params) => params,
                Err(error) => {
                    send_response(
                        sender,
                        Response::new_err(
                            id,
                            ErrorCode::InvalidParams as i32,
                            format!("invalid workspace/executeCommand parameters: {error}"),
                        ),
                    );
                    return;
                }
            };
            if params.command != EXPORT_COMMAND {
                send_response(
                    sender,
                    Response::new_err(
                        id,
                        ErrorCode::MethodNotFound as i32,
                        format!("unknown request command: {}", params.command),
                    ),
                );
                return;
            }
            let export = match decode_export_command(&params.command, &params.arguments) {
                Ok(export) => export,
                Err(error) => {
                    send_app_error(sender, id, ErrorCode::InvalidParams, &error);
                    return;
                }
            };
            let input = match state
                .lock()
                .expect("LSP state poisoned")
                .saved_export_input(&export, &config.root)
            {
                Ok(input) => input,
                Err(error) => {
                    send_show_message(sender, MessageType::ERROR, &error.message);
                    send_app_error(sender, id, ErrorCode::RequestFailed, &error);
                    return;
                }
            };
            match export_worker.submit(ExportJob {
                id: id.clone(),
                input,
                format: export.format,
            }) {
                Ok(()) => {}
                Err(crossbeam_channel::TrySendError::Full(job)) => send_response(
                    sender,
                    Response::new_err(
                        job.id,
                        ErrorCode::ServerCancelled as i32,
                        "PlantUML export is already in progress; try again after it finishes"
                            .to_string(),
                    ),
                ),
                Err(crossbeam_channel::TrySendError::Disconnected(job)) => send_response(
                    sender,
                    Response::new_err(
                        job.id,
                        ErrorCode::InternalError as i32,
                        "PlantUML export worker is unavailable".to_string(),
                    ),
                ),
            }
        }
        _ => send_response(
            sender,
            Response::new_err(
                id,
                ErrorCode::MethodNotFound as i32,
                format!("unknown request: {method}"),
            ),
        ),
    }
}

fn spawn_export_worker(
    config: ResolvedConfig,
    receiver: crossbeam_channel::Receiver<ExportJob>,
    sender: crossbeam_channel::Sender<Message>,
    cancellation: crate::runtime::ExportCancellation,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        while !cancellation.is_cancelled() {
            let Ok(job) = receiver.recv() else {
                break;
            };
            if cancellation.is_cancelled() {
                break;
            }
            let result = crate::runtime::run_export_file_cancellable(
                &config,
                job.input,
                job.format,
                cancellation.clone(),
            )
            .and_then(require_successful_export);
            if cancellation.is_cancelled() {
                break;
            }
            match result {
                Ok(report) => {
                    let outputs = report
                        .succeeded
                        .iter()
                        .flat_map(|success| success.outputs.iter())
                        .cloned()
                        .collect::<Vec<_>>();
                    let message = if outputs.is_empty() {
                        "PlantUML export completed".to_string()
                    } else {
                        format!("Exported PlantUML to {}", outputs.join(", "))
                    };
                    send_show_message(&sender, MessageType::INFO, &message);
                    send_response(&sender, Response::new_ok(job.id, report));
                }
                Err(error) => {
                    send_show_message(&sender, MessageType::ERROR, &error.message);
                    send_app_error(&sender, job.id, ErrorCode::RequestFailed, &error);
                }
            }
        }
    })
}

fn require_successful_export(
    report: crate::export::ExportReport,
) -> Result<crate::export::ExportReport, AppError> {
    if report.failures.is_empty() {
        return Ok(report);
    }
    let message = report
        .failures
        .iter()
        .map(|failure| format!("{}: {}", failure.input, failure.message))
        .collect::<Vec<_>>()
        .join("; ");
    Err(AppError::operation("export_failed", message))
}

fn send_app_error(
    sender: &crossbeam_channel::Sender<Message>,
    id: RequestId,
    code: ErrorCode,
    error: &AppError,
) {
    send_response(
        sender,
        Response::new_err(
            id,
            code as i32,
            format!("{}: {}", error.code, error.message),
        ),
    );
}

fn send_show_message(sender: &crossbeam_channel::Sender<Message>, typ: MessageType, message: &str) {
    let notification = Notification::new(
        "window/showMessage".to_string(),
        ShowMessageParams {
            typ,
            message: message.to_string(),
        },
    );
    let _ = sender.send(Message::Notification(notification));
}

fn send_response(sender: &crossbeam_channel::Sender<Message>, response: Response) {
    let _ = sender.send(Message::Response(response));
}

fn handle_notification(
    notification: Notification,
    state: &Arc<Mutex<LspState>>,
    checker: &Arc<dyn SyntaxCheck>,
    config: &ResolvedConfig,
    structural_worker: &StructuralWorker,
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
                structural_worker.submit(job);
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
                    structural_worker.submit(job);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Layout, RendererMode};
    use lsp_types::{Position, Range};

    #[test]
    fn structural_worker_coalesces_a_change_burst_to_the_latest_document() {
        let state = Arc::new(Mutex::new(LspState::default()));
        let (messages, received) = crossbeam_channel::unbounded();
        let worker = StructuralWorker::spawn(Arc::clone(&state), messages);
        let uri = Url::parse("file:///workspace/burst.puml").unwrap();

        for version in 1..=1_000 {
            let text = format!("@startuml\nAlice -> Bob: {version}\n@enduml\n");
            let job = state.lock().unwrap().did_change(uri.clone(), text, version);
            worker.submit(job);
        }

        let message = received.recv_timeout(Duration::from_secs(2)).unwrap();
        let Message::Notification(notification) = message else {
            panic!("expected diagnostics notification");
        };
        let publish: PublishDiagnosticsParams =
            serde_json::from_value(notification.params).unwrap();
        assert_eq!(publish.version, Some(1_000));
        assert!(received.recv_timeout(Duration::from_millis(350)).is_err());
        worker.shutdown().unwrap();
    }

    fn config(root: PathBuf) -> ResolvedConfig {
        ResolvedConfig {
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

    fn diagnostic(message: &str) -> Diagnostic {
        Diagnostic {
            range: Range::new(Position::new(1, 0), Position::new(1, 1)),
            message: message.to_string(),
            ..Diagnostic::default()
        }
    }

    #[test]
    fn lsp_remote_access_diagnostics_become_actionable_policy_errors_only_when_blocked() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("model.puml");
        std::fs::write(
            &input,
            "@startuml\n!include http://127.0.0.1:8080/theme.puml\n@enduml\n",
        )
        .unwrap();
        let config = config(temp.path().to_path_buf());

        let blocked = apply_remote_policy_hint(
            &input,
            &config,
            SyntaxCheckResult::Diagnostics(vec![diagnostic("Cannot open URL")]),
        );
        let SyntaxCheckResult::Diagnostics(diagnostics) = blocked else {
            panic!("expected a client-visible policy diagnostic")
        };
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("allowedRemoteUrls"));
        assert_eq!(diagnostics[0].range.start.line, 1);

        let syntax = apply_remote_policy_hint(
            &input,
            &config,
            SyntaxCheckResult::Diagnostics(vec![diagnostic("Syntax Error")]),
        );
        assert!(matches!(syntax, SyntaxCheckResult::Diagnostics(_)));
    }
}
