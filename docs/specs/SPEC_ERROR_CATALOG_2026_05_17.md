# SPEC: Global Error Code/Message Catalog

**Status:** Draft
**Date:** 2026-05-17
**Author:** AgentA
**Trigger:** Out-of-disk-space incident surfaced as `sqlite error: disk I/O error` instead of an actionable "Device out of space, free up disk and retry" message. Audit (§2) confirms this is the symptom of three structural gaps: no error code enum at the Rust boundary, no translation table at the RPC layer, no catalog at the frontend.

---

## 1. Problem

AgentMux surfaces errors to the user as opaque text, raw library exceptions, or numeric codes with no translation. The recent incident is one of many possible:

| Trigger | What the user sees today | What we want |
|---|---|---|
| Disk full during SQLite write | `sqlite error: disk I/O error` | "Device out of space. Free ~100 MB and retry." |
| `npm install` fails network | `spawn npm: ... exit 1` | "Couldn't reach npm registry. Check your connection." |
| Provider CLI missing | `<cmd> not available — install manually` | "Claude Code isn't installed. Click *Install now*." |
| Auth subprocess wants TTY | `Error: requires interactive TTY` | "OpenClaw needs a terminal session — use the *Connect* button in the launch modal." |
| CEF window crash | `Error: {text} (-1234)` | "Render process crashed (CEF code -1234). Reopen the pane." |
| Launcher startup port conflict | `bind 127.0.0.1:0: 10048` | "Another AgentMux instance is using port 10048. Close it or restart." |

These all flow through different paths today. Some are stringly-typed `Err(format!())`s; some are silent log lines; some are numeric codes from CEF / OS APIs. There is no central catalog the frontend can use to pick a friendly message.

## 2. Audit

Performed 2026-05-17. Full results in commit history; condensed here.

**Existing partial structure:**
- `agentmux-srv/src/backend/storage/error.rs` — `StoreError` enum (`NotFound`, `AlreadyExists`, `VersionMismatch`, `Sqlite`, `Json`, `Other(String)`). Isolated to the persistence layer; never translated to user-facing messages.
- `agentmux-srv/src/backend/rpc/engine.rs` — one ad-hoc error code prefix `"EC-TIME"` for timeouts. Everything else returns `String`.
- Frontend has `try { await RpcApi.foo() } catch (e) { setError(e.message) }` in ~12 places — displays whatever raw backend string arrived.

**Gaps (severity ranked):**

| Gap | Severity | Where |
|---|---|---|
| No structured error code at the Rust→TS boundary | **P0** | `RpcEngine::HandlerResult = Result<_, String>` (engine.rs:28) |
| No translation table for `std::io::Error` / `rusqlite::Error` | **P0** | `storage/filestore/core.rs`, `cli_handlers.rs`, etc. |
| Frontend renders raw backend strings verbatim | **P0** | `AgentInstallModal.tsx:125`, `PreLaunchAuthPanel.tsx:243`, `launch-flow.ts:120` |
| Silent failures in sidecar lifecycle (event-log rotation, cache eviction) | **P1** | `event_log.rs:203-220`, `filestore/core.rs:90-95` |
| Launcher startup errors only reach stderr / splash, not the running UI | **P1** | `agentmux-launcher/src/main.rs:89-100` |
| CEF host shows raw numeric error codes | **P2** | `agentmux-cef/src/client.rs` (CEF callback) |
| No central catalog doc for developers | **P2** | (does not exist) |

## 3. Goals

1. **Single source of truth.** One enum that all error-producing code paths map to, defined once and re-used across srv / cef / launcher / frontend.
2. **Stable codes.** Every error gets a string code like `AMX-IO-001` that survives renaming and refactors. Codes are the contract between layers; the human-facing message is presentation.
3. **Translation at the edge, not at the source.** Backend code returns the structured enum. The RPC boundary serializes it. The frontend looks up the message + recovery hint in a parallel TS catalog. Library/OS errors are translated into the catalog at the first layer that handles them (e.g. `std::io::Error` → `AgentMuxError::OutOfSpace` at the storage layer).
4. **Recoverability hints.** Each entry carries an optional retry suggestion ("free up disk and retry", "click *Install now*", "restart AgentMux"). Encoded once, rendered consistently.
5. **Incremental adoption.** The existing `Result<T, String>` API stays callable; new code uses the typed variant. The RPC engine accepts both during the migration window.
6. **Observability.** Codes flow into structured logs (`tracing` field `amx_code = ...`) so support requests can be triaged by code, not by free-text "what does this error mean."

## 4. Non-goals

- **Localization (i18n).** Catalog is English-only initially. The architecture leaves room for translation tables but we don't ship them in this PR series.
- **End-to-end retry mechanism.** Recovery hints are user-facing text only. Programmatic retry policies (exponential backoff, dead-letter, etc.) are a separate concern.
- **Migrating every existing error site.** First PR ships the scaffold + the I/O slice that actually hurts users today. Other layers migrate incrementally.

## 5. Design

### 5.1 The Rust enum

Lives in `agentmux-common/src/errors.rs` so all three Rust crates (srv, cef, launcher) can depend on it.

```rust
#[derive(Debug, thiserror::Error, serde::Serialize)]
#[serde(tag = "code", content = "details")]
pub enum AgentMuxError {
    // ── Filesystem / I/O ────────────────────────────────────
    #[error("device out of space writing {path}")]
    #[serde(rename = "AMX-IO-001")]
    OutOfSpace { path: String, source_msg: String },

    #[error("permission denied accessing {path}")]
    #[serde(rename = "AMX-IO-002")]
    PermissionDenied { path: String, source_msg: String },

    #[error("path not found: {path}")]
    #[serde(rename = "AMX-IO-003")]
    PathNotFound { path: String },

    #[error("path traversal blocked: {path}")]
    #[serde(rename = "AMX-IO-004")]
    PathTraversal { path: String },

    // ── Persistence (extends StoreError) ────────────────────
    #[error("schema migration {from}→{to} failed: {message}")]
    #[serde(rename = "AMX-STORE-001")]
    MigrationFailed { from: u32, to: u32, message: String },

    #[error("optimistic-lock version mismatch on {oid}")]
    #[serde(rename = "AMX-STORE-002")]
    VersionMismatch { oid: String, expected: u64, actual: u64 },

    // ── Provider CLI ────────────────────────────────────────
    #[error("CLI {cli} not installed for provider {provider}")]
    #[serde(rename = "AMX-CLI-001")]
    CliNotInstalled { provider: String, cli: String },

    #[error("npm install failed for {package}: {message}")]
    #[serde(rename = "AMX-CLI-002")]
    NpmInstallFailed { package: String, message: String },

    #[error("installed CLI shim missing: {expected_path}")]
    #[serde(rename = "AMX-CLI-003")]
    CliShimMissing { provider: String, expected_path: String },

    // ── Auth ────────────────────────────────────────────────
    #[error("OAuth subprocess requires an interactive TTY")]
    #[serde(rename = "AMX-AUTH-001")]
    AuthRequiresTty { provider: String },

    #[error("OAuth login timed out after {seconds}s")]
    #[serde(rename = "AMX-AUTH-002")]
    AuthTimeout { provider: String, seconds: u64 },

    // ── Network ─────────────────────────────────────────────
    #[error("HTTP request failed: {message}")]
    #[serde(rename = "AMX-NET-001")]
    HttpError { url: String, status: Option<u16>, message: String },

    // ── Lifecycle ───────────────────────────────────────────
    #[error("sidecar bind failed on port {port}: {message}")]
    #[serde(rename = "AMX-LIFECYCLE-001")]
    SidecarBindFailed { port: u16, message: String },

    #[error("single-instance lock held by pid {pid}")]
    #[serde(rename = "AMX-LIFECYCLE-002")]
    AlreadyRunning { pid: u32 },

    // ── Fallback (for un-migrated handlers) ─────────────────
    #[error("{0}")]
    #[serde(rename = "AMX-LEGACY")]
    Legacy(String),
}
```

**`From<std::io::Error>` impl** routes by `ErrorKind`:

```rust
impl From<std::io::Error> for AgentMuxError {
    fn from(e: std::io::Error) -> Self {
        use std::io::ErrorKind::*;
        match e.kind() {
            // Note: stable since Rust 1.83 — fallback to OS code check below.
            _ if e.raw_os_error() == Some(28) /* ENOSPC */
              || e.raw_os_error() == Some(112) /* ERROR_DISK_FULL */ =>
                AgentMuxError::OutOfSpace { path: String::new(), source_msg: e.to_string() },
            PermissionDenied => AgentMuxError::PermissionDenied { path: String::new(), source_msg: e.to_string() },
            NotFound => AgentMuxError::PathNotFound { path: String::new() },
            _ => AgentMuxError::Legacy(e.to_string()),
        }
    }
}
```

The `path` field is empty when the conversion happens implicitly; call sites that have the path use a helper:

```rust
AgentMuxError::from_io_with_path(&path, err)
```

### 5.2 RPC engine boundary

`agentmux-srv/src/backend/rpc/engine.rs` changes:

- `HandlerResult` becomes `Result<Option<serde_json::Value>, AgentMuxError>`.
- On error, the engine serializes the enum to JSON:
  ```json
  {
    "code": "AMX-IO-001",
    "message": "device out of space writing C:/Users/.../objects.db",
    "details": { "path": "C:/Users/.../objects.db", "source_msg": "..." }
  }
  ```
- Handlers can return `Err(amx_error)` directly OR keep returning `Err(String)` during migration; the engine wraps un-typed strings in `AgentMuxError::Legacy`.

### 5.3 Frontend translator

`frontend/app/errors/catalog.ts`:

```ts
export interface ErrorEntry {
    title: string;
    message: (details: Record<string, unknown>) => string;
    retry?: string;
}

export const ERROR_CATALOG: Record<string, ErrorEntry> = {
    "AMX-IO-001": {
        title: "Device out of space",
        message: (d) => `Couldn't write to ${d.path ?? "disk"} — no space left.`,
        retry: "Free up some space and try again.",
    },
    "AMX-IO-002": {
        title: "Permission denied",
        message: (d) => `AgentMux can't access ${d.path ?? "that file"}.`,
        retry: "Check folder permissions or run as the user that created it.",
    },
    "AMX-CLI-001": {
        title: "CLI not installed",
        message: (d) => `${d.provider} isn't installed yet.`,
        retry: "Click *Install now* in the agent picker.",
    },
    // ...mirrors the Rust enum
};
```

A small hook:

```ts
export function translateError(raw: unknown): { title: string; message: string; retry?: string } {
    if (typeof raw === "object" && raw !== null && "code" in raw) {
        const entry = ERROR_CATALOG[(raw as { code: string }).code];
        if (entry) {
            const details = (raw as { details?: Record<string, unknown> }).details ?? {};
            return {
                title: entry.title,
                message: entry.message(details),
                retry: entry.retry,
            };
        }
    }
    // Legacy fallback — raw string or unknown shape.
    const msg = raw instanceof Error ? raw.message : String(raw);
    return { title: "Something went wrong", message: msg };
}
```

And a banner component `<ErrorBanner code={...} />` that renders:
- title (bold, error icon)
- message
- retry hint (italicized, secondary text)
- code (small monospace, bottom right — for support requests)

### 5.4 Logging integration

When `AgentMuxError` is returned through the engine, `tracing` emits a structured field:

```rust
tracing::warn!(
    target: "amx::error",
    amx_code = %code,
    handler = %cmd_name,
    "rpc handler returned typed error"
);
```

Grafana / log search can then group support requests by `amx_code` instead of by free-text message.

## 6. Migration plan

Three PRs:

### PR 1 — Scaffold

- `agentmux-common/src/errors.rs` — the enum + serde derives + `From<std::io::Error>`.
- `agentmux-common/src/errors_test.rs` — unit tests covering the OS-code routing and serde round-trip.
- `agentmux-srv` RPC engine: accept both `Result<_, AgentMuxError>` and `Result<_, String>` handlers (wraps strings in `AMX-LEGACY`).
- `frontend/app/errors/catalog.ts` + `translateError` + `<ErrorBanner>`.
- One use site: the install modal's failure path migrates to render `<ErrorBanner code={...} />` for the `AMX-IO-*` and `AMX-CLI-*` codes the install handler now returns. Everything else keeps the legacy renderer.
- Tests: install handler returns `AMX-IO-001` on simulated disk-full; install modal renders the friendly message.

### PR 2 — Migrate the I/O surface

Convert every `std::io::Error` and `rusqlite::Error` to `AgentMuxError` at the layer that first handles them — primarily `storage/filestore/core.rs`, `cli_handlers.rs`, `install_handlers.rs`, `identity_handlers.rs`. Includes the silent-failure sites in `event_log.rs` (they now log with `amx_code` even though they don't propagate).

### PR 3 — Cover the rest

CLI, auth, network, and lifecycle categories. Migrate the launcher's startup errors so they reach the renderer (via the splash IPC channel) instead of stderr only.

Each PR keeps the `Legacy` fallback so un-migrated handlers continue to work. The fallback is removed only when grep finds no `Result<_, String>` returns from RPC handlers — likely 6-12 months out.

## 7. Open questions

- **Code prefix.** `AMX-` is succinct; alternatives are `AGNT-`, `AM-`, or no prefix. Decided to use `AMX-` for grep-ability (already used in `AGENTMUX_*` env vars).
- **Numeric vs string codes.** Strings (`AMX-IO-001`) are easier to read in logs and bug reports than `0x1001`. Trade-off: strings are slightly heavier to ship; we accept that.
- **Whether to include the raw source error in the JSON.** Yes — under `details.source_msg` for power users who want the underlying SQL error / OS error. Hidden in the UI by default (collapsed under a *Details* disclosure).
- **i18n.** Out of scope for now. Catalog is a `Record<string, ErrorEntry>`; an i18n layer can wrap `entry.message` later.

## 8. Documentation

Companion page in `agentmux-docs` will be added as `internals/error-catalog.md` once PR 1 lands — covering the categories, how to add new codes, the migration story.

## 9. References

- Audit findings: this spec §2 (full audit transcript in PR description).
- Existing `StoreError`: `agentmux-srv/src/backend/storage/error.rs`.
- `thiserror` crate: https://docs.rs/thiserror (already a workspace dep).
- The Rust API guidelines on error types: https://rust-lang.github.io/api-guidelines/interoperability.html#error-types-are-meaningful-and-well-behaved-c-good-err.
