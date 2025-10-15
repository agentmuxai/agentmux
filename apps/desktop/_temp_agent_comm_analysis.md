# Agent Communication Issue Analysis

## Problem
Spawned Claude agents are not receiving messages from the Desktop app.

## Root Cause
The embedded_claude.rs file watcher (`watch_messages`) only processes `Create` events from notify crate.
However, there are several issues:

### Issue 1: Race Condition
- Messages may be created BEFORE the agent spawn completes
- File watcher starts AFTER the agent is spawned
- Any messages written between spawn start and watcher initialization are missed

### Issue 2: File System Event Reliability  
- `notify::EventKind::Create` may not fire on all platforms consistently
- Windows may fire different events than Linux/Mac
- Some file operations trigger `Write` or `Modify` events instead of `Create`

### Issue 3: No Polling Fallback
- If file watcher misses an event, the message is never processed
- No periodic directory scanning to catch missed files

## Proposed Solutions

### Solution 1: Add Polling Fallback (Recommended)
Add periodic directory scanning (every 2-5 seconds) to catch any missed message files.

```rust
// In watch_messages function
let mut last_check = std::time::Instant::now();
const CHECK_INTERVAL: Duration = Duration::from_secs(3);

loop {
    // Check for new files periodically
    if last_check.elapsed() >= CHECK_INTERVAL {
        scan_messages_directory(&messages_dir, &stdin_tx, &instance_name).await?;
        last_check = std::time::Instant::now();
    }
    
    // Also process file system events
    tokio::select! {
        Some(event) = rx.recv() => {
            // Handle event
        }
        _ = tokio::time::sleep(CHECK_INTERVAL) => {
            // Timeout, loop will check directory
        }
    }
}
```

### Solution 2: Scan Directory on Startup
Before starting the watcher, scan the messages directory for any existing files:

```rust
// Before starting watcher
scan_messages_directory(&messages_dir, &stdin_tx, &instance_name).await?;

// Then start watching for new files
```

### Solution 3: Watch More Event Types
Instead of only `Create`, also watch `Write` and `Modify`:

```rust
match event.kind {
    notify::EventKind::Create(_) | 
    notify::EventKind::Modify(_) => {
        // Process message file
    }
    _ => {}
}
```

## Recommended Implementation
Combine all three solutions:
1. Scan directory on startup (catch existing messages)
2. Watch Create, Write, and Modify events (broader coverage)
3. Add polling fallback every 3 seconds (catch any missed events)

This provides defense-in-depth against all potential failure modes.
