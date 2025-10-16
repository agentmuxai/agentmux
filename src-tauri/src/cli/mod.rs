// CLI module for AgentMux Desktop
// Handles command-line argument parsing and execution

pub mod parser;
pub mod handlers;
pub mod output;

pub use parser::{Cli, Command};
pub use output::{CliResponse, OutputFormat};
