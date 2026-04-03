// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

mod cli;
mod rpc;

use clap::Parser;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let cli = cli::Cli::parse();
    let exit_code = cli::dispatch(cli).await;
    std::process::exit(exit_code);
}
