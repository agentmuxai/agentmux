use super::types::AgentMessage;
use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{new_debouncer, DebouncedEventKind};
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

pub struct FileWatcher {
    messages_dir: PathBuf,
    app_handle: AppHandle,
    agent_id: Option<String>,
    _watcher: Option<RecommendedWatcher>,
}

impl FileWatcher {
    /// Create a new file watcher
    pub fn new(messages_dir: PathBuf, app_handle: AppHandle) -> Self {
        Self {
            messages_dir,
            app_handle,
            agent_id: None,
            _watcher: None,
        }
    }

    /// Set the agent ID to filter messages
    pub fn set_agent_id(&mut self, agent_id: String) {
        self.agent_id = Some(agent_id);
    }

    /// Start watching the messages directory
    pub fn start(&mut self) -> Result<(), String> {
        // Ensure messages directory exists
        if !self.messages_dir.exists() {
            std::fs::create_dir_all(&self.messages_dir)
                .map_err(|e| format!("Failed to create messages directory: {}", e))?;
        }

        println!(
            "📂 Watching messages directory: {}",
            self.messages_dir.display()
        );

        let app_handle = self.app_handle.clone();
        let messages_dir = self.messages_dir.clone();
        let agent_id = self.agent_id.clone();

        // Create a channel to receive file system events
        let (tx, rx) = channel();

        // Create debouncer to avoid duplicate events
        let mut debouncer = new_debouncer(Duration::from_millis(100), tx)
            .map_err(|e| format!("Failed to create debouncer: {}", e))?;

        // Watch the messages directory
        debouncer
            .watcher()
            .watch(&messages_dir, RecursiveMode::Recursive)
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
                                    if let Err(e) =
                                        Self::handle_new_message(&path, &app_handle, &agent_id)
                                    {
                                        eprintln!("Error handling message: {}", e);
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

        println!("✅ File watcher started successfully");

        Ok(())
    }

    /// Handle a new message file
    fn handle_new_message(
        path: &Path,
        app_handle: &AppHandle,
        agent_id: &Option<String>,
    ) -> Result<(), String> {
        // Read and parse the message file
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("Failed to read message file: {}", e))?;

        let message: AgentMessage = serde_json::from_str(&content)
            .map_err(|e| format!("Failed to parse message JSON: {}", e))?;

        // Filter by agent ID if set
        if let Some(ref id) = agent_id {
            if !message.is_for_agent(id) {
                return Ok(()); // Skip message not for this agent
            }
        }

        println!(
            "📨 New message: {} -> {} ({})",
            message.from.id, message.to, message.id
        );

        // Emit event to frontend
        app_handle
            .emit("message_received", &message)
            .map_err(|e| format!("Failed to emit event: {}", e))?;

        Ok(())
    }

    /// Stop watching (called on drop)
    pub fn stop(&mut self) {
        println!("🛑 File watcher stopped");
        self._watcher = None;
    }
}

impl Drop for FileWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}
