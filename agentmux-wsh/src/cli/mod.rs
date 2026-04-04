// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! CLI command definitions and dispatch.

mod blocks;
mod conn;
mod file;
mod info;
mod meta;
mod term;
mod vars;
mod view;

use clap::{Parser, Subcommand};

use crate::rpc::RpcClient;

#[derive(Parser)]
#[command(name = "wsh", about = "CLI tool to control AgentMux")]
pub struct Cli {
    /// Block ID to operate on
    #[arg(short, long, global = true)]
    pub block: Option<String>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Show version information
    Version(info::VersionArgs),

    /// Get block/object metadata
    Getmeta(meta::GetMetaArgs),

    /// Set block/object metadata
    Setmeta(meta::SetMetaArgs),

    /// Get a variable value
    Getvar(vars::GetVarArgs),

    /// Set a variable value
    Setvar(vars::SetVarArgs),

    /// Block management commands
    #[command(subcommand)]
    Blocks(blocks::BlocksCommand),

    /// Create a new terminal block
    Term(term::TermArgs),

    /// File operations
    #[command(subcommand)]
    File(file::FileCommand),

    /// Connection management
    #[command(subcommand)]
    Conn(conn::ConnCommand),

    /// View management
    View(view::ViewArgs),

    /// Open a web view
    Web(view::WebArgs),

    /// Desktop notification
    Notify(info::NotifyArgs),

    /// Set background
    Setbg(info::SetbgArgs),

    /// Edit configuration
    Editconfig,

    /// Set configuration values
    Setconfig(info::SetconfigArgs),

    /// Wave path utilities
    Wavepath(info::WavepathArgs),

    /// Workspace management
    Workspace(info::WorkspaceArgs),

    /// AI integration
    Ai(info::AiArgs),

    /// Editor integration
    Editor(view::EditorArgs),

    /// Launch application
    Launch(view::LaunchArgs),

    /// Run a command
    Run(info::RunArgs),

    /// SSH connection
    Ssh(conn::SshArgs),

    /// WSL integration
    Wsl(conn::WslArgs),

    /// Debug utilities
    #[command(subcommand)]
    Debug(info::DebugCommand),

    /// Read a file
    Readfile(file::ReadfileArgs),
}

/// Connect to the backend and dispatch the CLI command.
pub async fn dispatch(cli: Cli) -> i32 {
    match cli.command {
        // Version doesn't need RPC
        Command::Version(args) => {
            info::cmd_version(&args);
            return 0;
        }
        _ => {}
    }

    // All other commands need an RPC connection
    let client = match RpcClient::connect().await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("wsh: failed to connect to backend: {}", e);
            return 1;
        }
    };

    let result = match cli.command {
        Command::Version(_) => unreachable!(),
        Command::Getmeta(args) => meta::cmd_getmeta(&client, args, cli.block).await,
        Command::Setmeta(args) => meta::cmd_setmeta(&client, args, cli.block).await,
        Command::Getvar(args) => vars::cmd_getvar(&client, args).await,
        Command::Setvar(args) => vars::cmd_setvar(&client, args).await,
        Command::Blocks(cmd) => blocks::cmd_blocks(&client, cmd).await,
        Command::Term(args) => term::cmd_term(&client, args).await,
        Command::File(cmd) => file::cmd_file(&client, cmd).await,
        Command::Conn(cmd) => conn::cmd_conn(&client, cmd).await,
        Command::View(args) => view::cmd_view(&client, args).await,
        Command::Web(args) => view::cmd_web(&client, args).await,
        Command::Notify(args) => info::cmd_notify(&client, args).await,
        Command::Setbg(args) => info::cmd_setbg(&client, args).await,
        Command::Editconfig => info::cmd_stub("editconfig").await,
        Command::Setconfig(args) => info::cmd_setconfig(&client, args).await,
        Command::Wavepath(args) => info::cmd_wavepath(&client, args).await,
        Command::Workspace(args) => info::cmd_workspace(&client, args).await,
        Command::Ai(args) => info::cmd_ai(&client, args).await,
        Command::Editor(args) => view::cmd_editor(&client, args).await,
        Command::Launch(args) => view::cmd_launch(&client, args).await,
        Command::Run(args) => info::cmd_run(&client, args).await,
        Command::Ssh(args) => conn::cmd_ssh(&client, args).await,
        Command::Wsl(args) => conn::cmd_wsl(&client, args).await,
        Command::Debug(cmd) => info::cmd_debug(&client, cmd).await,
        Command::Readfile(args) => file::cmd_readfile(&client, args).await,
    };

    match result {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("wsh: {}", e);
            1
        }
    }
}
