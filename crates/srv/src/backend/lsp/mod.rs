// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Language Server Protocol support — Phase 1 of
// SPEC_EDITOR_LSP_AND_THEMES_2026-05-26.md.
//
// The backend is a **process supervisor + message proxy**. It doesn't
// understand LSP semantics (the frontend's LspClient does); it just
// spawns the server binary, frames messages on the wire, and routes
// JSON-RPC bodies in/out. One server per (workspace_root, language).
//
// Surface:
//   - LspSupervisor::start(args)     → spawn (or attach to) a server
//   - LspSupervisor::send(id, msg)   → forward an LSP message
//   - LspSupervisor::stop(id)        → decrement refcount, drop on zero
//
// Server-pushed messages (publishDiagnostics, $/progress, etc.) are
// broadcast via the EventBus as `lsp:message` events.

pub mod supervisor;
pub mod workspace;

pub use supervisor::{LspSupervisor, ServerId, StartArgs, StartResult};
