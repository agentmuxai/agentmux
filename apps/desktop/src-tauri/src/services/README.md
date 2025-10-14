# Services Layer Architecture

## Design Principle: One Operation, Three Interfaces

All core business logic lives in the `services/` directory. Each service provides a single, well-tested operation that can be accessed through three different interfaces:

```
┌─────────────────────────────────────────────────────────────┐
│                    Core Services Layer                      │
│                  (services/logs.rs, etc.)                   │
│            • Business logic                                 │
│            • Unit tested                                    │
│            • No interface coupling                          │
└─────────────────────────────────────────────────────────────┘
                           │
                           │ Called by
                           ↓
    ┌──────────────────────┬──────────────────────┬───────────────────┐
    │                      │                      │                   │
    │ 1. Binary CLI        │ 2. In-App CLI        │ 3. Direct UI      │
    │ (External)           │ (Hybrid)             │ (Internal)        │
    │                      │                      │                   │
    │ agentmux-desktop.exe │ execute_cli_command  │ Tauri Commands    │
    │ logs export          │ "logs export"        │ export_logs()     │
    │    ↓                 │    ↓                 │    ↓              │
    │ cli/handlers.rs      │ cli/handlers.rs      │ main.rs           │
    │ (thin wrapper)       │ (thin wrapper)       │ (thin wrapper)    │
    └──────────────────────┴──────────────────────┴───────────────────┘
```

## Example: Log Export Service

### Core Service (`services/logs.rs`)
```rust
pub fn export_logs(request: LogExportRequest) -> LogExportResult {
    // All business logic here
    // Collects logs, formats output, writes file
    // Returns structured result
}
```

### Interface 1: Binary CLI (External)
```bash
# User runs from terminal
agentmux-desktop.exe logs export --format json --output my-logs.json
```
- Handled by: `cli/handlers.rs::handle_log_action()`
- Creates `LogExportRequest` from CLI args
- Calls `services::logs::export_logs()`
- Formats `LogExportResult` for terminal output

### Interface 2: In-App CLI (Hybrid)
```javascript
// User types in embedded console UI
await invoke('execute_cli_command', {
  command_str: 'logs export --format json'
})
```
- Handled by: `main.rs::execute_cli_command()`
- Parses command string → routes to `cli/handlers.rs`
- Same path as Interface 1
- Returns formatted text to UI

### Interface 3: Direct UI (Internal)
```javascript
// UI calls Tauri command directly
await invoke('export_logs', {
  output_path: 'my-logs.json',
  format: 'json'
})
```
- Handled by: `main.rs::export_logs()`
- Creates `LogExportRequest` from parameters
- Calls `services::logs::export_logs()`
- Returns JSON result to UI

## Benefits

1. **Single Source of Truth**: Business logic in one place
2. **Testable**: Services are pure functions, easy to unit test
3. **Flexible Access**: Users can choose their preferred interface
4. **No Duplication**: All interfaces share the same implementation
5. **Type Safety**: Rust ensures consistency across interfaces

## Adding New Services

1. Create service module: `services/my_feature.rs`
2. Implement core function: `pub fn my_operation(request: Request) -> Result`
3. Add unit tests in `#[cfg(test)] mod tests`
4. Add CLI handler in `cli/handlers.rs`
5. Add Tauri command in `main.rs`
6. Register command in `tauri::Builder::invoke_handler`

## Testing

```bash
# Run all service tests
cargo test --lib services::

# Run specific service tests
cargo test --lib services::logs

# With coverage
cargo test --lib services:: --coverage
```

## Current Services

- **logs** - Log export to text/JSON formats
- _(Add more services here as they're created)_
