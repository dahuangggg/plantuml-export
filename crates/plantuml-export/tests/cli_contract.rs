use std::path::PathBuf;

use clap::{CommandFactory, Parser};
use plantuml_export::cli::{Cli, Command, OutputFormat, RendererMode};

#[test]
fn parses_the_four_public_commands_and_internal_lsp_command() {
    for command in ["export", "check", "health", "version", "lsp"] {
        let cli = Cli::try_parse_from(["plantuml-export", command]).unwrap();
        assert_eq!(cli.command.name(), command);
    }
}

#[test]
fn parses_stable_export_flags_into_typed_overrides() {
    let cli = Cli::try_parse_from([
        "plantuml-export",
        "--json",
        "--root",
        "/workspace",
        "export",
        "--workspace",
        "--format",
        "pdf",
        "--out-dir",
        "build/diagrams",
        "--renderer",
        "jar",
        "--graphviz",
        "/tools/dot",
        "model.puml",
    ])
    .unwrap();

    assert!(cli.json);
    assert_eq!(cli.root, Some(PathBuf::from("/workspace")));
    let Command::Export(export) = cli.command else {
        panic!("expected export command")
    };
    assert!(export.workspace);
    assert_eq!(export.format, Some(OutputFormat::Pdf));
    assert_eq!(export.out_dir, Some(PathBuf::from("build/diagrams")));
    assert_eq!(export.renderer, Some(RendererMode::Jar));
    assert_eq!(export.graphviz_path, Some(PathBuf::from("/tools/dot")));
    assert_eq!(export.inputs, vec![PathBuf::from("model.puml")]);
}

#[test]
fn lsp_is_hidden_from_public_help() {
    let help = Cli::command().render_long_help().to_string();

    assert!(help.contains("export"));
    assert!(help.contains("check"));
    assert!(help.contains("health"));
    assert!(help.contains("version"));
    assert!(!help.contains("\n  lsp"));
    assert!(!help.contains("--security"));
}

#[test]
fn check_accepts_the_same_local_include_roots_as_export() {
    let cli = Cli::try_parse_from([
        "plantuml-export",
        "check",
        "--include-path",
        "shared/plantuml",
        "model.puml",
    ])
    .unwrap();

    let Command::Check(check) = cli.command else {
        panic!("expected check command")
    };
    assert_eq!(check.include_paths, vec![PathBuf::from("shared/plantuml")]);
}

#[test]
fn unreleased_security_flag_is_rejected_without_migration() {
    assert!(Cli::try_parse_from([
        "plantuml-export",
        "export",
        "--security",
        "allowlist",
        "model.puml",
    ])
    .is_err());
}
