// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Build script for agentmux-common.
//!
//! Its only job is to make `option_env!` reads reliable. `data_paths.rs`
//! bakes the default data channel from `AGENTMUX_BUILD_CHANNEL_DEFAULT`
//! at compile time via `option_env!`. Cargo does NOT track env vars
//! consumed by `option_env!` unless a build script tells it to — so
//! without this, switching the channel between builds (a different
//! branch, or a `--fresh` local package) would be served a STALE baked
//! channel from the incremental cache, and the running app would write
//! to the wrong data dir.
//!
//! We deliberately do NOT track `AGENTMUX_BUILD_LABEL`: that value
//! changes on every single build (it carries a per-build timestamp), so
//! tracking it would force a full recompile of this foundational crate
//! every package run and destroy incremental caching. The label is a
//! runtime concern for the packaging script (artifact naming), not a
//! compile-time bake, so it doesn't need cargo tracking here.

fn main() {
    println!("cargo:rerun-if-env-changed=AGENTMUX_BUILD_CHANNEL_DEFAULT");
}
