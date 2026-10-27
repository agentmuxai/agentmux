// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The framer: what it frames and what it leaves, the version stamp, and
//! waiting while a build that can't read frames runs.

use super::*;
use crate::backend::storage::filestore::{FileMeta, FileOpts};

const P: i64 = 65536;
const MS_PER_DAY: i64 = 24 * 60 * 60 * 1000;

fn user_version(fs: &FileStore) -> i64 {
    fs.lock_conn().query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap()
}

/// A transcript of `bytes`, of delta lines and replies, written in one append.
fn transcript(fs: &FileStore, zone: &str, bytes: i64) -> Vec<u8> {
    let mut data = Vec::new();
    let mut n = 0u64;
    while (data.len() as i64) < bytes {
        data.extend_from_slice(
            format!("{{\"type\":\"stream_event\",\"event\":{{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"word {n} \"}}}},\"session_id\":\"s\",\"parent_tool_use_id\":null,\"uuid\":\"{:08x}-0000-4000-8000-{:012x}\"}}\n", n, n).as_bytes(),
        );
        if n % 40 == 39 {
            data.extend_from_slice(format!("{{\"type\":\"assistant\",\"n\":{n}}}\n").as_bytes());
        }
        n += 1;
    }
    fs.make_file(zone, "output", FileMeta::new(), FileOpts::default()).unwrap();
    fs.append_lines(zone, "output", &data).unwrap();
    data
}

fn open() -> (tempfile::TempDir, FileStore) {
    let dir = tempfile::tempdir().unwrap();
    let fs = FileStore::open_global_transcripts(&dir.path().join("filestore.db")).unwrap();
    (dir, fs)
}

fn nobody_blocks() -> Option<String> {
    None
}

#[test]
fn a_pass_frames_what_is_old_enough_and_far_enough_from_the_end() {
    let (_dir, fs) = open();
    let data = transcript(&fs, "agent:a:current", 40 * P);
    // A transcript shorter than the margin is left alone.
    let short = transcript(&fs, "agent:b:current", 4 * P);
    let mut skip = HashSet::new();
    let report = pass_with(&fs, &nobody_blocks, PASS_FRAMES, &mut skip, agentmux_common::time::now_ms()).unwrap();
    // 40 parts, 8 MiB (128 parts) margin: nothing in a 2.5 MiB file is far enough from its end.
    assert_eq!(report.frames, 0);
    assert!(report.done);
    assert_eq!(user_version(&fs), 3, "stamped before looking for frames");

    let more = transcript(&fs, "agent:c:current", 200 * P);
    let report = pass_with(&fs, &nobody_blocks, PASS_FRAMES, &mut skip, agentmux_common::time::now_ms()).unwrap();
    // 200 parts, 128 within the margin: 72 parts before it, 4 whole frames.
    assert_eq!(report.frames, 4);
    assert_eq!(report.codec2, 4, "delta lines: codec 2");
    assert!(report.stored_bytes * 5 < report.raw_bytes, "{} of {}", report.stored_bytes, report.raw_bytes);
    assert!(report.done);
    for (zone, d) in [("agent:a:current", &data), ("agent:b:current", &short), ("agent:c:current", &more)] {
        assert_eq!(&fs.read_file(zone, "output").unwrap().unwrap(), d, "{zone}");
    }
    // A second pass finds nothing new.
    let again = pass_with(&fs, &nobody_blocks, PASS_FRAMES, &mut skip, agentmux_common::time::now_ms()).unwrap();
    assert_eq!(again.frames, 0);
}

#[test]
fn a_pass_stops_at_its_budget_and_the_next_carries_on() {
    let (_dir, fs) = open();
    let data = transcript(&fs, "agent:a:current", 240 * P);
    let mut skip = HashSet::new();
    let first = pass_with(&fs, &nobody_blocks, 2, &mut skip, agentmux_common::time::now_ms()).unwrap();
    assert_eq!((first.frames, first.done), (2, false));
    let second = pass_with(&fs, &nobody_blocks, 100, &mut skip, agentmux_common::time::now_ms()).unwrap();
    assert_eq!((second.frames, second.done), (5, true), "112 parts before the margin: 7 frames");
    assert_eq!(fs.read_file("agent:a:current", "output").unwrap().unwrap(), data);
}

#[test]
fn nothing_received_in_the_last_day_is_framed() {
    let (_dir, fs) = open();
    let zone = "agent:a:current";
    let data = transcript(&fs, zone, 200 * P);
    // The sidecar says every line arrived an hour ago.
    let now = agentmux_common::time::now_ms();
    let sidecar = format!("{{\"off\":0,\"ms\":{}}}\n", now - 3_600_000);
    fs.make_file(zone, "output.tsidx", FileMeta::new(), FileOpts::default()).unwrap();
    fs.append_lines(zone, "output.tsidx", sidecar.as_bytes()).unwrap();
    let mut skip = HashSet::new();
    assert_eq!(pass_with(&fs, &nobody_blocks, PASS_FRAMES, &mut skip, now).unwrap().frames, 0);
    // Two days later it is old enough.
    let later = now + 2 * MS_PER_DAY;
    assert_eq!(pass_with(&fs, &nobody_blocks, PASS_FRAMES, &mut skip, later).unwrap().frames, 4);
    assert_eq!(fs.read_file(zone, "output").unwrap().unwrap(), data);
}

#[test]
fn a_build_that_cant_read_frames_holds_everything_off() {
    let (_dir, fs) = open();
    transcript(&fs, "agent:a:current", 200 * P);
    let mut skip = HashSet::new();
    let old = || Some("agentmux-srv-0.59.36 (pid 1) can't read compressed history".to_string());
    let report = pass_with(&fs, &old, PASS_FRAMES, &mut skip, agentmux_common::time::now_ms()).unwrap();
    assert_eq!(report.frames, 0);
    assert!(report.waiting.as_deref().is_some_and(|w| w.contains("0.59.36")));
    assert_eq!(user_version(&fs), 2, "not stamped while an older build runs");
}

#[test]
fn a_part_that_cant_be_framed_is_skipped_once() {
    let (_dir, fs) = open();
    let zone = "agent:a:current";
    let data = transcript(&fs, zone, 200 * P);
    fs.lock_conn().execute("DELETE FROM db_file_data WHERE zoneid = ?1 AND partidx = 3", rusqlite::params![zone]).unwrap();
    let mut skip = HashSet::new();
    let report = pass_with(&fs, &nobody_blocks, PASS_FRAMES, &mut skip, agentmux_common::time::now_ms()).unwrap();
    assert_eq!((report.frames, report.skipped), (3, 1), "the frame holding the lost part is skipped");
    assert!(skip.contains(&(zone.to_string(), 0)));
    let again = pass_with(&fs, &nobody_blocks, PASS_FRAMES, &mut skip, agentmux_common::time::now_ms()).unwrap();
    assert_eq!((again.frames, again.skipped), (0, 0), "and not tried again");
    // `read_file` leaves the lost row out; every other part reads as written,
    // framed or not.
    let got = fs.read_file(zone, "output").unwrap().unwrap();
    let (p3, p4) = (3 * P as usize, 4 * P as usize);
    assert_eq!(got.len(), data.len() - P as usize);
    assert_eq!(&got[..p3], &data[..p3]);
    assert_eq!(&got[p3..], &data[p4..]);
}

#[test]
fn a_readers_file_counts_only_for_its_own_process() {
    let file = serde_json::json!({"pid": 7, "started_s": 1000, "version": "x", "codecs": [1, 2]}).to_string();
    assert!(reads_frames(file.as_bytes(), 1000));
    assert!(reads_frames(file.as_bytes(), 1001), "start times are whole seconds");
    assert!(!reads_frames(file.as_bytes(), 2000), "a reused pid is another process");
    let codec1_only = serde_json::json!({"pid": 7, "started_s": 1000, "codecs": [1]}).to_string();
    assert!(!reads_frames(codec1_only.as_bytes(), 1000), "a build that reads codec 1 only can't read every frame");
    assert!(!reads_frames(b"not json", 1000));
}

#[test]
fn two_srvs_framing_at_once_never_conflict() {
    let (dir, fs) = open();
    let data = transcript(&fs, "agent:a:current", 400 * P);
    let path = dir.path().join("filestore.db");
    // Two connections to the same store, as two srvs on one machine have.
    let other = FileStore::open_global_transcripts(&path).unwrap();
    let now = agentmux_common::time::now_ms();
    let reports = std::thread::scope(|s| {
        let a = s.spawn(|| pass_with(&fs, &nobody_blocks, 100, &mut HashSet::new(), now).unwrap_or_else(|e| panic!("framer a: {e:?}")));
        let b = s.spawn(|| pass_with(&other, &nobody_blocks, 100, &mut HashSet::new(), now).unwrap_or_else(|e| panic!("framer b: {e:?}")));
        [a.join().unwrap(), b.join().unwrap()]
    });
    // 400 parts, 128 within the margin: 272 parts before it, 17 frames, each written once.
    assert_eq!(reports[0].frames + reports[1].frames, 17, "{reports:?}");
    assert_eq!(fs.frame_starts("agent:a:current", "output").unwrap().len(), 17);
    assert_eq!(fs.read_file("agent:a:current", "output").unwrap().unwrap(), data);
    assert_eq!(other.read_file("agent:a:current", "output").unwrap().unwrap(), data);
}

/// Passes until one finishes, as the loop in `bootstrap/shutdown.rs` runs them
/// (a pass cut short by a busy store is run again a few seconds later).
fn drain(store: &FileStore, now: i64) -> usize {
    let mut skip = HashSet::new();
    let mut frames = 0;
    for _ in 0..2000 {
        let report = pass_with(store, &nobody_blocks, 100, &mut skip, now).unwrap_or_else(|e| panic!("a pass failed: {e:?}"));
        frames += report.frames;
        if report.done {
            return frames;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("the framer never finished");
}

/// What two srvs on a loaded machine do: the other srv holds the store's write lock
/// for longer than this one's busy timeout (5 s in production; shortened here so the
/// test doesn't wait). Framing must end its pass quietly and finish on a later one,
/// not fail (the failure AgentA saw in `two_srvs_framing_at_once_never_conflict`).
#[test]
fn a_store_busy_for_longer_than_the_timeout_ends_the_pass_quietly() {
    let (dir, fs) = open();
    let data = transcript(&fs, "agent:a:current", 400 * P);
    let path = dir.path().join("filestore.db");
    let other = FileStore::open_global_transcripts(&path).unwrap();
    fs.lock_conn().busy_timeout(std::time::Duration::from_millis(20)).unwrap();
    let now = agentmux_common::time::now_ms();
    // The other srv holds the write lock for as long as the test says: it signals
    // once it has the lock and keeps it until released, so no timing is assumed.
    let (locked_tx, locked_rx) = std::sync::mpsc::channel::<()>();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let holder = std::thread::spawn({
        let path = path.clone();
        move || {
            let mut conn = rusqlite::Connection::open(&path).unwrap();
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
            locked_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            tx.commit().unwrap();
        }
    });
    locked_rx.recv().unwrap();
    let report = pass_with(&fs, &nobody_blocks, 100, &mut HashSet::new(), now);
    release_tx.send(()).unwrap();
    holder.join().unwrap();
    let report = report.unwrap();
    assert!(report.busy, "{report:?}");
    assert!(!report.done);
    assert_eq!(report.frames, 0);
    // Later passes finish the work, once, whoever runs them.
    assert_eq!(drain(&fs, now) + drain(&other, now), 17);
    assert_eq!(fs.frame_starts("agent:a:current", "output").unwrap().len(), 17);
    assert_eq!(fs.read_file("agent:a:current", "output").unwrap().unwrap(), data);
}

/// Two srvs framing at once, each waiting for the write lock only briefly (the
/// timeout is 5 s in production, and a frame's transaction takes long enough on a
/// loaded machine, in a debug build, for one srv to keep the lock that long).
#[test]
fn two_srvs_with_short_busy_timeouts_frame_each_part_once() {
    let (dir, fs) = open();
    let data = transcript(&fs, "agent:a:current", 400 * P);
    let path = dir.path().join("filestore.db");
    let other = FileStore::open_global_transcripts(&path).unwrap();
    for store in [&fs, &other] {
        store.lock_conn().busy_timeout(std::time::Duration::from_millis(1)).unwrap();
    }
    let now = agentmux_common::time::now_ms();
    let frames = std::thread::scope(|s| {
        let a = s.spawn(|| drain(&fs, now));
        let b = s.spawn(|| drain(&other, now));
        a.join().unwrap() + b.join().unwrap()
    });
    assert_eq!(frames, 17);
    assert_eq!(fs.frame_starts("agent:a:current", "output").unwrap().len(), 17);
    assert_eq!(other.read_file("agent:a:current", "output").unwrap().unwrap(), data);
}

#[test]
fn an_unset_setting_runs_the_framer_and_only_false_turns_it_off() {
    assert!(on_unless_false(None), "on by default (operator, 2026-10-25)");
    assert!(on_unless_false(Some(true)));
    assert!(!on_unless_false(Some(false)), "the opt-out");
}

// ---------------------------------------------------------------------------
// Multi-process stress (ignored; run by hand): several srv-like processes on one
// scratch shared store, appending, framing, reading and truncating the WAL, with
// random SIGKILLs and restarts, and an integrity check while it runs.
//   STRESS_DIR=/tmp/x STRESS_SECS=60 cargo test -p agentmux-srv --lib stress_parent -- --ignored --nocapture
// ---------------------------------------------------------------------------

fn stress_env(name: &str, default: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn stress_line(n: u64, tag: &str) -> Vec<u8> {
    let mut v = format!("{{\"type\":\"stream_event\",\"event\":{{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"{tag} word {n} \"}}}},\"session_id\":\"s\",\"parent_tool_use_id\":null,\"uuid\":\"{:08x}-0000-4000-8000-{:012x}\"}}\n", n as u32, n).into_bytes();
    if n % 40 == 39 {
        v.extend_from_slice(format!("{{\"type\":\"assistant\",\"n\":{n}}}\n").as_bytes());
    }
    v
}

#[test]
#[ignore]
fn stress_child() {
    let dir = std::path::PathBuf::from(std::env::var("STRESS_DIR").expect("STRESS_DIR"));
    let role = std::env::var("STRESS_ROLE").expect("STRESS_ROLE");
    let deadline = stress_env("STRESS_DEADLINE_MS", 0) as i64;
    let zones = stress_env("STRESS_ZONES", 6);
    let fs = FileStore::open_global_transcripts(&dir.join("filestore.db")).expect("open");
    mark_reader(&dir);
    let mut skip = HashSet::new();
    let mut n: u64 = std::process::id() as u64 * 1_000_000;
    let mut rng = n ^ 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    let fail = |what: &str, e: &dyn std::fmt::Display| {
        println!("STRESS-ERROR role={role} pid={} {what}: {e}", std::process::id());
        std::process::exit(3);
    };
    while agentmux_common::time::now_ms() < deadline {
        match role.as_str() {
            "appender" => {
                let zone = format!("agent:z{}:current", next() % zones);
                let mut data = Vec::new();
                let target = 256 * 1024 + (next() % (768 * 1024)) as usize;
                while data.len() < target {
                    n += 1;
                    data.extend_from_slice(&stress_line(n, &zone));
                }
                if let Err(e) = fs.append_lines(&zone, "output", &data) {
                    fail("append", &e);
                }
                std::thread::sleep(std::time::Duration::from_millis(40 + next() % 120));
            }
            "framer" => {
                match pass_with(&fs, &nobody_blocks, 8, &mut skip, agentmux_common::time::now_ms() + 2 * MS_PER_DAY) {
                    Ok(r) => println!("STRESS-PASS pid={} frames={} busy={} done={}", std::process::id(), r.frames, r.busy, r.done),
                    Err(e) => fail("pass", &e),
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            "reader" => {
                let zone = format!("agent:z{}:current", next() % zones);
                match fs.read_file(&zone, "output") {
                    Ok(_) => {}
                    Err(e) => fail("read", &e),
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            "truncater" => {
                if let Err(e) = fs.checkpoint_truncate_apart() {
                    fail("truncate", &e);
                }
                std::thread::sleep(std::time::Duration::from_millis(1500));
            }
            other => panic!("unknown role {other}"),
        }
    }
}

#[test]
#[ignore]
fn stress_parent() {
    use std::process::{Child, Command, Stdio};
    let dir = std::path::PathBuf::from(std::env::var("STRESS_DIR").expect("STRESS_DIR"));
    std::fs::create_dir_all(&dir).unwrap();
    let secs = stress_env("STRESS_SECS", 60);
    let zones = stress_env("STRESS_ZONES", 6);
    let seed_mib = stress_env("STRESS_SEED_MIB", 30) as i64;
    let kill = stress_env("STRESS_KILL", 1) == 1;
    // roles: appender,appender,framer,framer,reader,truncater by default
    let roles: Vec<String> = std::env::var("STRESS_ROLES")
        .unwrap_or_else(|_| "appender,appender,framer,framer,reader,truncater".into())
        .split(',')
        .map(str::to_string)
        .collect();
    if std::env::var("STRESS_AUTOVAC").as_deref() == Ok("none") {
        // Like the live store: a file that already holds a page is left in the default (no auto-vacuum) mode.
        let conn = rusqlite::Connection::open(dir.join("filestore.db")).unwrap();
        conn.execute_batch("PRAGMA auto_vacuum=NONE; CREATE TABLE _seed(x); DROP TABLE _seed;").unwrap();
    }
    {
        let fs = FileStore::open_global_transcripts(&dir.join("filestore.db")).unwrap();
        for z in 0..zones {
            transcript(&fs, &format!("agent:z{z}:current"), seed_mib << 20);
        }
    }
    let started = agentmux_common::time::now_ms();
    let deadline = started + (secs as i64) * 1000;
    let exe = std::env::current_exe().unwrap();
    let spawn = |role: &str| -> Child {
        Command::new(&exe)
            .args(["--exact", "backend::transcript::framer::tests::stress_child", "--ignored", "--nocapture", "--test-threads=1"])
            .env("STRESS_DIR", &dir)
            .env("STRESS_ROLE", role)
            .env("STRESS_DEADLINE_MS", deadline.to_string())
            .env("STRESS_ZONES", zones.to_string())
            .stdout(Stdio::inherit())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    };
    let mut kids: Vec<(String, Child)> = roles.iter().map(|r| (r.clone(), spawn(r))).collect();
    let mut rng = 0x2545_F491_4F6C_DD1Du64 ^ deadline as u64;
    let (mut kills, mut bad_exits, mut checks) = (0, 0, 0);
    let mut next_check = agentmux_common::time::now_ms();
    let mut next_kill = agentmux_common::time::now_ms() + 3000;
    let mut verdict: Option<String> = None;
    while agentmux_common::time::now_ms() < deadline && verdict.is_none() {
        std::thread::sleep(std::time::Duration::from_millis(250));
        let now = agentmux_common::time::now_ms();
        for (role, kid) in kids.iter_mut() {
            if let Ok(Some(status)) = kid.try_wait() {
                if !status.success() && status.code().is_some() {
                    bad_exits += 1;
                    println!("STRESS-CHILD-EXIT role={role} status={status}");
                }
                if now < deadline {
                    *kid = spawn(role);
                }
            }
        }
        if kill && now >= next_kill {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            let i = (rng % kids.len() as u64) as usize;
            let role = kids[i].0.clone();
            let _ = kids[i].1.kill();
            let _ = kids[i].1.wait();
            kills += 1;
            println!("STRESS-KILL role={role}");
            kids[i].1 = spawn(&role);
            next_kill = now + 2000 + ((rng >> 8) % 5000) as i64;
        }
        if now >= next_check {
            next_check = now + 5000;
            checks += 1;
            let conn = rusqlite::Connection::open(dir.join("filestore.db")).unwrap();
            conn.execute_batch("PRAGMA busy_timeout=20000;").unwrap();
            let rows: Vec<String> = conn
                .prepare("PRAGMA integrity_check(20)")
                .unwrap()
                .query_map([], |r| r.get::<_, String>(0))
                .unwrap()
                .filter_map(Result::ok)
                .collect();
            if rows != ["ok"] {
                verdict = Some(format!("integrity_check at check #{checks} (t={}s, kills so far {kills}): {:?}", (now - started) / 1000, rows.iter().map(|r| r.lines().nth(1).unwrap_or("").to_string()).collect::<Vec<_>>()));
            }
        }
    }
    for (_, kid) in kids.iter_mut() {
        let _ = kid.wait();
    }
    let conn = rusqlite::Connection::open(dir.join("filestore.db")).unwrap();
    let rows: Vec<String> = conn
        .prepare("PRAGMA integrity_check(20)")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    let frames: i64 = conn.query_row("select count(*) from db_file_frame", [], |r| r.get(0)).unwrap();
    println!("STRESS-SUMMARY secs={secs} kills={kills} bad_exits={bad_exits} checks={checks} frames={frames} final_integrity_first_lines={:?} mid_run={verdict:?}", rows.iter().map(|r| r.lines().take(3).collect::<Vec<_>>().join(" | ")).collect::<Vec<_>>());
    assert!(verdict.is_none() && rows == ["ok"], "store damaged: {verdict:?} {rows:?}");
}
