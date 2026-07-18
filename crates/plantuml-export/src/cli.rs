use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

#[derive(Debug, Parser)]
#[command(
    name = "plantuml-export",
    version,
    about = "Native PlantUML export and diagnostics"
)]
pub struct Cli {
    /// Emit the stable machine-readable JSON response envelope.
    #[arg(long, global = true)]
    pub json: bool,

    /// Resolve project inputs and relative outputs from this worktree root.
    #[arg(long, global = true, value_name = "PATH")]
    pub root: Option<PathBuf>,

    /// Use this project configuration file instead of <root>/plantuml-export.toml.
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Export one or more saved PlantUML source files.
    Export(ExportArgs),
    /// Check syntax for one or more saved PlantUML source files.
    Check(CheckArgs),
    /// Report local renderer and dependency health.
    Health,
    /// Print the native toolchain version.
    Version,
    /// Run the native export and diagnostic language server over stdio.
    #[command(hide = true)]
    Lsp,
}

impl Command {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Export(_) => "export",
            Self::Check(_) => "check",
            Self::Health => "health",
            Self::Version => "version",
            Self::Lsp => "lsp",
        }
    }
}

#[derive(Debug, Args)]
pub struct ExportArgs {
    /// Saved PlantUML source files to export.
    #[arg(value_name = "INPUT")]
    pub inputs: Vec<PathBuf>,

    /// Discover supported PlantUML source files below the worktree root.
    #[arg(long)]
    pub workspace: bool,

    /// Fail when input discovery produces no source files.
    #[arg(long)]
    pub require_input: bool,

    /// Continue independent inputs after an export failure.
    #[arg(long)]
    pub keep_going: bool,

    #[arg(long, value_enum)]
    pub format: Option<OutputFormat>,

    #[arg(long, value_name = "PATH")]
    pub out_dir: Option<PathBuf>,

    #[arg(long, value_enum)]
    pub renderer: Option<RendererMode>,

    #[arg(long, value_enum)]
    pub layout: Option<Layout>,

    /// Add a local PlantUML include search root.
    #[arg(long = "include-path", value_name = "PATH")]
    pub include_paths: Vec<PathBuf>,

    /// Include workspace source files matching this glob.
    #[arg(long = "include", value_name = "GLOB")]
    pub include: Vec<String>,

    /// Exclude workspace source files matching this glob.
    #[arg(long = "exclude", value_name = "GLOB")]
    pub exclude: Vec<String>,

    /// Java executable used by the explicit jar renderer.
    #[arg(long = "java", value_name = "PATH")]
    pub java_path: Option<PathBuf>,

    /// PlantUML executable used by binary renderer mode.
    #[arg(long = "plantuml", value_name = "PATH")]
    pub binary_path: Option<PathBuf>,

    /// PlantUML jar used by jar renderer mode.
    #[arg(long = "jar", value_name = "PATH")]
    pub jar_path: Option<PathBuf>,

    /// Graphviz dot executable used by the graphviz layout.
    #[arg(long = "graphviz", value_name = "PATH")]
    pub graphviz_path: Option<PathBuf>,

    /// Disable all managed renderer network access.
    #[arg(long)]
    pub offline: bool,

    /// Opt in to PlantUML source metadata in generated outputs.
    #[arg(long, conflicts_with = "disable_metadata")]
    pub embed_source_metadata: bool,

    /// Explicitly disable PlantUML source metadata (the default).
    #[arg(long, conflicts_with = "embed_source_metadata")]
    pub disable_metadata: bool,
}

#[derive(Debug, Args)]
pub struct CheckArgs {
    /// Saved PlantUML source files to check.
    #[arg(value_name = "INPUT")]
    pub inputs: Vec<PathBuf>,

    /// Discover supported PlantUML source files below the worktree root.
    #[arg(long)]
    pub workspace: bool,

    /// Fail when input discovery produces no source files.
    #[arg(long)]
    pub require_input: bool,

    /// Add a local PlantUML include search root.
    #[arg(long = "include-path", value_name = "PATH")]
    pub include_paths: Vec<PathBuf>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum RendererMode {
    Managed,
    Binary,
    Jar,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Svg,
    Png,
    Pdf,
}

impl OutputFormat {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Svg => "svg",
            Self::Png => "png",
            Self::Pdf => "pdf",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum Layout {
    Graphviz,
    Smetana,
}
