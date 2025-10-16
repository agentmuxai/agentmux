# Debug Logging Guide for Message Communication

See the detailed logging instructions in the agentmux repository.

## Files Modified

1. **apps/desktop/src/App.tsx** - Status bar now uses environment variables
2. **apps/desktop/vite.config.ts** - Defines VITE_APP_VERSION and VITE_BUILD_TIME

## Recommended Next Steps

1. Manually add println! logging to:
   - apps/desktop/src-tauri/src/main.rs (send_message function)
   - apps/desktop/src-tauri/src/embedded_claude.rs (watch_messages function)

2. Build and run the app
3. Send a test message
4. Copy ALL console output
5. Analyze where the message flow breaks

Key checkpoints to log:
- Message file creation
- File watcher initialization  
- File event detection
- JSON parsing
- Message matching
- stdin forwarding
