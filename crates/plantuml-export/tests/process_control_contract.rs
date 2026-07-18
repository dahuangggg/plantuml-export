use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use plantuml_export::process_control::{
    execute, execute_cancellable, CommandSpec, ControlledProcessFailure, ProcessFailure,
    MAX_CAPTURE_BYTES,
};

#[test]
fn child_stdin_is_null_and_each_output_stream_is_bounded() {
    let fixture = tempfile::tempdir().unwrap();
    let helper = compile_process_helper(fixture.path());

    let stdin = execute(&command(&helper, &["stdin"]), Duration::from_secs(5)).unwrap();
    assert_eq!(stdin.stdout, b"0");

    let flood = execute(&command(&helper, &["flood"]), Duration::from_secs(5)).unwrap();
    assert_eq!(flood.stdout.len(), MAX_CAPTURE_BYTES);
    assert_eq!(flood.stderr.len(), MAX_CAPTURE_BYTES);
    assert!(flood.stdout.ends_with(b"...[output truncated]\n"));
    assert!(flood.stderr.ends_with(b"...[output truncated]\n"));
}

#[test]
fn cancellation_terminates_the_process_tree_not_only_the_direct_child() {
    let fixture = tempfile::tempdir().unwrap();
    let helper = compile_process_helper(fixture.path());
    let ready = fixture.path().join("ready");
    let survivor = fixture.path().join("survivor");
    let command = command(
        &helper,
        &["tree", ready.to_str().unwrap(), survivor.to_str().unwrap()],
    );

    let result = execute_cancellable(&command, Duration::from_secs(5), || ready.exists());

    assert!(matches!(result, Err(ControlledProcessFailure::Cancelled)));
    std::thread::sleep(Duration::from_millis(800));
    assert!(
        !survivor.exists(),
        "a descendant survived cancellation and wrote {}",
        survivor.display()
    );
}

#[test]
fn timeout_terminates_the_process_tree_not_only_the_direct_child() {
    let fixture = tempfile::tempdir().unwrap();
    let helper = compile_process_helper(fixture.path());
    let ready = fixture.path().join("ready");
    let survivor = fixture.path().join("survivor");
    let command = command(
        &helper,
        &["tree", ready.to_str().unwrap(), survivor.to_str().unwrap()],
    );

    let result = execute(&command, Duration::from_millis(150));

    assert!(matches!(result, Err(ProcessFailure::Timeout { .. })));
    std::thread::sleep(Duration::from_millis(800));
    assert!(
        !survivor.exists(),
        "a descendant survived timeout and wrote {}",
        survivor.display()
    );
}

fn command(program: &Path, args: &[&str]) -> CommandSpec {
    CommandSpec {
        program: program.to_path_buf(),
        args: args.iter().map(std::ffi::OsString::from).collect(),
        env: Vec::new(),
        env_remove: Vec::new(),
    }
}

fn compile_process_helper(directory: &Path) -> PathBuf {
    let source = directory.join("process-helper.rs");
    let executable = directory.join(format!("process-helper{}", std::env::consts::EXE_SUFFIX));
    fs::write(
        &source,
        r#"
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.first().map(String::as_str) {
        Some("stdin") => {
            let mut input = Vec::new();
            std::io::stdin().read_to_end(&mut input).unwrap();
            print!("{}", input.len());
        }
        Some("flood") => {
            let bytes = vec![b'x'; 1024 * 1024 + 64 * 1024];
            std::io::stdout().write_all(&bytes).unwrap();
            std::io::stderr().write_all(&bytes).unwrap();
        }
        Some("tree") => {
            let ready = PathBuf::from(&args[1]);
            let survivor = PathBuf::from(&args[2]);
            Command::new(std::env::current_exe().unwrap())
                .arg("survivor")
                .arg(survivor)
                .stdin(Stdio::null())
                .spawn()
                .unwrap();
            std::fs::write(ready, b"ready").unwrap();
            std::thread::sleep(Duration::from_secs(10));
        }
        Some("survivor") => {
            std::thread::sleep(Duration::from_millis(600));
            std::fs::write(&args[1], b"survived").unwrap();
        }
        _ => panic!("unknown helper mode"),
    }
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
        "failed to compile process helper: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    executable
}
