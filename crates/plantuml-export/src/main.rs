use std::ffi::OsString;
use std::process::ExitCode;

use clap::Parser;
use plantuml_export::cli::{Cli, Command};
use plantuml_export::config::{resolve, ConfigRequest};
use plantuml_export::lsp;
use plantuml_export::protocol::JsonEnvelope;
use plantuml_export::runtime::{self, CheckReport, HealthData};
use plantuml_export::AppError;
use serde::Serialize;
use serde_json::Value;

fn main() -> ExitCode {
    let code = run(std::env::args_os().collect());
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

fn run(args: Vec<OsString>) -> i32 {
    let json_requested = args.iter().any(|arg| arg == "--json");
    let command_hint = command_hint(&args);
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

    if matches!(cli.command, Command::Lsp) {
        return match lsp::run_stdio(config) {
            Ok(()) => 0,
            Err(error) => {
                report_error(false, command_name, &error);
                error.exit_code()
            }
        };
    }

    match dispatch(&cli.command, &config) {
        Ok(result) => report_result(cli.json, command_name, result),
        Err(error) => {
            report_error(cli.json, command_name, &error);
            error.exit_code()
        }
    }
}

fn dispatch(
    command: &Command,
    config: &plantuml_export::config::ResolvedConfig,
) -> Result<CommandResult, AppError> {
    match command {
        Command::Export(args) => {
            let report = runtime::run_export(config, args)?;
            let human = format_export_report(&report);
            let error = (!report.failures.is_empty()).then(|| {
                let code = if report.is_partial() {
                    "partial_export"
                } else {
                    "export_failed"
                };
                AppError::operation(
                    code,
                    format!(
                        "{} PlantUML input(s) failed to export",
                        report.failures.len()
                    ),
                )
            });
            CommandResult::new(report, human, error)
        }
        Command::Check(args) => {
            let report = runtime::run_check(config, args)?;
            let human = format_check_report(&report);
            let error = (!report.failures.is_empty()).then(|| {
                AppError::operation(
                    "syntax_errors",
                    format!(
                        "{} PlantUML input(s) contain syntax errors",
                        report.failures.len()
                    ),
                )
            });
            CommandResult::new(report, human, error)
        }
        Command::Health => {
            let health = runtime::run_health(config)?;
            let human = format_health(&health);
            let error = (!health.ready).then(|| {
                AppError::environment(
                    "renderer_unhealthy",
                    "one or more renderer prerequisites are unavailable or incompatible",
                )
            });
            CommandResult::new(health, human, error)
        }
        Command::Lsp => unreachable!("lsp is handled before result reporting"),
        Command::Version => unreachable!("version is handled before configuration"),
    }
}

struct CommandResult {
    data: Value,
    human: String,
    error: Option<AppError>,
}

impl CommandResult {
    fn new(data: impl Serialize, human: String, error: Option<AppError>) -> Result<Self, AppError> {
        let data = serde_json::to_value(data).map_err(|error| {
            AppError::environment(
                "result_serialization",
                format!("failed to serialize command result: {error}"),
            )
        })?;
        Ok(Self { data, human, error })
    }
}

fn report_result(json: bool, command: &str, result: CommandResult) -> i32 {
    match result.error {
        Some(error) => {
            if json {
                print_json(&JsonEnvelope::failure_with_data(
                    command,
                    &error,
                    result.data,
                ));
            } else {
                if !result.human.is_empty() {
                    println!("{}", result.human);
                }
                eprintln!("error: {error}");
            }
            error.exit_code()
        }
        None => {
            if json {
                print_json(&JsonEnvelope::success(command, result.data));
            } else if !result.human.is_empty() {
                println!("{}", result.human);
            }
            0
        }
    }
}

fn format_export_report(report: &plantuml_export::export::ExportReport) -> String {
    if report.succeeded.is_empty() && report.failures.is_empty() {
        return "No PlantUML inputs found.".to_string();
    }
    let mut lines = Vec::new();
    for success in &report.succeeded {
        lines.push(format!(
            "exported {} -> {}",
            success.input,
            success.outputs.join(", ")
        ));
    }
    for failure in &report.failures {
        lines.push(format!(
            "failed {} [{}]: {}",
            failure.input, failure.code, failure.message
        ));
    }
    lines.join("\n")
}

fn format_check_report(report: &CheckReport) -> String {
    if report.checked.is_empty() && report.failures.is_empty() {
        return "No PlantUML inputs found.".to_string();
    }
    let mut lines = report
        .checked
        .iter()
        .map(|input| format!("checked {input}"))
        .collect::<Vec<_>>();
    for failure in &report.failures {
        if failure.diagnostics.is_empty() {
            lines.push(format!(
                "failed {} [{}]: {}",
                failure.input, failure.code, failure.message
            ));
        } else {
            for diagnostic in &failure.diagnostics {
                lines.push(format!(
                    "{}:{}: {}",
                    failure.input, diagnostic.line, diagnostic.message
                ));
            }
        }
    }
    lines.join("\n")
}

fn format_health(health: &HealthData) -> String {
    format!(
        "renderer: {} ({})\nJava: {} ({})\nGraphviz: {} ({})",
        health.renderer.status,
        health.renderer.detail,
        health.java.status,
        health.java.detail,
        health.graphviz.status,
        health.graphviz.detail
    )
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
