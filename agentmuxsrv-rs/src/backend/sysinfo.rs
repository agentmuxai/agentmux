// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! Sysinfo data collection loop: collects CPU, memory, and network metrics
//! and publishes them via the WPS broker. Sampling interval is configurable
//! via the `telemetry:interval` setting (0.1s–2.0s, default 1.0s).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use sysinfo::{Disks, Networks, Pid, ProcessRefreshKind, ProcessesToUpdate};
use tokio::time::MissedTickBehavior;

use crate::backend::blockcontroller::pidregistry;
use crate::backend::blockcontroller::process_tree;
use crate::backend::rpc_types::TimeSeriesData;
use crate::backend::wconfig::ConfigWatcher;
use crate::backend::wps::{Broker, WaveEvent, EVENT_BLOCK_STATS, EVENT_SYS_INFO};

const BYTES_PER_GB: f64 = 1_073_741_824.0;
const BYTES_PER_MB: f64 = 1_048_576.0;
const PERSIST_COUNT: usize = 1024;
const DEFAULT_INTERVAL_SECS: f64 = 1.0;
const MIN_INTERVAL_SECS: f64 = 0.2;
const MAX_INTERVAL_SECS: f64 = 2.0;

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

/// Collect disk I/O rates (in MB/s).
/// sysinfo Disk::usage() returns deltas (bytes since last refresh) so we
/// divide by elapsed time to get rates.
fn get_disk_data(disks: &Disks, elapsed_secs: f64, values: &mut HashMap<String, f64>) {
    if elapsed_secs <= 0.0 {
        return;
    }
    let (total_read, total_write) = disks.list().iter().fold((0u64, 0u64), |(r, w), disk| {
        let u = disk.usage();
        (r + u.read_bytes, w + u.written_bytes)
    });
    let read_rate = total_read as f64 / elapsed_secs / BYTES_PER_MB;
    let write_rate = total_write as f64 / elapsed_secs / BYTES_PER_MB;
    values.insert("disk:read".to_string(), read_rate);
    values.insert("disk:write".to_string(), write_rate);
    values.insert("disk:total".to_string(), read_rate + write_rate);
}

/// Read the telemetry interval from config, clamped to [MIN, MAX].
fn get_interval_secs(config_watcher: &ConfigWatcher) -> f64 {
    let val = config_watcher.get_settings().telemetry_interval;
    if val <= 0.0 {
        return DEFAULT_INTERVAL_SECS;
    }
    val.clamp(MIN_INTERVAL_SECS, MAX_INTERVAL_SECS)
}

/// Run the sysinfo collection loop. Uses `tokio::time::interval` for steady
/// tick rate regardless of refresh duration. Interval is re-read from config
/// each tick and the timer is reset if it changes.
pub async fn run_sysinfo_loop(broker: Arc<Broker>, config_watcher: Arc<ConfigWatcher>, conn_name: String) {
    let mut sys = sysinfo::System::new_all();
    let mut networks = Networks::new_with_refreshed_list();
    let mut net_state = NetState::new();
    let mut disks = Disks::new_with_refreshed_list();
    let mut last_tick = Instant::now();

    let mut current_interval = get_interval_secs(&config_watcher);
    let mut ticker = tokio::time::interval(Duration::from_secs_f64(current_interval));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    // Skip the first immediate tick
    ticker.tick().await;

    tracing::info!("sysinfo loop started for conn:{}", conn_name);

    loop {
        ticker.tick().await;

        // Check if interval changed and reset ticker if so
        let new_interval = get_interval_secs(&config_watcher);
        if (new_interval - current_interval).abs() > 0.001 {
            current_interval = new_interval;
            ticker = tokio::time::interval(Duration::from_secs_f64(current_interval));
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            ticker.tick().await; // consume immediate first tick
            tracing::info!("sysinfo interval changed to {}s", current_interval);
        }

        // Refresh all metrics
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        networks.refresh(true);
        disks.refresh(true);

        let now_instant = Instant::now();
        let elapsed_secs = now_instant.duration_since(last_tick).as_secs_f64();
        last_tick = now_instant;

        let mut values = HashMap::new();
        get_cpu_data(&sys, &mut values);
        get_mem_data(&sys, &mut values);
        net_state.get_net_data(&networks, &mut values);
        get_disk_data(&disks, elapsed_secs, &mut values);

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

        // Per-pane process tree metrics: aggregate CPU/mem across each block's
        // shell process and all its descendants.
        let block_pids = pidregistry::get_all();
        if !block_pids.is_empty() {
            // Pass 1: cheap minimal refresh of all processes to populate parent()
            // links. ProcessRefreshKind::new() skips CPU accounting and memory
            // queries — just PID/PPID/name. ~0.5ms on a typical desktop.
            sys.refresh_processes_specifics(
                ProcessesToUpdate::All,
                false, // keep stale entries — pass 2 removes dead ones
                ProcessRefreshKind::nothing(),
            );

            // For each block, BFS the process tree from the shell PID.
            let mut block_trees: Vec<(String, Vec<Pid>)> = block_pids
                .iter()
                .map(|(block_id, pid)| {
                    let root = Pid::from(*pid as usize);
                    let tree = process_tree::collect_descendants(
                        &sys,
                        root,
                        process_tree::MAX_PIDS_PER_BLOCK,
                    );
                    (block_id.clone(), tree)
                })
                .collect();

            // Pass 2: targeted deep refresh (CPU + mem) for only the PIDs we care about.
            // Deduplicate across blocks so each PID is refreshed at most once.
            let mut all_pids: Vec<Pid> = block_trees
                .iter()
                .flat_map(|(_, pids)| pids.iter().copied())
                .collect();
            all_pids.sort_unstable();
            all_pids.dedup();
            sys.refresh_processes_specifics(
                ProcessesToUpdate::Some(&all_pids),
                true, // remove dead processes on this authoritative pass
                ProcessRefreshKind::everything(),
            );

            // Aggregate per block and publish.
            // After Pass 2 (remove_dead=true), sys.process() returns None for any
            // PID that no longer exists — use this to detect orphaned registry entries.
            let mut dead_block_ids: Vec<String> = Vec::new();

            for (block_id, pids) in &mut block_trees {
                // collect_descendants() always puts the root PID first.
                let root_pid = pids.first().copied().unwrap_or(Pid::from(0usize));
                let mut total_cpu: f64 = 0.0;
                let mut total_mem: u64 = 0;
                let mut live_count: u32 = 0;

                for pid in pids.iter() {
                    if let Some(proc) = sys.process(*pid) {
                        total_cpu += proc.cpu_usage() as f64;
                        total_mem += proc.memory();
                        live_count += 1;
                    }
                }

                // Root process is gone — evict from registry.  This is the last-resort
                // cleanup for processes that exit without normal wait-task teardown
                // (SIGKILL by the OS, unexpected crash, or stop() race).
                if sys.process(root_pid).is_none() {
                    dead_block_ids.push(block_id.clone());
                    continue; // skip publishing stale stats for a dead block
                }

                let mut block_values = HashMap::new();
                block_values.insert("cpu".to_string(), total_cpu);
                block_values.insert("mem".to_string(), total_mem as f64);
                block_values.insert("pids".to_string(), live_count as f64);

                let block_ts = TimeSeriesData {
                    ts: now,
                    values: block_values,
                };
                let block_event = WaveEvent {
                    event: EVENT_BLOCK_STATS.to_string(),
                    scopes: vec![format!("block:{}", block_id)],
                    sender: String::new(),
                    persist: 0,
                    data: serde_json::to_value(&block_ts).ok(),
                };
                broker.publish(block_event);
            }

            for block_id in &dead_block_ids {
                pidregistry::unregister(block_id);
                tracing::warn!(
                    block_id = %block_id,
                    "sysinfo: evicted dead root PID — process exited without normal cleanup"
                );
            }
        }
    }
}
