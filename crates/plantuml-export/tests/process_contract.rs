use std::fs;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use url::Url;

#[test]
fn version_is_a_real_successful_command_with_a_stable_json_envelope() {
    let output = Command::new(env!("CARGO_BIN_EXE_plantuml-export"))
        .args(["--json", "version"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["schemaVersion"], 1);
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "version");
    assert_eq!(body["data"]["version"], env!("CARGO_PKG_VERSION"));
    assert!(body["error"].is_null());
}

#[test]
fn empty_export_is_a_successful_noop_without_installing_a_renderer() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join(".git")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_plantuml-export"))
        .args(["--json", "--root"])
        .arg(root.path())
        .arg("export")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["ok"], true);
    assert_eq!(body["command"], "export");
    assert_eq!(body["data"]["succeeded"], serde_json::json!([]));
    assert_eq!(body["data"]["failures"], serde_json::json!([]));
    assert!(body["error"].is_null());
}

#[test]
fn invalid_configuration_uses_exit_two_and_the_same_json_envelope() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join(".git")).unwrap();
    fs::write(
        root.path().join("plantuml-export.toml"),
        "defaultFormat = \"png\"\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_plantuml-export"))
        .args(["--json", "--root"])
        .arg(root.path())
        .args(["export", "model.puml"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["schemaVersion"], 1);
    assert_eq!(body["ok"], false);
    assert_eq!(body["command"], "export");
    assert_eq!(body["error"]["kind"], "usage");
    assert_eq!(body["error"]["code"], "config_schema");
}

#[test]
fn malformed_cli_arguments_use_exit_two() {
    let output = Command::new(env!("CARGO_BIN_EXE_plantuml-export"))
        .args(["export", "--format", "gif", "model.puml"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("invalid value 'gif'"));
}

#[test]
fn environment_failures_use_exit_two_while_operation_failures_use_exit_one() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join(".git")).unwrap();
    // Make the environment failure deterministic even when the machine already
    // has a healthy managed JAR cache and Java/Graphviz installation.
    fs::write(
        root.path().join("plantuml-export.toml"),
        "renderer = \"binary\"\n",
    )
    .unwrap();

    let health = Command::new(env!("CARGO_BIN_EXE_plantuml-export"))
        .args(["--json", "--root"])
        .arg(root.path())
        .arg("health")
        .output()
        .unwrap();
    let export = Command::new(env!("CARGO_BIN_EXE_plantuml-export"))
        .args(["--json", "--root"])
        .arg(root.path())
        .args(["export", "model.puml"])
        .output()
        .unwrap();

    assert_eq!(health.status.code(), Some(2));
    assert_eq!(export.status.code(), Some(1));
}

#[test]
fn export_check_health_and_partial_json_are_wired_to_the_native_runtime() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("repo");
    let config_home = fixture.path().join("config");
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::create_dir_all(config_home.join("plantuml-export")).unwrap();
    fs::write(root.join("good.puml"), "@startuml\nAlice -> Bob\n@enduml\n").unwrap();
    fs::write(root.join("bad.puml"), "@startuml\nFAIL\n@enduml\n").unwrap();
    fs::write(
        root.join("environment.puml"),
        "@startuml\nENVFAIL\n@enduml\n",
    )
    .unwrap();
    let renderer = compile_fake_renderer(fixture.path());
    let renderer_toml = renderer.to_string_lossy().replace('\\', "\\\\");
    fs::write(
        config_home.join("plantuml-export/config.toml"),
        format!("renderer = \"binary\"\nbinaryPath = \"{renderer_toml}\"\nlayout = \"smetana\"\n"),
    )
    .unwrap();

    let health = configured_command(&root, &config_home)
        .arg("health")
        .output()
        .unwrap();
    assert_eq!(health.status.code(), Some(0));
    let health: Value = serde_json::from_slice(&health.stdout).unwrap();
    assert_eq!(health["data"]["ready"], true);
    assert_eq!(health["data"]["graphviz"]["status"], "not_required");

    let export = configured_command(&root, &config_home)
        .args(["export", "--keep-going", "good.puml", "bad.puml"])
        .output()
        .unwrap();
    assert_eq!(export.status.code(), Some(1));
    let export: Value = serde_json::from_slice(&export.stdout).unwrap();
    assert_eq!(export["ok"], false);
    assert_eq!(export["error"]["kind"], "operation");
    assert_eq!(export["error"]["code"], "partial_export");
    assert_eq!(export["data"]["succeeded"].as_array().unwrap().len(), 1);
    assert_eq!(export["data"]["failures"].as_array().unwrap().len(), 1);
    assert!(root.join("out/good.svg").is_file());
    assert!(!root.join("out/bad.svg").exists());

    let check = configured_command(&root, &config_home)
        .args(["check", "bad.puml"])
        .output()
        .unwrap();
    assert_eq!(check.status.code(), Some(1));
    let check: Value = serde_json::from_slice(&check.stdout).unwrap();
    assert_eq!(check["error"]["code"], "syntax_errors");
    assert_eq!(check["data"]["failures"][0]["diagnostics"][0]["line"], 2);
    assert_eq!(
        check["data"]["failures"][0]["diagnostics"][0]["message"],
        "synthetic syntax error"
    );

    let environment = configured_command(&root, &config_home)
        .args(["check", "environment.puml"])
        .output()
        .unwrap();
    assert_eq!(environment.status.code(), Some(2));
    let environment: Value = serde_json::from_slice(&environment.stdout).unwrap();
    assert_eq!(environment["error"]["kind"], "environment");
    assert_eq!(environment["error"]["code"], "syntax_check_environment");
}

fn configured_command(root: &std::path::Path, config_home: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_plantuml-export"));
    command
        .args(["--json", "--root"])
        .arg(root)
        .env("HOME", config_home)
        .env("XDG_CONFIG_HOME", config_home)
        .env("XDG_STATE_HOME", config_home.join("state"))
        .env("APPDATA", config_home)
        .env("LOCALAPPDATA", config_home.join("local"));
    command
}

fn state_root(config_home: &std::path::Path) -> std::path::PathBuf {
    #[cfg(target_os = "macos")]
    {
        return config_home.join("Library/Application Support/plantuml-export");
    }
    #[cfg(target_os = "windows")]
    {
        return config_home.join("local/plantuml-export/state");
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        return config_home.join("state/plantuml-export");
    }
    #[allow(unreachable_code)]
    config_home.join("state/plantuml-export")
}

fn single_state_manifest(config_home: &std::path::Path) -> std::path::PathBuf {
    let exports = state_root(config_home).join("exports");
    let mut workspaces = fs::read_dir(&exports)
        .unwrap_or_else(|error| panic!("read state exports {}: {error}", exports.display()))
        .map(|entry| entry.expect("state workspace").path())
        .collect::<Vec<_>>();
    workspaces.sort();
    assert_eq!(workspaces.len(), 1, "one workspace state directory");
    workspaces.remove(0).join("manifest.json")
}

fn compile_fake_renderer(directory: &std::path::Path) -> std::path::PathBuf {
    let source = directory.join("fake-renderer.rs");
    let executable = directory.join(format!("fake-renderer{}", std::env::consts::EXE_SUFFIX));
    fs::write(
        &source,
        r#"
use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;

fn main() {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if args.iter().any(|arg| arg == OsStr::new("--version")) {
        println!("PlantUML fake 1.0");
        return;
    }
    let input = PathBuf::from(args.last().expect("input"));
    let text = fs::read_to_string(&input).expect("read input");
    if args.iter().any(|arg| arg == OsStr::new("-stdrpt:1")) {
        if text.contains("SLOW") {
            let marker = std::env::var_os("PLANTUML_EXPORT_TEST_MARKER").expect("marker");
            fs::write(marker, b"started").expect("write marker");
            std::thread::sleep(std::time::Duration::from_secs(3));
            let completed = std::env::var_os("PLANTUML_EXPORT_TEST_COMPLETED").expect("completed");
            fs::write(completed, b"completed").expect("write completion");
            return;
        }
        if text.contains("ENVFAIL") {
            eprintln!("synthetic runtime failure");
            std::process::exit(3);
        }
        if text.contains("FAIL") {
            eprintln!("protocolVersion=1");
            eprintln!("status=ERROR");
            eprintln!("lineNumber=2");
            eprintln!("label=synthetic syntax error");
            eprintln!("Error line 2 in file: {}", input.display());
            eprintln!("Some diagram description contains errors");
            std::process::exit(1);
        }
    }
    if text.contains("SLOW_EXPORT") {
        let marker = std::env::var_os("PLANTUML_EXPORT_TEST_MARKER").expect("marker");
        fs::write(marker, b"started").expect("write marker");
        std::thread::sleep(std::time::Duration::from_secs(2));
        let completed = std::env::var_os("PLANTUML_EXPORT_TEST_COMPLETED").expect("completed");
        fs::write(completed, b"completed").expect("write completion");
    }
    if text.contains("FAIL") {
        eprintln!("synthetic render failure");
        std::process::exit(1);
    }
    let output_index = args.iter().position(|arg| arg == OsStr::new("--output-dir")).unwrap();
    let format_index = args.iter().position(|arg| arg == OsStr::new("--format")).unwrap();
    let output_dir = PathBuf::from(&args[output_index + 1]);
    let format = args[format_index + 1].to_string_lossy();
    let output = output_dir.join(input.file_stem().unwrap()).with_extension(format.as_ref());
    fs::write(output, b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>").unwrap();
}
"#,
    )
    .unwrap();
    let output = Command::new("rustc")
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "failed to compile fake renderer: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    executable
}

#[test]
fn json_parse_errors_identify_the_command_not_a_global_option_value() {
    let output = Command::new(env!("CARGO_BIN_EXE_plantuml-export"))
        .args([
            "--json",
            "--root",
            "version",
            "export",
            "--format",
            "gif",
            "model.puml",
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["command"], "export");
}

#[test]
fn lsp_helper_exports_through_code_actions_without_a_path_cli() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("repo");
    let config_home = fixture.path().join("config");
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::create_dir_all(config_home.join("plantuml-export")).unwrap();
    let source = "@startuml\nAlice -> Bob\n@enduml\n";
    let input_path = root.join("model.puml");
    fs::write(&input_path, source).unwrap();
    let uri = Url::from_file_path(&input_path).unwrap();
    let renderer = compile_fake_renderer(fixture.path());
    let renderer_toml = renderer.to_string_lossy().replace('\\', "\\\\");
    fs::write(
        config_home.join("plantuml-export/config.toml"),
        format!("renderer = \"binary\"\nbinaryPath = \"{renderer_toml}\"\nlayout = \"smetana\"\n"),
    )
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_plantuml-export"))
        .arg("--root")
        .arg(&root)
        .arg("lsp")
        .env("HOME", &config_home)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_STATE_HOME", config_home.join("state"))
        .env("APPDATA", &config_home)
        .env("LOCALAPPDATA", config_home.join("local"))
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut child_stdout = child.stdout.take().unwrap();
    let (stdout_sender, stdout_receiver) = std::sync::mpsc::channel();
    let stdout_reader = thread::spawn(move || {
        let mut buffer = [0_u8; 4096];
        loop {
            match child_stdout.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    if stdout_sender.send(Ok(buffer[..read].to_vec())).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = stdout_sender.send(Err(error));
                    break;
                }
            }
        }
    });
    let input = [
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}),
        serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        serde_json::json!({
            "jsonrpc":"2.0",
            "method":"textDocument/didOpen",
            "params":{"textDocument":{"uri":uri,"languageId":"plantuml","version":1,"text":source}}
        }),
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":2,
            "method":"textDocument/codeAction",
            "params":{
                "textDocument":{"uri":uri},
                "range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},
                "context":{"diagnostics":[]}
            }
        }),
        serde_json::json!({
            "jsonrpc":"2.0",
            "id":3,
            "method":"workspace/executeCommand",
            "params":{
                "command":"plantuml-export.export",
                "arguments":[{"uri":uri,"format":"svg"}]
            }
        }),
    ];
    let stdin = child.stdin.as_mut().unwrap();
    stdin.write_all(lsp_frames(input).as_bytes()).unwrap();
    stdin.flush().unwrap();

    let exported = root.join("out/model.svg");
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut stdout_bytes = Vec::new();
    let mut stdout_read_error = None;
    let mut export_completed = false;
    while Instant::now() < deadline {
        let wait = deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(50));
        match stdout_receiver.recv_timeout(wait) {
            Ok(Ok(chunk)) => stdout_bytes.extend_from_slice(&chunk),
            Ok(Err(error)) => {
                stdout_read_error = Some(error);
                break;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
        let stdout = String::from_utf8_lossy(&stdout_bytes);
        if exported.is_file()
            && stdout.contains("Exported PlantUML to out/model.svg")
            && stdout.contains("\"id\":3")
        {
            export_completed = true;
            break;
        }
    }
    let shutdown = [
        serde_json::json!({"jsonrpc":"2.0","id":4,"method":"shutdown","params":null}),
        serde_json::json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ];
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(lsp_frames(shutdown).as_bytes()).unwrap();
    drop(stdin);

    let output = child.wait_with_output().unwrap();
    stdout_reader.join().unwrap();
    for chunk in stdout_receiver.try_iter() {
        match chunk {
            Ok(chunk) => stdout_bytes.extend_from_slice(&chunk),
            Err(error) => stdout_read_error = Some(error),
        }
    }

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(stdout_bytes).unwrap();
    assert!(
        stdout_read_error.is_none(),
        "failed to read LSP stdout: {stdout_read_error:?}"
    );
    assert!(
        export_completed,
        "LSP export did not complete before shutdown\nstdout:\n{}\nstderr:\n{}",
        stdout,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("\"textDocumentSync\":1"));
    assert!(stdout.contains("\"codeActionProvider\":true"));
    assert!(stdout.contains("\"plantuml-export.export\""));
    assert!(stdout.contains("Export PlantUML to SVG"));
    assert!(stdout.contains("Exported PlantUML to out/model.svg"));
    assert!(!stdout.contains("out/out"));
    assert!(!stdout.contains("completionProvider"));
    assert!(!stdout.contains("hoverProvider"));
    assert!(
        exported.is_file(),
        "LSP export did not create {}\nstdout:\n{}\nstderr:\n{}",
        exported.display(),
        stdout,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read(&exported).unwrap(),
        b"<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>"
    );
    let manifest: Value =
        serde_json::from_slice(&fs::read(single_state_manifest(&config_home)).unwrap()).unwrap();
    let manifest_output = &manifest["inputs"]["model.puml"]["formats"]["svg"]["outputs"][0];
    assert_eq!(manifest_output["path"], "out/model.svg");
    let sha256 = manifest_output["sha256"].as_str().expect("output SHA-256");
    assert_eq!(sha256.len(), 64);
    assert!(sha256
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
}

#[test]
fn managed_lsp_finishes_the_handshake_before_first_use_runtime_preparation() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("repo");
    let fake_home = fixture.path().join("home");
    let config_home = fixture.path().join("config");
    let cache_home = fixture.path().join("cache");
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::create_dir_all(&fake_home).unwrap();
    fs::create_dir_all(&config_home).unwrap();
    fs::create_dir_all(&cache_home).unwrap();
    fs::write(root.join("plantuml-export.toml"), "offline = true\n").unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_plantuml-export"))
        .arg("--root")
        .arg(&root)
        .arg("lsp")
        .env("HOME", &fake_home)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_CACHE_HOME", &cache_home)
        .env("APPDATA", &config_home)
        .env("LOCALAPPDATA", &cache_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin
        .write_all(
            lsp_frames([
                serde_json::json!({
                    "jsonrpc":"2.0",
                    "id":1,
                    "method":"initialize",
                    "params":{"capabilities":{}}
                }),
                serde_json::json!({
                    "jsonrpc":"2.0",
                    "method":"initialized",
                    "params":{}
                }),
            ])
            .as_bytes(),
        )
        .unwrap();
    thread::sleep(Duration::from_millis(50));
    drop(stdin);
    let output = child.wait_with_output().unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("textDocumentSync"),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("offline mode forbids download"),
        "unexpected startup error: {stderr}"
    );
    assert!(!stderr.contains("LSP protocol error"));
}

#[test]
fn lsp_startup_health_checks_java_with_smetana_and_not_graphviz() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("repo");
    let config_home = fixture.path().join("config");
    let jar = fixture.path().join("plantuml.jar");
    let missing_java = fixture.path().join("missing-java");
    let missing_graphviz = fixture.path().join("missing-dot");
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::create_dir_all(config_home.join("plantuml-export")).unwrap();
    fs::write(&jar, b"not empty").unwrap();
    let jar_toml = jar.to_string_lossy().replace('\\', "\\\\");
    let java_toml = missing_java.to_string_lossy().replace('\\', "\\\\");
    let graphviz_toml = missing_graphviz.to_string_lossy().replace('\\', "\\\\");
    fs::write(
        config_home.join("plantuml-export/config.toml"),
        format!(
            "renderer = \"jar\"\njarPath = \"{jar_toml}\"\njavaPath = \"{java_toml}\"\ngraphvizPath = \"{graphviz_toml}\"\n"
        ),
    )
    .unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_plantuml-export"))
        .arg("--root")
        .arg(&root)
        .arg("lsp")
        .env("XDG_CONFIG_HOME", &config_home)
        .env("APPDATA", &config_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin
        .write_all(
            lsp_frames([
                serde_json::json!({
                    "jsonrpc":"2.0",
                    "id":1,
                    "method":"initialize",
                    "params":{"capabilities":{}}
                }),
                serde_json::json!({
                    "jsonrpc":"2.0",
                    "method":"initialized",
                    "params":{}
                }),
            ])
            .as_bytes(),
        )
        .unwrap();
    thread::sleep(Duration::from_millis(50));
    drop(stdin);
    let output = child.wait_with_output().unwrap();

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("textDocumentSync"),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains(&missing_java.display().to_string()),
        "{stderr}"
    );
    assert!(
        stderr.contains("Graphviz: Smetana layout does not require Graphviz"),
        "{stderr}"
    );
    assert!(!stderr.contains(&missing_graphviz.display().to_string()));
    assert!(!stderr.contains("LSP protocol error"));
}

#[test]
fn lsp_shutdown_waits_for_cancelled_save_cleanup() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("repo");
    let config_home = fixture.path().join("config");
    let scratch = fixture.path().join("scratch");
    let marker = fixture.path().join("renderer-started");
    let completed = fixture.path().join("renderer-completed");
    let source = root.join("slow.puml");
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::create_dir_all(config_home.join("plantuml-export")).unwrap();
    fs::create_dir_all(&scratch).unwrap();
    fs::write(&source, "@startuml\nSLOW\n@enduml\n").unwrap();
    let renderer = compile_fake_renderer(fixture.path());
    let renderer_toml = renderer.to_string_lossy().replace('\\', "\\\\");
    fs::write(
        config_home.join("plantuml-export/config.toml"),
        format!("renderer = \"binary\"\nbinaryPath = \"{renderer_toml}\"\n"),
    )
    .unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_plantuml-export"))
        .arg("--root")
        .arg(&root)
        .arg("lsp")
        .env("XDG_CONFIG_HOME", &config_home)
        .env("APPDATA", &config_home)
        .env("TMPDIR", &scratch)
        .env("TMP", &scratch)
        .env("TEMP", &scratch)
        .env("PLANTUML_EXPORT_TEST_MARKER", &marker)
        .env("PLANTUML_EXPORT_TEST_COMPLETED", &completed)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let uri = Url::from_file_path(&source).unwrap();
    let initial = [
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}),
        serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":"plantuml","version":1,"text":"@startuml\nSLOW\n@enduml\n"}}}),
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didSave","params":{"textDocument":{"uri":uri}}}),
    ];
    let stdin = child.stdin.as_mut().unwrap();
    stdin.write_all(lsp_frames(initial).as_bytes()).unwrap();
    stdin.flush().unwrap();

    let wait_started = Instant::now();
    while !marker.exists() && wait_started.elapsed() < Duration::from_secs(5) {
        thread::sleep(Duration::from_millis(10));
    }
    if !marker.exists() {
        let _ = child.kill();
        let output = child.wait_with_output().unwrap();
        panic!(
            "save checker did not start: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let shutdown = [
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"shutdown","params":null}),
        serde_json::json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ];
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(lsp_frames(shutdown).as_bytes()).unwrap();
    drop(stdin);
    let output = child.wait_with_output().unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "LSP stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !completed.exists(),
        "renderer was not cancelled on shutdown"
    );
    assert_eq!(fs::read_dir(&scratch).unwrap().count(), 0);
}

#[test]
fn lsp_bounds_export_work_and_shutdown_cancels_the_active_job() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("repo");
    let config_home = fixture.path().join("config");
    let marker = fixture.path().join("export-started");
    let completed = fixture.path().join("export-completed");
    let source_text = "@startuml\nSLOW_EXPORT\n@enduml\n";
    let source = root.join("slow-export.puml");
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::create_dir_all(config_home.join("plantuml-export")).unwrap();
    fs::write(&source, source_text).unwrap();
    let renderer = compile_fake_renderer(fixture.path());
    let renderer_toml = renderer.to_string_lossy().replace('\\', "\\\\");
    fs::write(
        config_home.join("plantuml-export/config.toml"),
        format!("renderer = \"binary\"\nbinaryPath = \"{renderer_toml}\"\nlayout = \"smetana\"\n"),
    )
    .unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_plantuml-export"))
        .arg("--root")
        .arg(&root)
        .arg("lsp")
        .env("HOME", &config_home)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_STATE_HOME", config_home.join("state"))
        .env("APPDATA", &config_home)
        .env("LOCALAPPDATA", config_home.join("local"))
        .env("PLANTUML_EXPORT_TEST_MARKER", &marker)
        .env("PLANTUML_EXPORT_TEST_COMPLETED", &completed)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let uri = Url::from_file_path(&source).unwrap();
    let initial = [
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}),
        serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":"plantuml","version":1,"text":source_text}}}),
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"workspace/executeCommand","params":{"command":"plantuml-export.export","arguments":[{"uri":uri,"format":"svg"}]}}),
    ];
    let stdin = child.stdin.as_mut().unwrap();
    stdin.write_all(lsp_frames(initial).as_bytes()).unwrap();
    stdin.flush().unwrap();

    let wait_started = Instant::now();
    while !marker.exists() && wait_started.elapsed() < Duration::from_secs(5) {
        thread::sleep(Duration::from_millis(10));
    }
    if !marker.exists() {
        let _ = child.kill();
        let output = child.wait_with_output().unwrap();
        panic!(
            "export did not start\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let remaining = [
        serde_json::json!({"jsonrpc":"2.0","id":3,"method":"workspace/executeCommand","params":{"command":"plantuml-export.export","arguments":[{"uri":uri,"format":"svg"}]}}),
        serde_json::json!({"jsonrpc":"2.0","id":4,"method":"workspace/executeCommand","params":{"command":"plantuml-export.export","arguments":[{"uri":uri,"format":"svg"}]}}),
        serde_json::json!({"jsonrpc":"2.0","id":5,"method":"shutdown","params":null}),
        serde_json::json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ];
    let shutdown_started = Instant::now();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(lsp_frames(remaining).as_bytes()).unwrap();
    drop(stdin);
    let output = child.wait_with_output().unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "LSP stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        shutdown_started.elapsed() < Duration::from_secs(1),
        "shutdown waited for queued exports"
    );
    assert!(!completed.exists(), "active export was not cancelled");
    assert!(!root.join("out/slow-export.svg").exists());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"id\":4"), "stdout: {stdout}");
    assert!(stdout.contains("\"code\":-32802"), "stdout: {stdout}");
    assert!(stdout.contains("already in progress"), "stdout: {stdout}");
}

fn lsp_frames(messages: impl IntoIterator<Item = Value>) -> String {
    messages
        .into_iter()
        .map(|message| {
            let body = serde_json::to_vec(&message).unwrap();
            format!(
                "Content-Length: {}\r\n\r\n{}",
                body.len(),
                String::from_utf8(body).unwrap()
            )
        })
        .collect()
}
