// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! Sysinfo data collection loop: collects CPU, memory, and network metrics
//! at 1-second intervals and publishes them via the WPS broker.
//! Port of Go's pkg/wshrpc/wshremote/sysinfo.go.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use sysinfo::Networks;

use crate::backend::rpc_types::TimeSeriesData;
use crate::backend::wps::{Broker, WaveEvent, EVENT_SYS_INFO};

const BYTES_PER_GB: f64 = 1_073_741_824.0;
const BYTES_PER_MB: f64 = 1_048_576.0;
const PERSIST_COUNT: usize = 1024;

/// Collect CPU usage (total + per-core).
fn get_cpu_data(sys: &sysinfo::System, values: &mut HashMap<String, f64>) {
    let cpus = sys.cpus();
    if cpus.is_empty() {
        return;
    }
    // Total CPU usage (average across all cores)
    let total: f64 = cpus.iter().map(|c| c.cpu_usage() as f64).sum::<f64>() / cpus.len() as f64;
    values.insert("cpu".to_string(), total);
    // Per-core usage
    for (idx, cpu) in cpus.iter().enumerate() {
        values.insert(format!("cpu:{}", idx), cpu.cpu_usage() as f64);
    }
}

/// Collect memory metrics (in GB).
fn get_mem_data(sys: &sysinfo::System, values: &mut HashMap<String, f64>) {
    let total = sys.total_memory() as f64 / BYTES_PER_GB;
    let used = sys.used_memory() as f64 / BYTES_PER_GB;
    let available = sys.available_memory() as f64 / BYTES_PER_GB;
    let free = sys.free_memory() as f64 / BYTES_PER_GB;
    values.insert("mem:total".to_string(), total);
    values.insert("mem:used".to_string(), used);
    values.insert("mem:available".to_string(), available);
    values.insert("mem:free".to_string(), free);
}

/// Network I/O tracking state for rate calculations.
struct NetState {
    prev_sent: u64,
    prev_recv: u64,
    prev_time: Option<Instant>,
}

impl NetState {
    fn new() -> Self {
        Self {
            prev_sent: 0,
            prev_recv: 0,
            prev_time: None,
        }
    }

    /// Collect network I/O rates (in MB/s).
    fn get_net_data(&mut self, networks: &Networks, values: &mut HashMap<String, f64>) {
        // Sum across all interfaces
        let mut total_sent: u64 = 0;
        let mut total_recv: u64 = 0;
        for (_name, data) in networks.iter() {
            total_sent += data.total_transmitted();
            total_recv += data.total_received();
        }

        let now = Instant::now();
        if let Some(prev_time) = self.prev_time {
            let elapsed = now.duration_since(prev_time).as_secs_f64();
            if elapsed > 0.0 {
                let sent_rate = (total_sent.saturating_sub(self.prev_sent)) as f64 / elapsed / BYTES_PER_MB;
                let recv_rate = (total_recv.saturating_sub(self.prev_recv)) as f64 / elapsed / BYTES_PER_MB;
                values.insert("net:bytessent".to_string(), sent_rate);
                values.insert("net:bytesrecv".to_string(), recv_rate);
                values.insert("net:bytestotal".to_string(), sent_rate + recv_rate);
            }
        }

        self.prev_sent = total_sent;
        self.prev_recv = total_recv;
        self.prev_time = Some(now);
    }
}

/// Run the sysinfo collection loop. Collects CPU/memory/network metrics every second
/// and publishes them to the WPS broker with the given connection name as scope.
pub async fn run_sysinfo_loop(broker: Arc<Broker>, conn_name: String) {
    let mut sys = sysinfo::System::new_all();
    let mut networks = Networks::new_with_refreshed_list();
    let mut net_state = NetState::new();
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));

    tracing::info!("sysinfo loop started for conn:{}", conn_name);

    loop {
        interval.tick().await;

        // Refresh CPU and memory data
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        networks.refresh(true);

        let mut values = HashMap::new();
        get_cpu_data(&sys, &mut values);
        get_mem_data(&sys, &mut values);
        net_state.get_net_data(&networks, &mut values);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        let ts_data = TimeSeriesData { ts: now, values };

        let event = WaveEvent {
            event: EVENT_SYS_INFO.to_string(),
            scopes: vec![conn_name.clone()],
            sender: String::new(),
            persist: PERSIST_COUNT,
            data: serde_json::to_value(&ts_data).ok(),
        };

        broker.publish(event);
    }
}
