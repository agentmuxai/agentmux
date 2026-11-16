// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The sampler's own thread. The sampler used to be a task on srv's runtime, so
//! when the runtime stopped running tasks (the log's `runtime-stall:` lines), the
//! tick that measures the machine waited with everything else, and the CPU chart
//! showed a hole exactly when the machine was busiest. On its own OS thread, with
//! a runtime of its own that holds nothing else, the tick is only as late as the
//! operating system is.

use std::io;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::thread::JoinHandle;

use crate::backend::mps::Broker;
use crate::backend::sampler::panes::PaneSampler;
use crate::backend::sysinfo::run_sysinfo_loop;
use crate::backend::wconfig::ConfigState;

/// Starts the sampling loop on a thread of its own. It runs until `stop` is set
/// (within one interval), which only a test does.
pub fn spawn(
    broker: Arc<Broker>,
    config: Arc<ConfigState>,
    conn_name: String,
    panes: PaneSampler,
    stop: Arc<AtomicBool>,
) -> io::Result<JoinHandle<()>> {
    std::thread::Builder::new().name("sysinfo-sampler".into()).spawn(move || {
        // Timers only: the loop reads the OS directly and publishes through the broker's own lock.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("the sampler's runtime");
        rt.block_on(run_sysinfo_loop(broker, config, conn_name, panes, stop));
    })
}
