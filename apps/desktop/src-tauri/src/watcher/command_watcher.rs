use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Command {
    pub id: String,
    pub command: String,
    pub params: serde_json::Value,
    pub timestamp: String,
}

pub struct CommandWatcher {
    commands_dir: PathBuf,
    app_handle: AppHandle,
    _watcher: Option<RecommendedWatcher>,
}

impl CommandWatcher {
    /// Create a new command watcher
    pub fn new(commands_dir: PathBuf, app_handle: AppHandle) -> Self {
        Self {
            commands_dir,
            app_handle,
            _watcher: None,
        }
    }

    /// Start watching the commands directory
    pub fn start(&mut self) -> Result<(), String> {
        // Ensure commands directory exists
        if !self.commands_dir.exists() {
            std::fs::create_dir_all(&self.commands_dir)
                .map_err(|e| format!("Failed to create commands directory: {}", e))?;
        }

        println!(
            "📂 Watching commands directory: {}",
            self.commands_dir.display()
        );

        let app_handle = self.app_handle.clone();
        let commands_dir = self.commands_dir.clone();

        // Create a channel to receive file system events
        let (tx, rx) = channel();

        // Create debouncer to avoid duplicate events
        let mut debouncer = new_debouncer(Duration::from_millis(100), tx)
            .map_err(|e| format!("Failed to create debouncer: {}", e))?;

        // Watch the commands directory
        debouncer
            .watcher()
            .watch(&commands_dir, RecursiveMode::Recursive)
            .map_err(|e| format!("Failed to watch directory: {}", e))?;

        // Spawn a task to handle file events
        std::thread::spawn(move || {
            for result in rx {
                match result {
                    Ok(events) => {
                        for event in events {
                            if let DebouncedEventKind::Any = event.kind {
                                let path = &event.path;
                                if path.extension().and_then(|s| s.to_str()) == Some("json") {
                                    if let Err(e) = Self::handle_command(&path, &app_handle) {
                                        eprintln!("Error handling command: {}", e);
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Watch error: {:?}", e);
                    }
                }
            }
        });

        println!("✅ Command watcher started successfully");

        Ok(())
    }

    /// Handle a new command file
    fn handle_command(path: &Path, app_handle: &AppHandle) -> Result<(), String> {
        // Read and parse the command file
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read command file: {}", e))?;

        let command: Command = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse command JSON: {}", e))?;

        println!(
            "🎛️  New command: {} ({})",
            command.command, command.id
        );

        // Emit event to frontend
        app_handle
            .emit("cli_command", &command)
            .map_err(|e| format!("Failed to emit event: {}", e))?;

        // Delete the command file after processing
        std::fs::remove_file(path)
            .map_err(|e| format!("Failed to delete command file: {}", e))?;

        Ok(())
    }

    /// Stop watching (called on drop)
    pub fn stop(&mut self) {
        println!("🛑 Command watcher stopped");
        self._watcher = None;
    }
}

impl Drop for CommandWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}
