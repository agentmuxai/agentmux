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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use cef::{
    rc::Rc, wrap_end_tracing_callback, CefString, EndTracingCallback, ImplEndTracingCallback,
    WrapEndTracingCallback,
};

use crate::state::AppState;

/// Categories covering GPU allocations (`gpu`), compositor resources (`cc`),
/// Skia GPU resources, and GPU memory buffers — see the spec §1 for what each
/// one means and why the GPU process's own `gpu` category size is the number
/// that matters for #2218. `-*` first disables every default category so we
/// aren't also capturing full per-frame event timing, which would make an
/// hours-long capture enormous for no benefit to this investigation.
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

/// Start a memory-infra GPU trace. Optional `categories` arg overrides
/// `DEFAULT_TRACE_CATEGORIES`. Fire-and-forget: `cef::begin_tracing`'s own
/// completion callback (fired once every process has started tracing) isn't
/// wired up here — for a manually-triggered diagnostics capture the small
/// delay before every process is actually recording doesn't matter, and it
/// keeps this handler synchronous like its siblings in this module.
pub fn begin_gpu_trace(_state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    require_dev_mode()?;
    if TRACING_ACTIVE.swap(true, Ordering::SeqCst) {
        return Err("GPU trace already running — call end_gpu_trace first".to_string());
    }
    let categories = args
        .get("categories")
        .and_then(|v| v.as_str())
        .unwrap_or(DEFAULT_TRACE_CATEGORIES);
    let cef_categories = CefString::from(categories);
    let started = cef::begin_tracing(Some(&cef_categories), None) != 0;
    if !started {
        TRACING_ACTIVE.store(false, Ordering::SeqCst);
        return Err("cef::begin_tracing returned failure".to_string());
    }
    tracing::info!(categories, "gpu_trace: started");
    Ok(serde_json::json!({ "started": true, "categories": categories }))
}

/// Stop the trace started by `begin_gpu_trace` and flush every process's
/// trace data to `<instance data dir>/gpu-traces/<filename>`. `filename`
/// (required arg) is a bare file name, not a path — rejected outright if it
/// contains a path separator or `..`, so the write target is always confined
/// to `TRACE_SUBDIR` by construction rather than by validating an
/// attacker-suppliable path after the fact. Completion is asynchronous — CEF
/// calls back once every process has written its data, which this logs via
/// `tracing::info!` rather than surfacing over IPC (this is a diagnostics
/// tool; the operator reads the log / opens the resulting file directly,
/// matching the spec's own scoping — see §5, no UI affordance planned).
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
    let cef_path = CefString::from(path_str.as_str());
    let mut callback = GpuTraceEndCallback::new(path_str.clone());
    let stopping = cef::end_tracing(Some(&cef_path), Some(&mut callback)) != 0;
    if !stopping {
        tracing::warn!(path = path_str, "gpu_trace: cef::end_tracing returned failure");
        return Err("cef::end_tracing returned failure".to_string());
    }
    tracing::info!(path = path_str, "gpu_trace: stopping, flush in progress");
    Ok(serde_json::json!({ "stopping": true, "path": path_str }))
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
