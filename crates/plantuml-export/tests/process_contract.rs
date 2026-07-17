use std::fs;
use std::io::Write;
use std::process::{Command, Stdio};

use serde_json::Value;

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
fn valid_but_unimplemented_export_is_an_operation_failure_not_a_false_success() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join(".git")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_plantuml-export"))
        .args(["--json", "--root"])
        .arg(root.path())
        .args(["export", "model.puml"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    let body: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["ok"], false);
    assert_eq!(body["command"], "export");
    assert_eq!(body["error"]["kind"], "operation");
    assert_eq!(body["error"]["code"], "not_implemented");
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
    assert_eq!(body["error"]["code"], "legacy_config");
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
fn legacy_unreleased_cli_flags_return_explicit_migration_errors() {
    for arguments in [
        vec!["export", "--renderer", "auto", "model.puml"],
        vec!["export", "--plantuml-version", "v1.2025.4", "model.puml"],
        vec!["export", "--no-auto-download", "model.puml"],
        vec![
            "export",
            "--server-url",
            "https://example.test",
            "model.puml",
        ],
        vec!["export", "-t", "png", "model.puml"],
        vec!["export", "-o", "out", "model.puml"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_plantuml-export"))
            .args(arguments)
            .output()
            .unwrap();

        assert_eq!(output.status.code(), Some(2));
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("legacy unreleased CLI configuration")
        );
    }
}

#[test]
fn lsp_command_runs_the_real_stdio_server_and_advertises_full_sync_only() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join(".git")).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_plantuml-export"))
        .arg("--root")
        .arg(root.path())
        .arg("lsp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let input = [
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"capabilities":{}}}),
        serde_json::json!({"jsonrpc":"2.0","method":"initialized","params":{}}),
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"shutdown","params":null}),
        serde_json::json!({"jsonrpc":"2.0","method":"exit","params":null}),
    ]
    .into_iter()
    .map(|message| {
        let body = serde_json::to_vec(&message).unwrap();
        format!("Content-Length: {}\r\n\r\n{}", body.len(), String::from_utf8(body).unwrap())
    })
    .collect::<String>();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();

    let output = child.wait_with_output().unwrap();

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("\"textDocumentSync\":1"));
    assert!(!stdout.contains("completionProvider"));
    assert!(!stdout.contains("hoverProvider"));
}
