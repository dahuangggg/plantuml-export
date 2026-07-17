pub mod cli;
pub mod config;
pub mod diagnostics;
pub mod error;
pub mod lsp;
pub mod protocol;

pub use error::{AppError, ErrorKind};
