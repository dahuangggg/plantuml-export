pub mod cli;
pub mod config;
pub mod diagnostics;
pub mod discovery;
pub mod error;
pub mod export;
pub mod export_state;
pub mod lsp;
pub mod process_control;
pub mod protocol;
pub mod renderer;
pub mod runtime;

pub use error::{AppError, ErrorKind};
