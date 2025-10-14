# AgentMux Desktop Architecture

## Core Principle: Decouple Interface from Business Logic

**Rule**: ALL business logic MUST live in the `services/` layer. Interface adapters (CLI, Tauri commands) are thin wrappers that:
1. Parse input from their specific format
2. Call the service
3. Format the result for their specific output

## Architecture Pattern: One Operation, Three Interfaces

```
┌─────────────────────────────────────────────────────────────┐
│                    Services Layer                           │
│                  (src-tauri/src/services/)                  │
│                                                              │
│  • Pure business logic                                      │
│  • Fully unit tested                                        │
│  • No dependencies on interface types                       │
│  • Returns structured data (not formatted strings)          │
└─────────────────────────────────────────────────────────────┘
                           │
                           │ Called by
                           ↓
    ┌──────────────────────┬──────────────────────┬───────────────────┐
    │                      │                      │                   │
    │ 1. Binary CLI        │ 2. In-App CLI        │ 3. Direct UI      │
    │ (External)           │ (Hybrid)             │ (Internal)        │
    │                      │                      │                   │
    │ agentmux.exe         │ execute_cli_command  │ Tauri Commands    │
    │    ↓                 │    ↓                 │    ↓              │
    │ cli/handlers.rs      │ cli/handlers.rs      │ main.rs           │
    │ (thin wrapper)       │ (thin wrapper)       │ (thin wrapper)    │
    └──────────────────────┴──────────────────────┴───────────────────┘
```

## Example: Log Export

### ✅ CORRECT - Service Layer Pattern

**Service** (`services/logs.rs`):
```rust
pub fn export_logs(request: LogExportRequest) -> LogExportResult {
    // All business logic here
    let log_entries = collect_log_entries();
    let result = write_log_file(&path, &format, &log_entries);

    LogExportResult {
        success: result.is_ok(),
        output_path: path.display().to_string(),
        entries_count: log_entries.len(),
        error_message: result.err().map(|e| e.to_string()),
    }
}
```

**CLI Handler** (`cli/handlers.rs`):
```rust
async fn handle_log_action(action: LogAction) -> CliResponse {
    // 1. Parse CLI args to service request
    let request = LogExportRequest {
        output_path: output.map(PathBuf::from),
        format: LogFormat::from(export_format.as_str()),
    };

    // 2. Call service
    let result = export_logs(request);

    // 3. Format for CLI output
    if result.success {
        CliResponse::success(format!("✓ Logs exported to: {}", result.output_path), ...)
    } else {
        CliResponse::error(result.error_message.unwrap_or(...))
    }
}
```

**Tauri Command** (`main.rs`):
```rust
#[tauri::command]
async fn export_logs(output_path: Option<String>, format: String) -> Result<String, String> {
    // 1. Parse Tauri args to service request
    let request = LogExportRequest {
        output_path: output_path.map(PathBuf::from),
        format: LogFormat::from(format.as_str()),
    };

    // 2. Call service
    let result = export_logs_service(request);

    // 3. Format for JSON output
    if result.success {
        Ok(json!({ "output_path": result.output_path, ... }).to_string())
    } else {
        Err(result.error_message.unwrap_or(...))
    }
}
```

### ❌ WRONG - Business Logic in Handler

```rust
// DON'T DO THIS!
async fn handle_log_action(action: LogAction) -> CliResponse {
    // Business logic embedded in handler
    let mut log_entries = Vec::new();
    for log_dir in potential_log_dirs {
        if log_dir.exists() {
            // ... 50 lines of logic ...
        }
    }
    fs::write(&output_path, content)?;

    // This violates separation of concerns:
    // 1. Can't reuse from Tauri commands without duplication
    // 2. Hard to test (requires CLI parsing)
    // 3. Couples business logic to interface format
}
```

## Implementation Checklist

When adding a new feature:

- [ ] **Service Layer First**
  - [ ] Create `services/feature_name.rs`
  - [ ] Define request/response structs with `#[derive(Serialize, Deserialize)]`
  - [ ] Implement pure function: `pub fn operation(request: Request) -> Result`
  - [ ] Write unit tests in `#[cfg(test)] mod tests`
  - [ ] Add to `services/mod.rs`

- [ ] **CLI Handler** (if needed)
  - [ ] Add parser in `cli/parser.rs`
  - [ ] Add handler in `cli/handlers.rs`
  - [ ] Parse CLI args → Service request
  - [ ] Call service
  - [ ] Format result → `CliResponse`

- [ ] **Tauri Command** (if needed)
  - [ ] Add `#[tauri::command]` in `main.rs`
  - [ ] Parse Tauri args → Service request
  - [ ] Call service
  - [ ] Format result → JSON string
  - [ ] Register in `invoke_handler![]`

- [ ] **Documentation**
  - [ ] Update `services/README.md` with new service
  - [ ] Add usage examples for all 3 interfaces

## Directory Structure

```
src-tauri/src/
├── services/           # Core business logic (NO interface coupling)
│   ├── mod.rs
│   ├── logs.rs         # Example: Log export service
│   ├── agents.rs       # Future: Agent management service
│   └── README.md       # Architecture documentation
│
├── cli/                # CLI interface adapter
│   ├── mod.rs
│   ├── parser.rs       # Clap argument parsing
│   ├── handlers.rs     # Route to services
│   └── output.rs       # Format service results for CLI
│
├── main.rs             # Tauri interface adapter
│   └── (Tauri commands that call services)
│
├── lib.rs              # Library exports
│   └── pub mod services;
│
└── (other modules...)
```

## Benefits

1. **No Duplication**: Business logic written once, used three ways
2. **Testable**: Service functions are pure, easy to unit test
3. **Maintainable**: Changes to business logic in ONE place
4. **Type Safe**: Rust compiler enforces consistency
5. **Clear Ownership**: Services own business logic, handlers own formatting

## Anti-Patterns to Avoid

❌ **Business logic in CLI handlers**
❌ **Business logic in Tauri commands**
❌ **Mixing interface concerns (CLI args, JSON parsing) with business logic**
❌ **Returning formatted strings from services** (return structured data)
❌ **Services depending on CLI or Tauri types**

## Migration Guide

If you find existing code that violates this pattern:

1. Create service module with pure function
2. Extract business logic to service
3. Add tests to service
4. Update CLI handler to call service
5. Add Tauri command if needed
6. Verify all 3 interfaces work

See PR #12 for example refactoring.

## Questions?

See `services/README.md` for detailed architecture documentation and examples.
