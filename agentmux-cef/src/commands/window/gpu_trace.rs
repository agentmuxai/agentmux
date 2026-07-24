// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// GPU memory tracing — dev-only diagnostics for #2218 (system commit charge
// growing with no corresponding process-level growth, i.e. a bucket not
// attributed to any process's own Private Bytes). Chromium's memory-infra
// tracing breaks GPU allocations into named categories instead of one opaque
// total; this pins that down to two RPC commands an operator/agent can
// trigger for an arbitrary-length capture window (unlike `--trace-startup`,
// which is bounded and requires restarting with the flag set before the
// capture window starts).
//
// Spec: docs/specs/SPEC_GPU_MEMORY_TRACING_SCAFFOLDING_2026_07_24.md §2.2.
// Not a user feature — no UI affordance. Diagnostics only, same tier as
// `toggle_devtools`/`inspect_element_at` (meta.rs), which this is modeled on.
//
// `cef::begin_tracing`/`cef::end_tracing` must run on the CEF UI thread, same
// as every other CEF-touching call in this codebase (see `post_show_dev_tools`
// / `ShowDevToolsTask`, `ui_tasks/window.rs`) — this handler is invoked from
// the IPC/Tokio task, not that thread, so both calls are marshaled via
// `post_task(ThreadId::UI, ...)` rather than called inline. That makes both
// RPC commands fire-and-forget: the real begin/end outcome is logged from
// inside each `Task::execute()`, not returned synchronously to the caller
// (matches `post_show_dev_tools`'s own fire-and-forget shape).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use cef::{
    post_task, rc::Rc, wrap_end_tracing_callback, wrap_task, CefString, EndTracingCallback,
    ImplEndTracingCallback, ImplTask, Task, ThreadId, WrapEndTracingCallback, WrapTask,
};

use crate::state::AppState;

/// Categories covering GPU allocations (`gpu`), compositor resources (`cc`),
/// Skia GPU resources, and GPU memory buffers — see the spec §1 for what each
/// one means and why the GPU process's own `gpu` category size is the number
/// that matters for #2218. `-*` first disables every default category so we
/// aren't also capturing full per-frame event timing, which would make an
/// hours-long capture enormous for no benefit to this investigation. This is
/// standard Chromium category-filter shorthand, not AgentMux-specific syntax
/// — Chromium's own startup-tracing docs use the identical `-*,<category>`
/// shape (`--trace-startup=-*,disabled-by-default-memory-infra`); multiple
/// included categories after `-*` is the same mechanism, just more of them.
const DEFAULT_TRACE_CATEGORIES: &str =
    "-*,disabled-by-default-memory-infra,gpu,cc,skia,disabled-by-default-gpu.memory";

/// Guards against overlapping begin/end calls. Module-local rather than an
/// `AppState` field — this is a rarely-used diagnostic toggle, not
/// app-lifecycle state, so it doesn't need to live alongside everything else
/// `AppState` tracks.
static TRACING_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Subdirectory of the instance data dir traces are written into — fixed,
/// not caller-controlled, so `end_gpu_trace`'s filename arg can never write
/// outside it (see `resolve_trace_path`).
const TRACE_SUBDIR: &str = "gpu-traces";

/// Reject unconditionally unless explicitly opted into dev mode. Diagnostics
/// only, no legitimate production use case — matches the runtime
/// `AGENTMUX_DEV=1` check already used for the dev-only GPU-tier switch in
/// `app/mod.rs`'s `on_before_command_line_processing` (`AGENTMUX_DEV`, not
/// `is_dev_self()`, since that checks build identity, not opt-in).
fn require_dev_mode() -> Result<(), String> {
    if std::env::var("AGENTMUX_DEV").is_ok() {
        Ok(())
    } else {
        Err("GPU tracing is dev-only — set AGENTMUX_DEV=1 to enable".to_string())
    }
}

/// Request a memory-infra GPU trace start. Optional `categories` arg
/// overrides `DEFAULT_TRACE_CATEGORIES`. Fire-and-forget — see module doc for
/// why: the actual `cef::begin_tracing` call happens on the UI thread via
/// `BeginGpuTraceTask`, and its real outcome is logged from there, not
/// returned here.
pub fn begin_gpu_trace(_state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    require_dev_mode()?;
    if TRACING_ACTIVE.swap(true, Ordering::SeqCst) {
        return Err("GPU trace already running — call end_gpu_trace first".to_string());
    }
    let categories = args
        .get("categories")
        .and_then(|v| v.as_str())
        .unwrap_or(DEFAULT_TRACE_CATEGORIES)
        .to_string();
    tracing::info!(categories = %categories, "gpu_trace: requesting begin_tracing on UI thread");
    let mut task = BeginGpuTraceTask::new(categories.clone());
    post_task(ThreadId::UI, Some(&mut task));
    Ok(serde_json::json!({ "requested": true, "categories": categories }))
}

/// Request the trace started by `begin_gpu_trace` be stopped and flushed to
/// `<instance data dir>/gpu-traces/<filename>`. `filename` (required arg) is
/// a bare file name, not a path — rejected outright if it contains a path
/// separator or `..`, so the write target is always confined to
/// `TRACE_SUBDIR` by construction rather than by validating an
/// attacker-suppliable path after the fact. Fire-and-forget for the same
/// UI-thread reason as `begin_gpu_trace`; completion (once every process has
/// flushed) is logged via `tracing::info!` from `GpuTraceEndCallback` rather
/// than surfaced over IPC (this is a diagnostics tool — the operator reads
/// the log / opens the resulting file directly, matching the spec's own
/// scoping — see §5, no UI affordance planned).
pub fn end_gpu_trace(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    require_dev_mode()?;
    let filename = args
        .get("filename")
        .and_then(|v| v.as_str())
        .ok_or("end_gpu_trace requires a \"filename\" arg (bare file name, not a path)")?;
    let path = resolve_trace_path(state, filename)?;
    if !TRACING_ACTIVE.swap(false, Ordering::SeqCst) {
        return Err("no GPU trace is running — call begin_gpu_trace first".to_string());
    }
    let path_str = path.to_string_lossy().into_owned();
    tracing::info!(path = %path_str, "gpu_trace: requesting end_tracing on UI thread");
    let mut task = EndGpuTraceTask::new(path_str.clone());
    post_task(ThreadId::UI, Some(&mut task));
    Ok(serde_json::json!({ "requested": true, "path": path_str }))
}

/// Build `<instance data dir>/gpu-traces/<filename>`, creating the
/// subdirectory if needed. `filename` must be a single path component — any
/// separator (`/`, `\`) or `..`/`.` special component is rejected, which
/// rules out escaping `TRACE_SUBDIR` regardless of what the caller passes
/// (no canonicalize-and-check-prefix dance needed, since a bare component
/// can't traverse anywhere).
fn resolve_trace_path(state: &Arc<AppState>, filename: &str) -> Result<std::path::PathBuf, String> {
    if filename.is_empty()
        || filename.contains(['/', '\\', '\0'])
        || filename == "."
        || filename == ".."
    {
        return Err(format!("invalid trace filename: {filename:?}"));
    }
    let data_dir = state
        .version_data_dir
        .lock()
        .clone()
        .ok_or("instance data dir not yet initialized")?;
    let dir = std::path::PathBuf::from(data_dir).join(TRACE_SUBDIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    Ok(dir.join(filename))
}

wrap_task! {
    pub struct BeginGpuTraceTask {
        categories: String,
    }

    impl Task {
        fn execute(&self) {
            let cef_categories = CefString::from(self.categories.as_str());
            let started = cef::begin_tracing(Some(&cef_categories), None) != 0;
            if started {
                tracing::info!(categories = %self.categories, "gpu_trace: started");
            } else {
                // Roll back the optimistic guard set in begin_gpu_trace so a
                // failed start doesn't permanently block future attempts.
                TRACING_ACTIVE.store(false, Ordering::SeqCst);
                tracing::warn!(categories = %self.categories, "gpu_trace: cef::begin_tracing returned failure");
            }
        }
    }
}

wrap_task! {
    pub struct EndGpuTraceTask {
        path: String,
    }

    impl Task {
        fn execute(&self) {
            let cef_path = CefString::from(self.path.as_str());
            let mut callback = GpuTraceEndCallback::new(self.path.clone());
            let stopping = cef::end_tracing(Some(&cef_path), Some(&mut callback)) != 0;
            if !stopping {
                tracing::warn!(path = %self.path, "gpu_trace: cef::end_tracing returned failure");
            } else {
                tracing::info!(path = %self.path, "gpu_trace: stopping, flush in progress");
            }
        }
    }
}

wrap_end_tracing_callback! {
    struct GpuTraceEndCallback {
        requested_path: String,
    }

    impl EndTracingCallback {
        fn on_end_tracing_complete(&self, tracing_file: Option<&CefString>) {
            let written = tracing_file
                .map(|f| f.to_string())
                .unwrap_or_else(|| self.requested_path.clone());
            let size_bytes = std::fs::metadata(&written).map(|m| m.len()).ok();
            tracing::info!(
                path = written,
                size_bytes,
                "gpu_trace: flush complete — open in chrome://tracing or ui.perfetto.dev"
            );
        }
    }
}
