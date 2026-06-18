---
type: patch
---
Refactor: split websocket.rs inline command handlers into per-family submodules. websocket.rs shrinks from 2371 to 939 lines. Extracted: agent input/subprocess (agent_handlers.rs), shell exec/stop (shell_handlers.rs), editor/file-ops (editor_handlers.rs), LSP (lsp_handlers.rs). No logic changes (A8)