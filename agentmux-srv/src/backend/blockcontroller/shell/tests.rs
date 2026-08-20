// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Unit tests for the shell controller, moved verbatim from the pre-split
//! `shell.rs` inline `mod tests`. Public items are reached via the flat
//! re-exports in `super` (`super::*`); test-only private helpers are reached
//! through their owning submodule path.

use super::*;

// Struct fields / meta helpers / status constants used by the tests but not part
// of the crate-public flat surface.
use super::super::{
    BlockInputUnion, Controller, META_KEY_CMD_CLOSE_ON_EXIT_DELAY, META_KEY_CMD_RUN_ONCE,
    META_KEY_CMD_RUN_ON_START, META_KEY_CONNECTION, META_KEY_CMD_CLEAR_ON_START, STATUS_DONE,
    STATUS_INIT,
};
use crate::backend::obj::{self, MetaMapType};
use crate::backend::shellexec::{ConnInterface, MockConn};
use crate::backend::storage::filestore::FileStore;
use crate::backend::storage::store::Store;
use crate::backend::wps;

// Private / non-flat-re-exported helpers reached through their owning submodule.
use super::file_ops::{handle_truncate_block_file, mirror_append_to_global};
use super::translation::{extract_agent_events, AGENT_LINE_BUFFER_CAP};

use std::sync::Arc;

    fn make_shell_meta() -> MetaMapType {
        let mut meta = MetaMapType::new();
        meta.insert(
            "controller".to_string(),
            serde_json::Value::String("shell".to_string()),
        );
        meta
    }

    fn make_cmd_meta(cmd: &str) -> MetaMapType {
        let mut meta = MetaMapType::new();
        meta.insert(
            "controller".to_string(),
            serde_json::Value::String("cmd".to_string()),
        );
        meta.insert(
            "cmd".to_string(),
            serde_json::Value::String(cmd.to_string()),
        );
        meta
    }

    // ── pty_size_from_rt_opts ────────────────────────────────────────────
    // Seeds the initial PTY geometry from the resync `rtopts` payload so the
    // agent pane is born at the right width (no post-spawn resize race).
    // See docs/analysis/AGENT_PANE_PTY_RESIZE_RACE_2026_06_16.md.

    #[test]
    fn pty_size_defaults_when_rt_opts_absent() {
        let sz = ShellController::pty_size_from_rt_opts(&None);
        assert_eq!((sz.rows, sz.cols), (25, 200));
    }

    #[test]
    fn pty_size_defaults_when_termsize_is_serde_default() {
        // rows==0 && cols==0 is the serde default → treat as absent.
        let v = serde_json::json!({ "termsize": { "rows": 0, "cols": 0 } });
        let sz = ShellController::pty_size_from_rt_opts(&Some(v));
        assert_eq!((sz.rows, sz.cols), (25, 200));
    }

    #[test]
    fn pty_size_honors_supplied_termsize() {
        let v = serde_json::json!({ "termsize": { "rows": 50, "cols": 130 } });
        let sz = ShellController::pty_size_from_rt_opts(&Some(v));
        assert_eq!((sz.rows, sz.cols), (50, 130));
    }

    #[test]
    fn pty_size_keeps_default_rows_for_cols_only_payload() {
        let v = serde_json::json!({ "termsize": { "rows": 0, "cols": 130 } });
        let sz = ShellController::pty_size_from_rt_opts(&Some(v));
        assert_eq!((sz.rows, sz.cols), (25, 130));
    }

    #[test]
    fn pty_size_clamps_oversized_values() {
        let v = serde_json::json!({ "termsize": { "rows": 99999, "cols": 99999 } });
        let sz = ShellController::pty_size_from_rt_opts(&Some(v));
        assert_eq!((sz.rows, sz.cols), (1000, 1000));
    }

    #[test]
    fn pty_size_defaults_on_unparseable_rt_opts() {
        // Unknown keys deserialize to RuntimeOpts default (termsize 0/0) → fallback.
        let v = serde_json::json!({ "totally": "unrelated" });
        let sz = ShellController::pty_size_from_rt_opts(&Some(v));
        assert_eq!((sz.rows, sz.cols), (25, 200));
    }

    #[test]
    fn pty_size_ignores_non_positive_axes() {
        // Negative axes fail the `> 0` guard and keep their default.
        let v = serde_json::json!({ "termsize": { "rows": -5, "cols": -1 } });
        let sz = ShellController::pty_size_from_rt_opts(&Some(v));
        assert_eq!((sz.rows, sz.cols), (25, 200));
    }

    #[test]
    fn test_shell_controller_new() {
        let ctrl = ShellController::new(
            "shell".to_string(),
            "tab-1".to_string(),
            "block-1".to_string(),
            None,
            None,
            None,
            None,
        );
        assert_eq!(ctrl.controller_type(), "shell");
        assert_eq!(ctrl.block_id(), "block-1");

        let status = ctrl.get_runtime_status();
        assert_eq!(status.shellprocstatus, STATUS_INIT);
        assert_eq!(status.blockid, "block-1");
        assert_eq!(status.version, 0);
    }

    #[test]
    fn test_shell_controller_start_stop() {
        let ctrl = ShellController::new(
            "shell".to_string(),
            "tab-1".to_string(),
            "block-1".to_string(),
            None,
            None,
            None,
            None,
        );

        // Use mock factory so we don't open a real PTY in tests
        ctrl.set_conn_factory(Box::new(|_conn_name, _meta| {
            Ok(Box::new(MockConn::new(0)) as Box<dyn ConnInterface>)
        }));

        let meta = make_shell_meta();
        let result = ctrl.start(meta, None, false);
        assert!(result.is_ok());

        // After start with mock, process immediately exits → status is done
        let status = ctrl.get_runtime_status();
        assert_eq!(status.shellprocstatus, STATUS_DONE);

        // Stop should work
        let result = ctrl.stop(true, STATUS_DONE);
        assert!(result.is_ok());
    }

    #[test]
    fn test_shell_controller_run_on_start_false() {
        let ctrl = ShellController::new(
            "shell".to_string(),
            "tab-1".to_string(),
            "block-1".to_string(),
            None,
            None,
            None,
            None,
        );

        let mut meta = make_shell_meta();
        meta.insert(
            META_KEY_CMD_RUN_ON_START.to_string(),
            serde_json::Value::Bool(false),
        );

        let result = ctrl.start(meta, None, false);
        assert!(result.is_ok());

        // Should still be in init state (didn't start)
        let status = ctrl.get_runtime_status();
        assert_eq!(status.shellprocstatus, STATUS_INIT);
    }

    #[test]
    fn test_shell_controller_force_start() {
        let ctrl = ShellController::new(
            "shell".to_string(),
            "tab-1".to_string(),
            "block-1".to_string(),
            None,
            None,
            None,
            None,
        );

        ctrl.set_conn_factory(Box::new(|_conn_name, _meta| {
            Ok(Box::new(MockConn::new(0)) as Box<dyn ConnInterface>)
        }));

        let mut meta = make_shell_meta();
        meta.insert(
            META_KEY_CMD_RUN_ON_START.to_string(),
            serde_json::Value::Bool(false),
        );

        // Force should override run_on_start=false
        let result = ctrl.start(meta, None, true);
        assert!(result.is_ok());

        let status = ctrl.get_runtime_status();
        // With mock, immediately exits to done
        assert_eq!(status.shellprocstatus, STATUS_DONE);
    }

    #[test]
    fn test_shell_controller_with_conn_factory() {
        let ctrl = ShellController::new(
            "cmd".to_string(),
            "tab-1".to_string(),
            "block-1".to_string(),
            None,
            None,
            None,
            None,
        );

        // Set a custom factory that returns a mock with exit code 42
        ctrl.set_conn_factory(Box::new(|_conn_name, _meta| {
            Ok(Box::new(MockConn::new(42)) as Box<dyn ConnInterface>)
        }));

        let meta = make_cmd_meta("echo hello");
        let result = ctrl.start(meta, None, true);
        assert!(result.is_ok());

        let status = ctrl.get_runtime_status();
        assert_eq!(status.shellprocstatus, STATUS_DONE);
        assert_eq!(status.shellprocexitcode, 42);
    }

    #[test]
    fn test_shell_controller_conn_factory_error() {
        let ctrl = ShellController::new(
            "shell".to_string(),
            "tab-1".to_string(),
            "block-1".to_string(),
            None,
            None,
            None,
            None,
        );

        ctrl.set_conn_factory(Box::new(|_conn_name, _meta| {
            Err("connection refused".to_string())
        }));

        let meta = make_shell_meta();
        let result = ctrl.start(meta, None, true);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("connection refused"));

        let status = ctrl.get_runtime_status();
        assert_eq!(status.shellprocstatus, STATUS_DONE);
        assert_eq!(status.shellprocexitcode, -1);
    }

    #[test]
    fn test_shell_controller_send_input_not_running() {
        let ctrl = ShellController::new(
            "shell".to_string(),
            "tab-1".to_string(),
            "block-1".to_string(),
            None,
            None,
            None,
            None,
        );

        let result = ctrl.send_input(BlockInputUnion::data(b"hello".to_vec()), None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not running"));
    }

    #[test]
    fn test_shell_controller_status_version_increments() {
        let ctrl = ShellController::new(
            "shell".to_string(),
            "tab-1".to_string(),
            "block-1".to_string(),
            None,
            None,
            None,
            None,
        );

        ctrl.set_conn_factory(Box::new(|_conn_name, _meta| {
            Ok(Box::new(MockConn::new(0)) as Box<dyn ConnInterface>)
        }));

        let v0 = ctrl.get_runtime_status().version;

        let meta = make_shell_meta();
        ctrl.start(meta, None, true).unwrap();

        let v_after = ctrl.get_runtime_status().version;
        // Status changed from init → running → done = at least 2 increments
        assert!(v_after > v0);
    }

    #[test]
    fn test_shell_controller_stores_filestore_for_write_through() {
        // SPEC_TERMINAL_SCROLLBACK_PERSISTENCE_2026_07_23.md §2.1 — confirms
        // the constructor wiring itself (the actual PTY read loop's use of
        // `filestore_read.as_ref()` is inside a real-PTY code path, `set_
        // conn_factory`'s mock path is a separate, simpler branch that
        // doesn't reach it — `handle_append_block_file`'s own Some-filestore
        // behavior is already covered by `test_handle_append_block_file_
        // writes_to_filestore` above).
        let fs = Arc::new(FileStore::open_in_memory().expect("filestore"));
        let ctrl = ShellController::new(
            "shell".to_string(),
            "tab-1".to_string(),
            "block-1".to_string(),
            None,
            None,
            None,
            Some(fs.clone()),
        );
        assert!(ctrl.filestore.is_some());
        assert!(Arc::ptr_eq(ctrl.filestore.as_ref().unwrap(), &fs));
    }

    #[test]
    fn test_controller_trait_as_arc() {
        let ctrl: Arc<dyn Controller> = Arc::new(ShellController::new(
            "shell".to_string(),
            "tab-1".to_string(),
            "block-1".to_string(),
            None,
            None,
            None,
            None,
        ));

        assert_eq!(ctrl.controller_type(), "shell");
        assert_eq!(ctrl.block_id(), "block-1");
        let status = ctrl.get_runtime_status();
        assert_eq!(status.shellprocstatus, STATUS_INIT);
    }

    #[test]
    fn test_meta_helpers() {
        let mut meta = MetaMapType::new();
        assert!(ShellController::should_run_on_start(&meta)); // default true
        assert!(!ShellController::should_run_once(&meta)); // default false
        assert!(!ShellController::should_clear_on_start(&meta)); // default false
        assert!(!ShellController::should_close_on_exit(&meta)); // default false

        meta.insert(
            META_KEY_CMD_RUN_ON_START.to_string(),
            serde_json::Value::Bool(false),
        );
        assert!(!ShellController::should_run_on_start(&meta));

        meta.insert(
            META_KEY_CMD_RUN_ONCE.to_string(),
            serde_json::Value::Bool(true),
        );
        assert!(ShellController::should_run_once(&meta));

        meta.insert(
            META_KEY_CMD_CLEAR_ON_START.to_string(),
            serde_json::Value::Bool(true),
        );
        assert!(ShellController::should_clear_on_start(&meta));
    }

    #[test]
    fn test_close_on_exit_delay() {
        let mut meta = MetaMapType::new();
        assert_eq!(ShellController::close_on_exit_delay_ms(&meta), 2000); // default

        meta.insert(
            META_KEY_CMD_CLOSE_ON_EXIT_DELAY.to_string(),
            serde_json::json!(5000),
        );
        assert_eq!(ShellController::close_on_exit_delay_ms(&meta), 5000);
    }

    #[test]
    fn test_conn_name_from_meta() {
        let mut meta = MetaMapType::new();
        assert_eq!(ShellController::get_conn_name(&meta), "local"); // default

        meta.insert(
            META_KEY_CONNECTION.to_string(),
            serde_json::Value::String("user@host".to_string()),
        );
        assert_eq!(ShellController::get_conn_name(&meta), "user@host");
    }

    #[test]
    fn test_handle_append_block_file() {
        let broker = wps::Broker::new();

        // Subscribe to block file events
        broker.subscribe(
            "test-route",
            wps::SubscriptionRequest {
                event: wps::EVENT_BLOCK_FILE.to_string(),
                scopes: vec!["block:block-1".to_string()],
                allscopes: false,
            },
        );

        handle_append_block_file(&broker, "block-1", "term", b"hello world", None, None);

        // Check event was published
        let _history = broker.read_event_history(wps::EVENT_BLOCK_FILE, "block:block-1", 10);
        // Note: events are only persisted if persist > 0, so we verify via the publish mechanism
        // The broker successfully processed without panic, which verifies correctness
    }

    /// Helper: read all non-blank line offsets back out of a rebuilt output.idx,
    /// returning (covered_size, offsets).
    #[cfg(test)]
    fn read_idx(fs: &FileStore, block_id: &str) -> (u64, Vec<u64>) {
        let raw = fs.read_file(block_id, "output.idx").unwrap().unwrap();
        let covered = u64::from_le_bytes(raw[0..8].try_into().unwrap());
        let offsets = raw[8..]
            .chunks_exact(8)
            .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
            .collect();
        (covered, offsets)
    }

    #[test]
    fn test_rebuild_output_idx_basic() {
        use crate::backend::storage::filestore::FileStore;
        let fs = FileStore::open_in_memory().expect("filestore");
        let bid = "idx-block";
        let data = b"line0\nline1\nline2\n";
        fs.make_file(bid, "output", Default::default(), Default::default()).unwrap();
        fs.append_data(bid, "output", data).unwrap();

        let n = rebuild_output_idx(&fs, bid, data.len() as u64).unwrap();
        assert_eq!(n, 3);
        let (covered, offsets) = read_idx(&fs, bid);
        assert_eq!(covered, data.len() as u64);
        // "line0\n"=0, "line1\n"=6, "line2\n"=12
        assert_eq!(offsets, vec![0, 6, 12]);
    }

    #[test]
    fn test_rebuild_output_idx_blank_and_crlf_and_no_trailing_nl() {
        use crate::backend::storage::filestore::FileStore;
        let fs = FileStore::open_in_memory().expect("filestore");
        let bid = "idx-block2";
        // Blank line (just spaces), a CRLF line, a blank line, and a final line
        // with no trailing newline. Non-blank lines start at: 0 ("a\n"),
        // 8 ("b\r\n" after "a\n   \n"=6 ... let's compute precisely below).
        // bytes: "a\n"   (0..2)
        //        "   \n" (2..6)   blank
        //        "b\r\n" (6..9)   non-blank -> offset 6
        //        "\n"    (9..10)  blank
        //        "tail"  (10..14) non-blank, no trailing nl -> offset 10
        let data = b"a\n   \nb\r\n\ntail";
        fs.make_file(bid, "output", Default::default(), Default::default()).unwrap();
        fs.append_data(bid, "output", data).unwrap();

        let n = rebuild_output_idx(&fs, bid, data.len() as u64).unwrap();
        assert_eq!(n, 3, "a, b(crlf), tail are the 3 non-blank lines");
        let (_covered, offsets) = read_idx(&fs, bid);
        assert_eq!(offsets, vec![0, 6, 10]);

        // Sanity: the recorded offsets really do start the expected non-blank lines.
        let full = fs.read_file(bid, "output").unwrap().unwrap();
        assert_eq!(&full[0..1], b"a");
        assert_eq!(&full[6..7], b"b");
        assert_eq!(&full[10..14], b"tail");
    }

    #[test]
    fn test_rebuild_output_idx_empty() {
        use crate::backend::storage::filestore::FileStore;
        let fs = FileStore::open_in_memory().expect("filestore");
        let bid = "idx-empty";
        fs.make_file(bid, "output", Default::default(), Default::default()).unwrap();
        let n = rebuild_output_idx(&fs, bid, 0).unwrap();
        assert_eq!(n, 0);
        let (covered, offsets) = read_idx(&fs, bid);
        assert_eq!(covered, 0);
        assert!(offsets.is_empty());
    }

    #[test]
    fn test_handle_truncate_block_file() {
        let broker = wps::Broker::new();
        // Should not panic
        handle_truncate_block_file(&broker, "block-1", "term");
    }

    /// reagentx finding on PR #2683: a declared-background task detaches
    /// into its own session (`setsid()` in bash_wrap.rs), which means it's
    /// no longer a member of the CLI process's own group — so `stop()`'s
    /// existing group-wide kill can no longer reach it either, on
    /// non-Windows (where `process_tracker` is a no-op stub and was never
    /// the enforcement mechanism to begin with). `stop()`'s new step
    /// queries `db_background_tasks` and kills each `Running` task's own
    /// group by its recorded pid — this proves that step actually reaches
    /// a real, isolated-process-group child, not just that the query runs.
    #[cfg(unix)]
    #[tokio::test]
    async fn stop_kills_a_declared_background_tasks_own_detached_process_group() {
        use std::os::unix::process::CommandExt as _;

        let tmp = tempfile::NamedTempFile::new().unwrap();
        let store = Arc::new(Store::open(tmp.path()).unwrap());

        // A real, disposable child, explicitly made its own process group
        // leader (`process_group(0)`) — mirrors what `setsid()` does for a
        // real declared-background bashwrap invocation (session leader
        // implies group leader too). Without this, `-(pid)` below would
        // target a nonexistent group (safe no-op) rather than actually
        // proving the kill reaches an isolated group the way it must in
        // production.
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .process_group(0)
            .spawn()
            .expect("spawn a disposable `sleep 30` child");
        let pid = child.id();

        store
            .background_task_observe("bg-task-1", "test-stop-kills-bg-block", "sleep 30", 1000, 1000)
            .unwrap();
        store.background_task_set_pid("bg-task-1", pid as i64).unwrap();

        let ctrl = ShellController::new(
            "shell".to_string(),
            "tab-1".to_string(),
            "test-stop-kills-bg-block".to_string(),
            None,
            None,
            Some(store),
            None,
        );

        Controller::stop(&ctrl, true, STATUS_DONE).unwrap();

        // Bounded wait for the SIGTERM (or the KILL_GRACE_SECS SIGKILL
        // backstop, in the unlikely event `sleep` ignored SIGTERM) to
        // actually land — this is real async process teardown, not
        // instantaneous.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if let Ok(Some(_)) = child.try_wait() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the declared-background task's process group should have been killed by stop(), but the child is still alive"
            );
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    #[test]
    fn test_register_and_get_controller() {
        let ctrl: Arc<dyn Controller> = Arc::new(ShellController::new(
            "shell".to_string(),
            "tab-1".to_string(),
            "test-register-block".to_string(),
            None,
            None,
            None,
            None,
        ));

        super::super::register_controller("test-register-block", ctrl.clone());

        let retrieved = super::super::get_controller("test-register-block");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().block_id(), "test-register-block");

        // Cleanup
        super::super::delete_controller("test-register-block");
        assert!(super::super::get_controller("test-register-block").is_none());
    }

    #[test]
    fn test_resync_creates_shell_controller() {
        use crate::backend::obj::Block;

        let mut meta = MetaMapType::new();
        meta.insert(
            "controller".to_string(),
            serde_json::Value::String("shell".to_string()),
        );
        // Disable auto-start so we don't open a real PTY in tests
        meta.insert(
            META_KEY_CMD_RUN_ON_START.to_string(),
            serde_json::Value::Bool(false),
        );

        let block = Block {
            oid: "resync-test-block".to_string(),
            version: 1,
            meta,
            ..Default::default()
        };

        let result = super::super::resync_controller(&block, "tab-1", None, false, None, None, None, None, None, std::sync::Arc::from("test-boot"));
        assert!(result.is_ok());

        let ctrl = super::super::get_controller("resync-test-block");
        assert!(ctrl.is_some());
        assert_eq!(ctrl.unwrap().controller_type(), "shell");

        // Cleanup
        super::super::delete_controller("resync-test-block");
    }

    /// Phase 1.3 integration test: write output via handle_append_block_file with a
    /// FileStore, then verify the data is readable back from the store.
    #[test]
    fn test_handle_append_block_file_writes_to_filestore() {
        use crate::backend::storage::filestore::FileStore;
        use std::sync::Arc;

        let broker = wps::Broker::new();
        let fs = Arc::new(FileStore::open_in_memory().expect("open in-memory filestore"));

        let block_id = "test-block-fs";
        let filename = "output";

        // First append — file does not exist yet; handle_append_block_file must create it lazily.
        let line1 = b"line one\n";
        handle_append_block_file(&broker, block_id, filename, line1, Some(&fs), None);

        // Second append
        let line2 = b"line two\n";
        handle_append_block_file(&broker, block_id, filename, line2, Some(&fs), None);

        // Read back from FileStore
        let data = fs.read_file(block_id, filename)
            .expect("read_file ok")
            .expect("data present");

        let text = String::from_utf8(data).expect("valid utf8");
        assert!(text.contains("line one"), "expected 'line one' in {:?}", text);
        assert!(text.contains("line two"), "expected 'line two' in {:?}", text);

        // Also verify total size matches
        let stat = fs.stat(block_id, filename).unwrap().unwrap();
        assert_eq!(stat.size, (line1.len() + line2.len()) as i64);

        // Verify WPS events were also published (broker path still works)
        broker.subscribe(
            "test-route-fs",
            wps::SubscriptionRequest {
                event: wps::EVENT_BLOCK_FILE.to_string(),
                scopes: vec![format!("block:{}", block_id)],
                allscopes: false,
            },
        );
        // Re-publish one more line to confirm broker still receives events alongside filestore
        handle_append_block_file(&broker, block_id, filename, b"line three\n", Some(&fs), None);
        let stat_after = fs.stat(block_id, filename).unwrap().unwrap();
        assert_eq!(stat_after.size, (line1.len() + line2.len() + b"line three\n".len()) as i64);
    }

    /// Helper: parse a zone's `output.tsidx` sidecar into (off, ms) pairs.
    #[cfg(test)]
    fn read_tsidx(fs: &FileStore, zone: &str) -> Vec<(u64, i64)> {
        let raw = fs
            .read_file(zone, crate::backend::agent_session::TSIDX_FILE)
            .unwrap()
            .unwrap();
        String::from_utf8_lossy(&raw)
            .lines()
            .map(|l| {
                let v: serde_json::Value = serde_json::from_str(l).unwrap();
                (v["off"].as_u64().unwrap(), v["ms"].as_i64().unwrap())
            })
            .collect()
    }

    /// §4.4 of SPEC_AGENT_PANE_SESSION_SCOPED_SCROLLBACK_AND_AGENT_HISTORY_VIEW
    /// _2026_08_09.md: agent transcript appends (global_output_zone = Some)
    /// stamp one `{off, ms}` tsidx record per batch, keyed at the batch's
    /// start byte offset.
    #[test]
    fn tsidx_stamped_per_batch_for_agent_transcript_appends() {
        use crate::backend::storage::filestore::FileStore;
        use std::sync::Arc;

        let broker = wps::Broker::new();
        let fs = Arc::new(FileStore::open_in_memory().unwrap());
        let block_id = "tsidx-agent-block";
        let line1 = b"{\"a\":1}\n";
        let line2 = b"{\"b\":2}\n";

        // Zone name is irrelevant to the per-channel stamp — only its
        // presence gates it. No global store is installed in this test, so
        // the mirror side is a no-op.
        handle_append_block_file(&broker, block_id, "output", line1, Some(&fs), Some("agent:tsidx-test:current"));
        handle_append_block_file(&broker, block_id, "output", line2, Some(&fs), Some("agent:tsidx-test:current"));

        let entries = read_tsidx(&fs, block_id);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, 0);
        assert_eq!(entries[1].0, line1.len() as u64);
        assert!(entries[0].1 > 0, "stamp must be a real unix-ms time");
        assert!(entries[1].1 >= entries[0].1, "stamps monotonic");
    }

    /// PTY `term` data and non-agent blocks (global_output_zone = None) must
    /// never grow a tsidx sidecar.
    #[test]
    fn tsidx_not_written_without_agent_zone() {
        use crate::backend::storage::filestore::FileStore;
        use std::sync::Arc;

        let broker = wps::Broker::new();
        let fs = Arc::new(FileStore::open_in_memory().unwrap());
        let block_id = "tsidx-term-block";

        handle_append_block_file(&broker, block_id, "term", b"prompt$ ", Some(&fs), None);
        handle_append_block_file(&broker, block_id, "output", b"line\n", Some(&fs), None);

        assert!(fs
            .stat(block_id, crate::backend::agent_session::TSIDX_FILE)
            .unwrap()
            .is_none());
    }

    /// The global-zone mirror stamps its own tsidx, offset-keyed against the
    /// GLOBAL zone's output (which can differ from the per-channel offsets).
    #[test]
    fn tsidx_stamped_by_global_mirror() {
        use crate::backend::storage::filestore::FileStore;
        use std::sync::Arc;

        let gfs = Arc::new(FileStore::open_in_memory().unwrap());
        let zone = "agent:tsidx-mirror:current";
        let line1 = b"{\"a\":1}\n";
        let line2 = b"{\"b\":2}\n";

        mirror_append_to_global(&gfs, zone, line1);
        mirror_append_to_global(&gfs, zone, line2);

        let entries = read_tsidx(&gfs, zone);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, 0);
        assert_eq!(entries[1].0, line1.len() as u64);
    }

    /// codex P2 on PR #2508: the tsidx stamp must be the offset the append
    /// ACTUALLY landed at (append_data_at reads size + writes under one
    /// store lock), so interleaved appends can't mislabel a batch.
    #[test]
    fn tsidx_offsets_match_actual_append_positions() {
        use crate::backend::storage::filestore::FileStore;
        use std::sync::Arc;

        let broker = wps::Broker::new();
        let fs = Arc::new(FileStore::open_in_memory().unwrap());
        let block_id = "tsidx-offset-block";
        let lines: [&[u8]; 3] = [b"{\"n\":1}
", b"{\"nn\":22}
", b"{\"nnn\":333}
"];
        for l in lines {
            handle_append_block_file(&broker, block_id, "output", l, Some(&fs), Some("agent:tsidx-offsets:current"));
        }
        let entries = read_tsidx(&fs, block_id);
        assert_eq!(entries.len(), 3);
        let mut expected = 0u64;
        for (i, l) in lines.iter().enumerate() {
            assert_eq!(entries[i].0, expected, "entry {i} offset");
            expected += l.len() as u64;
        }
        // And the offsets agree with the output file's actual size.
        let stat = fs.stat(block_id, "output").unwrap().unwrap();
        assert_eq!(stat.size as u64, expected);
    }

    /// `persist_to_blockfile_silent` (user-message lines) stamps the
    /// per-channel sidecar too — those lines are transcript content.
    #[test]
    fn tsidx_stamped_by_persist_silent() {
        use crate::backend::storage::filestore::FileStore;
        use std::sync::Arc;

        let fs = Arc::new(FileStore::open_in_memory().unwrap());
        let block_id = "tsidx-silent-block";
        let line = b"{\"type\":\"user_message\"}\n";

        super::file_ops::persist_to_blockfile_silent(
            block_id,
            "output",
            line,
            Some(&fs),
            Some("agent:tsidx-silent:current"),
        );

        let entries = read_tsidx(&fs, block_id);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, 0);
    }

    /// Minimal WpsClient that records every event delivered to it, so tests
    /// can assert on the broadcast payload (not just the FileStore side effect).
    struct RecordingClient {
        events: std::sync::Mutex<Vec<wps::WaveEvent>>,
    }

    impl RecordingClient {
        fn new() -> Self {
            Self { events: std::sync::Mutex::new(Vec::new()) }
        }
    }

    impl wps::WpsClient for Arc<RecordingClient> {
        fn send_event(&self, _route_id: &str, event: wps::WaveEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    #[test]
    fn test_handle_append_block_file_broadcasts_start_offset() {
        use crate::backend::storage::filestore::FileStore;

        let broker = wps::Broker::new();
        let client = Arc::new(RecordingClient::new());
        broker.set_client(Box::new(Arc::clone(&client)));
        broker.subscribe(
            "test-route-offset",
            wps::SubscriptionRequest {
                event: wps::EVENT_BLOCK_FILE.to_string(),
                scopes: vec!["block:offset-block".to_string()],
                allscopes: false,
            },
        );

        let fs = Arc::new(FileStore::open_in_memory().expect("open in-memory filestore"));
        let block_id = "offset-block";
        let filename = "term";

        // First append — file doesn't exist yet, so the chunk starts at offset 0.
        handle_append_block_file(&broker, block_id, filename, b"hello ", Some(&fs), None);
        // Second append — file now has 6 bytes, so this chunk starts at offset 6.
        handle_append_block_file(&broker, block_id, filename, b"world\n", Some(&fs), None);

        let events = client.events.lock().unwrap();
        assert_eq!(events.len(), 2);
        let offsets: Vec<Option<u64>> = events
            .iter()
            .map(|e| {
                let data: wps::WSFileEventData =
                    serde_json::from_value(e.data.clone().unwrap()).unwrap();
                data.offset
            })
            .collect();
        assert_eq!(offsets, vec![Some(0), Some(6)]);
    }

    #[test]
    fn test_handle_append_block_file_omits_offset_without_filestore() {
        let broker = wps::Broker::new();
        let client = Arc::new(RecordingClient::new());
        broker.set_client(Box::new(Arc::clone(&client)));
        broker.subscribe(
            "test-route-no-fs",
            wps::SubscriptionRequest {
                event: wps::EVENT_BLOCK_FILE.to_string(),
                scopes: vec!["block:no-fs-block".to_string()],
                allscopes: false,
            },
        );

        handle_append_block_file(&broker, "no-fs-block", "term", b"hi", None, None);

        let events = client.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        let data: wps::WSFileEventData =
            serde_json::from_value(events[0].data.clone().unwrap()).unwrap();
        assert_eq!(data.offset, None);
    }

    // ────────────────────────────────────────────────────────────────
    // Cross-channel global transcript mirror
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn mirror_append_to_global_creates_and_appends() {
        use crate::backend::agent_session::OUTPUT_FILE;
        let gfs = Arc::new(FileStore::open_in_memory().expect("global filestore"));
        let zone = "agent:def-mirror-1:current";

        // First append creates the file lazily.
        mirror_append_to_global(&gfs, zone, b"{\"type\":\"user\"}\n");
        // Second append extends it.
        mirror_append_to_global(&gfs, zone, b"{\"type\":\"assistant\"}\n");

        let data = gfs
            .read_file(zone, OUTPUT_FILE)
            .expect("read ok")
            .expect("present");
        let text = String::from_utf8(data).unwrap();
        assert!(text.contains("\"user\""), "got {text:?}");
        assert!(text.contains("\"assistant\""), "got {text:?}");
        // Two NDJSON lines.
        assert_eq!(text.lines().filter(|l| !l.trim().is_empty()).count(), 2);
    }

    #[test]
    fn resolve_global_output_zone_maps_agent_block() {
        let wstore = Arc::new(Store::open_in_memory().expect("wstore"));

        // Agent-anchored block → zone resolved from agentId meta.
        let oid = uuid::Uuid::new_v4().to_string();
        let mut meta = MetaMapType::new();
        meta.insert("view".to_string(), serde_json::json!("agent"));
        meta.insert("agentId".to_string(), serde_json::json!("def-zone-1"));
        let mut block = obj::Block {
            oid: oid.clone(),
            parentoref: String::new(),
            version: 1,
            runtimeopts: None,
            stickers: None,
            meta,
            subblockids: None,
        };
        wstore.insert(&mut block).expect("insert block");

        let some = Some(wstore.clone());
        assert_eq!(
            resolve_global_output_zone(&some, &oid).as_deref(),
            Some("agent:def-zone-1:current"),
        );

        // Unknown block id → None (no crash).
        assert_eq!(resolve_global_output_zone(&some, "no-such-block"), None);
        // No store → None.
        assert_eq!(resolve_global_output_zone(&None, &oid), None);
    }

    // ────────────────────────────────────────────────────────────────
    // extract_agent_events — Phase 1.5 PR 1
    // ────────────────────────────────────────────────────────────────

    use crate::agents::translator::claude::ClaudeTranslator;
    use crate::agents::types::AgentEvent;

    #[test]
    fn extract_agent_events_full_line_translates() {
        let mut t = ClaudeTranslator::new();
        let mut buf: Vec<u8> = Vec::new();
        let line =
            br#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"hello"}}}
"#;
        let events = extract_agent_events(&mut buf, line, &mut t);
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::AssistantText { delta } => assert_eq!(delta, "hello"),
            other => panic!("expected AssistantText, got {other:?}"),
        }
        // Buffer drained.
        assert!(buf.is_empty());
    }

    #[test]
    fn extract_agent_events_chunked_line_accumulates() {
        // PTY can deliver a single logical line across multiple read()
        // calls. The buffer must accumulate across calls and only emit
        // once the newline arrives.
        let mut t = ClaudeTranslator::new();
        let mut buf: Vec<u8> = Vec::new();
        // First chunk: prefix of the JSON, no newline.
        let events = extract_agent_events(
            &mut buf,
            br#"{"type":"stream_event","event":{"type":"content_"#,
            &mut t,
        );
        assert!(events.is_empty());
        // Second chunk: rest of the JSON + newline.
        let events = extract_agent_events(
            &mut buf,
            br#"block_delta","delta":{"type":"text_delta","text":"hi"}}}
"#,
            &mut t,
        );
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::AssistantText { delta } => assert_eq!(delta, "hi"),
            other => panic!("expected AssistantText, got {other:?}"),
        }
    }

    #[test]
    fn extract_agent_events_drops_non_json_lines() {
        // Interactive pane output (ANSI escapes, prompts, blank lines)
        // must not produce events.
        let mut t = ClaudeTranslator::new();
        let mut buf: Vec<u8> = Vec::new();
        let pty_text = b"\x1b[2K\x1b[0;0H> some prompt\n[m\nplain text\n";
        let events = extract_agent_events(&mut buf, pty_text, &mut t);
        assert!(events.is_empty(), "got unexpected events: {events:?}");
        // Buffer drained — only the trailing line (if any) is retained.
        // Here every chunk had a newline at the end, so buf is empty.
        assert!(buf.is_empty());
    }

    #[test]
    fn extract_agent_events_drops_carriage_returns() {
        // PTYs in cooked mode often emit \r\n. Trimming should accept
        // both.
        let mut t = ClaudeTranslator::new();
        let mut buf: Vec<u8> = Vec::new();
        let line = br#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"crlf"}}}
"#;
        // Replace the \n with \r\n.
        let mut bytes: Vec<u8> = line.to_vec();
        let last = bytes.len() - 1;
        bytes.insert(last, b'\r');
        let events = extract_agent_events(&mut buf, &bytes, &mut t);
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn extract_agent_events_drops_malformed_json() {
        // A line that starts with `{` but isn't valid JSON should be
        // silently dropped.
        let mut t = ClaudeTranslator::new();
        let mut buf: Vec<u8> = Vec::new();
        let events = extract_agent_events(&mut buf, b"{not_valid_json\n", &mut t);
        assert!(events.is_empty());
    }

    #[test]
    fn extract_agent_events_resets_oversized_buffer() {
        // A producer that never emits newlines for >1 MiB triggers
        // the buffer reset so memory stays bounded.
        let mut t = ClaudeTranslator::new();
        let mut buf: Vec<u8> = Vec::new();
        let chunk = vec![b'x'; AGENT_LINE_BUFFER_CAP + 1];
        let events = extract_agent_events(&mut buf, &chunk, &mut t);
        assert!(events.is_empty());
        assert!(
            buf.is_empty(),
            "buffer should reset past cap, was {} bytes",
            buf.len()
        );
    }

    #[test]
    fn extract_agent_events_preserves_utf8_across_read_boundary() {
        // Reagent P1 / Codex P2 on PR #833: a multi-byte UTF-8
        // character split across two PTY reads must decode cleanly
        // once the complete line arrives, not lossy-decode each
        // half into U+FFFD.
        //
        // 'こんにちは' = e3 81 93 / e3 82 93 / e3 81 ab / e3 81 a1 / e3 81 af
        // Each char is 3 bytes. Split a line mid-character to verify
        // the buffered-bytes path preserves the codepoint.
        let mut t = ClaudeTranslator::new();
        let mut buf: Vec<u8> = Vec::new();
        let frame = r#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"こんにちは"}}}
"#;
        let frame_bytes = frame.as_bytes();
        // Split at byte 60 — somewhere inside one of the multi-byte
        // sequences of こんにちは (which starts around byte 75 in the
        // JSON). Use a position that's clearly inside the codepoint
        // run.
        let split = frame_bytes
            .iter()
            .position(|&b| b == 0xe3)
            .expect("expected to find a multi-byte codepoint")
            + 1; // split right after the leading byte (mid-codepoint)
        let (a, b) = frame_bytes.split_at(split);
        let events = extract_agent_events(&mut buf, a, &mut t);
        // First chunk had no newline.
        assert!(events.is_empty());
        let events = extract_agent_events(&mut buf, b, &mut t);
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::AssistantText { delta } => {
                assert_eq!(
                    delta, "こんにちは",
                    "UTF-8 must round-trip cleanly across read boundary; got {delta:?}"
                );
                assert!(!delta.contains('\u{FFFD}'), "no replacement chars");
            }
            other => panic!("expected AssistantText, got {other:?}"),
        }
    }

    #[test]
    fn extract_agent_events_two_lines_one_chunk() {
        // PTY can also deliver multiple complete lines in a single
        // read(). Verify they're all processed.
        let mut t = ClaudeTranslator::new();
        let mut buf: Vec<u8> = Vec::new();
        let two_lines = br#"{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"a"}}}
{"type":"stream_event","event":{"type":"content_block_delta","delta":{"type":"text_delta","text":"b"}}}
"#;
        let events = extract_agent_events(&mut buf, two_lines, &mut t);
        assert_eq!(events.len(), 2);
    }
