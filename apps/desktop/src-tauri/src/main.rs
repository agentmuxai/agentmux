// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bus;
mod watcher;
mod commands;
mod tauri_commands;

#[cfg(test)]
mod tests;

use agentmux_desktop::{cli, embedded_claude, ipc};
use tauri_commands::types::{AppState, BusConfig, AgentInfo};
use tauri_commands::export_logs;
use tauri_commands::send_message;
use bus::BusManager;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;
use watcher::{AgentMessage, CommandWatcher, FileWatcher};
// Import all Tauri commands from modules
use tauri_commands::{
    start_bus, stop_bus, get_connected_agents, get_bus_status, get_recent_messages,
    start_file_watcher, stop_file_watcher, start_command_watcher, stop_command_watcher,
    spawn_agent, get_agent_status, get_agent_output, list_agents,
    spawn_embedded_claude, send_claude_input, list_claude_instances,
    execute_cli_command,
};
use tauri_commands::cli::cli_command_to_ipc;



// ============================================================================
#[tokio::main]
async fn main() {
    use clap::Parser;
    use cli::parser::Cli;
    use cli::output::OutputFormat;

    // Parse CLI arguments
    let cli = Cli::parse();

    // Handle --version and --help (already handled by clap via parse())

    // Enable verbose logging if requested
    if cli.verbose {
        println!("[DEBUG] Verbose logging enabled");
        std::env::set_var("RUST_LOG", "debug");
    }

    // Check for existing instance BEFORE processing CLI command or launching GUI
    if let Ok(lock) = ipc::read_lock_file() {
        if !ipc::is_lock_stale(&lock) {
            // Valid lock file exists - another instance is running
            println!("[IPC] Found running instance (PID: {}, port: {})", lock.pid, lock.ipc_port);

            if let Some(ref command) = cli.command {
                // CLI command provided - forward to existing instance
                let ipc_command = cli_command_to_ipc(command);

                match ipc::send_ipc_command(ipc_command) {
                    Ok(response) => {
                        println!("{}", response.output);
                        std::process::exit(if response.success { 0 } else { 1 });
                    }
                    Err(e) => {
                        eprintln!("Failed to communicate with running instance: {}", e);
                        std::process::exit(1);
                    }
                }
            } else {
                // No CLI command - GUI launch attempt, focus existing window and exit
                println!("[IPC] GUI instance already running. Focusing existing window...");

                // Send a focus command via IPC
                let focus_command = ipc::IpcCommand {
                    command_type: "internal".to_string(),
                    action: "focus".to_string(),
                    args: std::collections::HashMap::new(),
                    caller_pid: Some(std::process::id()),
                };

                match ipc::send_ipc_command(focus_command) {
                    Ok(_) => {
                        println!("[IPC] Successfully focused existing window");
                        std::process::exit(0);
                    }
                    Err(e) => {
                        eprintln!("[IPC] Failed to focus existing window: {}", e);
                        std::process::exit(1);
                    }
                }
            }
        } else {
            // Stale lock, remove it
            println!("[IPC] Found stale lock file, removing...");
            let _ = ipc::remove_lock_file();
        }
    }
    // No lock file or stale lock - continue to start new instance

    let claude_instances_arc = Arc::new(Mutex::new(std::collections::HashMap::new()));

    let app_state = AppState {
        bus_manager: Arc::new(Mutex::new(None)),
        file_watcher: Arc::new(Mutex::new(None)),
        command_watcher: Arc::new(Mutex::new(None)),
        claude_instances: claude_instances_arc.clone(),
    };

    // Create CLI state wrapper pointing to same instances
    let cli_state = embedded_claude::ClaudeInstancesState {
        instances: claude_instances_arc.clone(),
    };

    // Execute CLI command if provided (before UI initialization)
    if let Some(command) = cli.command {
        let format: OutputFormat = cli.json.into();

        // Note: We pass None for state and app_handle since Tauri isn't initialized yet
        // This means CLI commands run in a limited mode before GUI starts
        // For full functionality, use --headless mode or run after app starts
        let result = cli::handlers::handle_command(
            command,
            format,
            None,
            None,
        ).await;

        // Print result
        println!("{}", result.format(&format));

        // Exit if in headless mode
        if cli.headless {
            std::process::exit(if result.success { 0 } else { 1 });
        }

        // If not headless, command output was printed but app continues to GUI
        // The UI will show the current state after command execution
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Start IPC server for single-instance communication
            if let Err(e) = ipc::start_ipc_server(app.handle().clone()) {
                eprintln!("[IPC] Failed to start IPC server: {}", e);
            }

            // Note: Lock file cleanup will happen on process exit via Drop trait
            // or can be handled by signal handlers in production

            Ok(())
        })
        .manage(app_state)
        .manage(cli_state)
        .invoke_handler(tauri::generate_handler![
            start_bus,
            stop_bus,
            get_connected_agents,
            get_bus_status,
            get_recent_messages,
            start_file_watcher,
            stop_file_watcher,
            send_message,
            start_command_watcher,
            stop_command_watcher,
            spawn_agent,
            get_agent_status,
            get_agent_output,
            list_agents,
            spawn_embedded_claude,
            send_claude_input,
            list_claude_instances,
            execute_cli_command,
            export_logs,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
