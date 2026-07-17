use std::ffi::OsString;
use std::process::ExitCode;

use clap::Parser;
use plantuml_export::cli::{Cli, Command};
use plantuml_export::config::{resolve, ConfigRequest};
use plantuml_export::lsp;
use plantuml_export::protocol::JsonEnvelope;
use plantuml_export::AppError;
use serde::Serialize;

fn main() -> ExitCode {
    let code = run(std::env::args_os().collect());
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

fn run(args: Vec<OsString>) -> i32 {
    let json_requested = args.iter().any(|arg| arg == "--json");
    let command_hint = command_hint(&args);
    if let Some(error) = legacy_cli_error(&args) {
        report_error(json_requested, command_hint, &error);
        return error.exit_code();
    }
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => {
            let exit_code = error.exit_code();
            if exit_code == 0 {
                let _ = error.print();
                return 0;
            }
            if json_requested {
                let error = AppError::usage("invalid_arguments", error.to_string());
                report_error(true, command_hint, &error);
            } else {
                let _ = error.print();
            }
            return 2;
        }
    };

    let command_name = cli.command.name();
    if matches!(cli.command, Command::Version) {
        let version = VersionData {
            version: env!("CARGO_PKG_VERSION"),
        };
        if cli.json {
            print_json(&JsonEnvelope::success(command_name, version));
        } else {
            println!("plantuml-export {}", version.version);
        }
        return 0;
    }

    let config = match ConfigRequest::from_cli(&cli).and_then(resolve) {
        Ok(config) => config,
        Err(error) => {
            report_error(cli.json, command_name, &error);
            return error.exit_code();
        }
    };

    match dispatch(&cli.command, config) {
        Ok(()) => 0,
        Err(error) => {
            report_error(cli.json, command_name, &error);
            error.exit_code()
        }
    }
}

fn dispatch(
    command: &Command,
    config: plantuml_export::config::ResolvedConfig,
) -> Result<(), AppError> {
    match command {
        Command::Export(_) | Command::Check(_) => Err(AppError::operation(
            "not_implemented",
            format!(
                "{} is not implemented in this native toolchain foundation",
                command.name()
            ),
        )),
        Command::Health => Err(AppError::environment(
            "health_unavailable",
            "renderer health checks are not implemented in this native toolchain foundation",
        )),
        Command::Lsp => lsp::run_stdio(config),
        Command::Version => unreachable!("version is handled before configuration"),
    }
}

fn command_hint(args: &[OsString]) -> &'static str {
    let mut arguments = args.iter().skip(1).filter_map(|arg| arg.to_str());
    while let Some(argument) = arguments.next() {
        if matches!(argument, "--root" | "--config") {
            let _ = arguments.next();
            continue;
        }
        if argument.starts_with("--root=") || argument.starts_with("--config=") {
            continue;
        }
        match argument {
            "export" => return "export",
            "check" => return "check",
            "health" => return "health",
            "version" => return "version",
            "lsp" => return "lsp",
            _ => {}
        }
    }
    "unknown"
}

fn legacy_cli_error(args: &[OsString]) -> Option<AppError> {
    let arguments: Vec<&str> = args.iter().skip(1).filter_map(|arg| arg.to_str()).collect();
    for (index, argument) in arguments.iter().enumerate() {
        let legacy = match *argument {
            "--plantuml-version" | "--no-auto-download" | "--server-url" | "-t" | "-o" => {
                Some(*argument)
            }
            "--renderer" if arguments.get(index + 1) == Some(&"auto") => Some("--renderer auto"),
            "--renderer=auto" => Some("--renderer=auto"),
            _ => None,
        };
        if let Some(legacy) = legacy {
            return Some(AppError::usage(
                "legacy_cli",
                format!(
                    "legacy unreleased CLI configuration `{legacy}` is not supported; use the v0.1 explicit renderer and long-form export flags"
                ),
            ));
        }
    }
    None
}

fn report_error(json: bool, command: &str, error: &AppError) {
    if json {
        print_json(&JsonEnvelope::failure(command, error));
    } else {
        eprintln!("error: {error}");
    }
}

fn print_json(value: &impl Serialize) {
    match serde_json::to_string(value) {
        Ok(value) => println!("{value}"),
        Err(error) => eprintln!("error: failed to serialize JSON response: {error}"),
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
struct VersionData {
    version: &'static str,
}
