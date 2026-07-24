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

/// Start a memory-infra GPU trace. Optional `categories` arg overrides
/// `DEFAULT_TRACE_CATEGORIES`. Fire-and-forget: `cef::begin_tracing`'s own
/// completion callback (fired once every process has started tracing) isn't
/// wired up here — for a manually-triggered diagnostics capture the small
/// delay before every process is actually recording doesn't matter, and it
/// keeps this handler synchronous like its siblings in this module.
pub fn begin_gpu_trace(_state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
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
/// trace data to `path` (required arg). Completion is asynchronous — CEF
/// calls back once every process has written its data, which this logs via
/// `tracing::info!` rather than surfacing over IPC (this is a diagnostics
/// tool; the operator reads the log / opens the resulting file directly,
/// matching the spec's own scoping — see §5, no UI affordance planned).
pub fn end_gpu_trace(_state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or("end_gpu_trace requires a \"path\" arg")?
        .to_string();
    if !TRACING_ACTIVE.swap(false, Ordering::SeqCst) {
        return Err("no GPU trace is running — call begin_gpu_trace first".to_string());
    }
    let cef_path = CefString::from(path.as_str());
    let mut callback = GpuTraceEndCallback::new(path.clone());
    let stopping = cef::end_tracing(Some(&cef_path), Some(&mut callback)) != 0;
    if !stopping {
        tracing::warn!(path, "gpu_trace: cef::end_tracing returned failure");
        return Err("cef::end_tracing returned failure".to_string());
    }
    tracing::info!(path, "gpu_trace: stopping, flush in progress");
    Ok(serde_json::json!({ "stopping": true, "path": path }))
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
