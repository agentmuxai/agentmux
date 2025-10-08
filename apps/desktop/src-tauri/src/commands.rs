use crate::bus::BusMessage;
use tauri::State;
use crate::AppState;

#[tauri::command]
pub async fn get_recent_messages(
    state: State<'_, AppState>,
    limit: Option<usize>,
) -> Result<Vec<BusMessage>, String> {
    let manager_guard = state.bus_manager.lock().await;

    if let Some(manager) = manager_guard.as_ref() {
        let actual_limit = limit.unwrap_or(100);
        // Access the message history through the bus state
        // This requires exposing a method on BusManager
        Ok(vec![]) // Placeholder - will implement after exposing method
    } else {
        Ok(vec![])
    }
}
