// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! An ordinary srv boot stays inside its time budget.
//!
//! SPEC_FAST_STARTUP_UPGRADE_OWNS_MIGRATIONS_AND_UPDATES_2026_09_15 sets the
//! budget: under 300 ms from srv's start to the end of its boot path on a warm
//! boot with nothing pending (REPORT_STARTUP_ARCHITECTURE_AND_MIGRATIONS_AUDIT
//! §6.3). srv prints one `boot_timing: total=<n>ms ...` line where that path
//! ends (`boot_timing::log_summary`); this test boots srv on a fresh data
//! folder, boots it again on the same folder, and reads that line.
//!
//! The first boot builds the databases and is not measured. Of the warm boots
//! the fastest counts, so a busy CI runner has to be slow three times running
//! to fail it. An unoptimised test build is several times slower than the
//! shipped one, so it gets a wider limit: it still catches a step that starts
//! to sleep, wait on the network or walk every agent, which is what the budget
//! is for.

// This binary must hold this checkout's code (`agentmux_common::checkout`).
agentmux_common::checkout_guard!();

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// The spec's budget, for the shipped (optimised) build.
const BUDGET_MS: u64 = 300;
/// What an unoptimised test build is held to instead.
const DEBUG_BUDGET_MS: u64 = 1000;
const WARM_BOOTS: usize = 3;
const BOOT_WAIT: Duration = Duration::from_secs(60);

/// `total=153ms` in a `boot_timing:` line, in milliseconds.
fn total_ms(line: &str) -> Option<u64> {
    let rest = &line[line.find("boot_timing:")?..];
    let digits = rest[rest.find("total=")? + "total=".len()..].split("ms").next()?;
    digits.trim().parse().ok()
}

fn spawn_srv(data_dir: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_agentmux-srv"))
        .env("AGENTMUX_AUTH_KEY", "boot-budget-test-key")
        .env("AGENTMUX_DATA_DIR", data_dir)
        .env("AGENTMUX_SHARED_DIR", data_dir.join("shared"))
        .env("AGENTMUX_LOG_DIR", data_dir.join("logs"))
        .env("AGENTMUX_DISABLE_CLOUD_SUBSCRIBER", "1")
        // A home the runner inherited (a `task dev` shell has one) outranks
        // the data folder above and would put the databases and the lock there.
        .env("AGENTMUX_HOME_OVERRIDE", data_dir)
        .env_remove("AGENTMUX_DATA_HOME")
        // The `boot_timing` line is INFO; a `RUST_LOG` the test runner inherited
        // (`warn`, say) would filter it out and the test would wait for nothing.
        .env("RUST_LOG", "info")
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("failed to spawn agentmux-srv")
}

/// Whether a file called `name` exists anywhere under `dir`.
fn has_file(dir: &Path, name: &str) -> bool {
    std::fs::read_dir(dir).into_iter().flatten().flatten().any(|e| {
        let path = e.path();
        if path.is_dir() { has_file(&path, name) } else { e.file_name() == name }
    })
}

/// Boot srv on `data_dir`, return the `total` it reports, and stop it.
fn boot_once(data_dir: &Path) -> u64 {
    let mut child = spawn_srv(data_dir);
    let stderr = child.stderr.take().expect("srv stderr");
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if let Some(ms) = total_ms(&line) {
                let _ = tx.send(ms);
                return;
            }
        }
    });
    let total = rx.recv_timeout(BOOT_WAIT);

    // Close stdin, as the launcher does when it goes, and wait for srv to let
    // go of the data folder's lock; kill it if it does not.
    drop(child.stdin.take());
    let deadline = Instant::now() + Duration::from_secs(20);
    while child.try_wait().expect("try_wait").is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    total.unwrap_or_else(|_| panic!("srv printed no boot_timing line within {BOOT_WAIT:?}"))
}

#[test]
fn a_warm_boot_with_nothing_pending_is_inside_the_budget() {
    let data_dir = tempfile::tempdir().expect("temp data dir");
    let cold = boot_once(data_dir.path());
    assert!(
        has_file(data_dir.path(), "objects.db"),
        "srv did not keep its databases in the test's data folder; the measurement would not be of a fresh boot"
    );
    let warm: Vec<u64> = (0..WARM_BOOTS).map(|_| boot_once(data_dir.path())).collect();
    let best = *warm.iter().min().expect("at least one warm boot");
    let budget = if cfg!(debug_assertions) { DEBUG_BUDGET_MS } else { BUDGET_MS };
    eprintln!("boot_budget: cold={cold}ms warm={warm:?} budget={budget}ms");
    assert!(
        best <= budget,
        "a warm boot took {best} ms at best ({warm:?}), over the {budget} ms budget; \
         `muxlog srv grep boot_timing` shows which step grew"
    );
}

#[test]
fn the_total_is_read_from_a_boot_timing_line() {
    let line = r#"{"fields":{"message":"boot_timing: total=153ms accounted=196ms | open_objects_db=5ms"}}"#;
    assert_eq!(total_ms(line), Some(153));
    assert_eq!(total_ms("something else total=9ms"), None);
}
