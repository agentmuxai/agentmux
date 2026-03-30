// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// CEF UI thread tasks — placeholder for future post_task integration.
//
// NOTE: The wrap_task! + post_task(ThreadId::UI) approach crashes with the
// current CEF Rust bindings (v146). The Browser handle doesn't survive the
// cross-thread dispatch correctly. Until this is fixed upstream, host method
// calls that need the UI thread are handled via alternative approaches:
//
// - DevTools: opened via remote debugging protocol (port 9222)
// - Zoom: skipped in CEF, applied via CSS instead
// - Window ops (minimize/maximize): use Win32 APIs directly (safe from any thread)

