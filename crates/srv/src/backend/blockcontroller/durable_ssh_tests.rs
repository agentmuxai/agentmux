// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tests for durable_ssh.rs (moved out to keep that file under its size cap).

use super::*;
use crate::backend::remote::conn::SshDest;

#[test]
fn only_a_helper_session_id_reaches_a_command_line() {
    assert!(valid_session_id("amx-0123abcd") && valid_session_id("a_b-C"));
    for bad in [
        "",
        "bad/id; touch /tmp/pwn",
        "a b",
        "$(id)",
        &"x".repeat(65),
    ] {
        assert!(!valid_session_id(bad), "{bad:?}");
    }
}

#[test]
fn reconnecting_backs_off_and_never_stops() {
    let waits: Vec<u64> = (0..8).map(|a| backoff(a).as_secs()).collect();
    assert_eq!(waits, [1, 2, 5, 10, 30, 30, 30, 30]);
    assert_eq!(backoff(u32::MAX).as_secs(), 30);
}

#[test]
fn output_is_appended_once_and_gaps_are_named() {
    assert_eq!(accept(10, 0, 5), Accept::Seen, "all before what was seen");
    assert_eq!(accept(10, 5, 5), Accept::Seen);
    assert_eq!(
        accept(10, 5, 8),
        Accept::Append { skip: 5, next: 13 },
        "overlap: only the new part"
    );
    assert_eq!(accept(10, 10, 4), Accept::Append { skip: 0, next: 14 });
    assert_eq!(
        accept(10, 12, 3),
        Accept::Gap {
            from: 10,
            to: 12,
            next: 15
        }
    );
}

#[test]
fn the_attach_command_names_the_versioned_helper() {
    let c = attach_command("amx-1", 42, 120, 40);
    assert!(c.starts_with("~/.agentmux-remote/bin/"), "{c}");
    assert!(
        c.ends_with("/agentmux-remote attach --session amx-1 --offset 42 --cols 120 --rows 40"),
        "{c}"
    );
}

/// A stand-in for `ssh` + the helper, speaking the helper's real frames:
/// the first connection attaches, sends "hello" and drops (exit 255, a
/// network drop); the reconnect must ask for exactly offset 7, gets
/// "world" and the session's end.
const FAKE_SSH: &str = r#"
import os, struct, sys, time
remote = sys.argv[-1].split()
offset = int(remote[remote.index('--offset') + 1])
state = os.environ['FAKE_SSH_STATE']
n = int(open(state).read()) if os.path.exists(state) else 0
open(state, 'w').write(str(n + 1))
with open(os.environ['FAKE_SSH_LOG'], 'ab') as log:
    log.write(b'run %d offset %d\n' % (n, offset))
out = sys.stdout.buffer
def frame(kind, payload):
    out.write(struct.pack('>BI', kind, len(payload)) + payload)
    out.flush()
stream = b'hello\r\n' + b'world\r\n'
if n == 0:
    frame(14, struct.pack('>Q', 0) + bytes([1]) + b'amx-test')
    frame(10, struct.pack('>Q', 0) + stream[:7])
    sys.exit(255)
frame(14, struct.pack('>Q', 7) + bytes([0]) + b'amx-test')
frame(10, struct.pack('>Q', offset) + stream[offset:])
frame(12, struct.pack('>i', 0))
time.sleep(5)
"#;

fn python() -> Option<std::path::PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["python", "py"]
    } else {
        &["python3", "python"]
    };
    names.iter().find_map(|n| which::which(n).ok())
}

/// `ssh` for a test: Python running `script` (a fake host), as `name` in
/// `dir`. `None`, to skip the test, where there is no Python.
fn fake_ssh(dir: &std::path::Path, name: &str, script: &str) -> Option<std::path::PathBuf> {
    let python = python()?;
    let script_path = dir.join(format!("{name}.py"));
    std::fs::write(&script_path, script).unwrap();
    if cfg!(windows) {
        let cmd = dir.join(format!("{name}.cmd"));
        let line = format!("@\"{}\" \"{}\" %*\r\n", python.display(), script_path.display());
        std::fs::write(&cmd, line).unwrap();
        return Some(cmd);
    }
    let sh = dir.join(name);
    let line = format!("#!/bin/sh\nexec \"{}\" \"{}\" \"$@\"\n", python.display(), script_path.display());
    std::fs::write(&sh, line).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&sh, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    Some(sh)
}

/// Held by the tests that point `AGENTMUX_REMOTE_HELPER_DIR` at their own
/// builds: the environment is the whole process's.
static HELPER_DIR_ENV: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// A durable pane's run for a test: pane `block` on host `conn`, reached
/// by `ssh_path`, its session `session`.
fn test_run(state: &crate::server::AppState, block: &str, conn: &str, session: &str, ssh_path: std::path::PathBuf) -> Run {
    Run {
        block_id: block.to_string(),
        conn: conn.to_string(),
        host: HostSsh {
            dest: SshDest {
                destination: conn.to_string(),
                port: None,
            },
            ssh_path,
            control_dir: None,
            env: Vec::new(),
        },
        session: session.to_string(),
        size: (80, 24),
        broker: Some(state.broker.clone()),
        filestore: Some(state.filestore.clone()),
        _askpass_grant: None,
        plain: Arc::new(AtomicBool::new(false)),
        by_default: false,
    }
}

/// The reconnect loop for real: a dropped link reattaches at exactly the
/// offset it had reached, nothing is appended twice, and the session's
/// own end ends the pane's run.
#[tokio::test]
async fn a_dropped_link_reattaches_at_its_offset_and_appends_nothing_twice() {
    let dir = tempfile::tempdir().unwrap();
    let Some(ssh_path) = fake_ssh(dir.path(), "fake_ssh", FAKE_SSH) else {
        eprintln!("skipped: no python to stand in for ssh");
        return;
    };
    std::env::set_var("FAKE_SSH_STATE", dir.path().join("state"));
    std::env::set_var("FAKE_SSH_LOG", dir.path().join("log"));

    let state = crate::server::tests::test_state();
    let block = "durable-test-block";
    let run = test_run(&state, block, "fakehost", "amx-test", ssh_path);
    let (_input_tx, input_rx) = mpsc::unbounded_channel();
    let (_leave_tx, leave_rx) = watch::channel(Leave::Stay);
    let ended = tokio::time::timeout(
        Duration::from_secs(30),
        run.run(input_rx, leave_rx, Box::new(|_| {})),
    )
    .await
    .expect("the run ends");
    assert_eq!(
        ended,
        Some(Done {
            code: 0,
            session_over: true
        }),
        "the session's own end ends the run"
    );

    let log = std::fs::read_to_string(dir.path().join("log")).unwrap();
    assert_eq!(
        log, "run 0 offset 0\nrun 1 offset 7\n",
        "the reconnect asks for exactly what it missed"
    );
    let term = state.filestore.read_file(block, "term").unwrap().unwrap();
    let text = String::from_utf8_lossy(&term).into_owned();
    assert_eq!(text.matches("hello").count(), 1, "{text:?}");
    assert_eq!(text.matches("world").count(), 1, "{text:?}");
    assert!(
        text.find("hello") < text.find("reconnecting")
            && text.find("reconnecting") < text.find("world"),
        "{text:?}"
    );
    let offset = state
        .filestore
        .stat(block, "term")
        .unwrap()
        .unwrap()
        .meta
        .get(FILE_META_REMOTE_OFFSET)
        .cloned();
    // Over, so the next session starts from 0 (no "the previous session
    // is gone" for a plain `exit`).
    assert_eq!(offset, Some(serde_json::json!(0)));
}

/// Wait (up to 10 s) until a fake ssh's log starts with `prefix`.
async fn until_logged(log: &std::path::Path, prefix: &str) {
    let logged = tokio::time::timeout(Duration::from_secs(10), async {
        while !std::fs::read_to_string(log).is_ok_and(|c| c.starts_with(prefix)) {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    });
    logged.await.unwrap_or_else(|_| panic!("nothing logged starting with {prefix:?}"));
}

/// Every attach fails (the host is unreachable); `end` is logged.
const FAKE_SSH_DOWN: &str = r#"
import os, sys
remote = sys.argv[-1].split()
with open(os.environ['FAKE_SSH_LOG2'], 'ab') as log:
    log.write((' '.join(remote[1:3]) + '\n').encode())
sys.exit(0 if remote[1] == 'end' else 255)
"#;

/// A pane closed while its host is unreachable still ends its session
/// there (one `agentmux-remote end`), rather than leaving the shell
/// running with nothing able to reach it.
#[tokio::test]
async fn closing_while_disconnected_still_ends_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let Some(ssh_path) = fake_ssh(dir.path(), "fake_ssh_down", FAKE_SSH_DOWN) else {
        eprintln!("skipped: no python to stand in for ssh");
        return;
    };
    let log = dir.path().join("log2");
    std::env::set_var("FAKE_SSH_LOG2", &log);

    let state = crate::server::tests::test_state();
    let run = test_run(&state, "durable-down-block", "downhost", "amx-down", ssh_path);
    let (_input_tx, input_rx) = mpsc::unbounded_channel();
    let (leave_tx, leave_rx) = watch::channel(Leave::Stay);
    let task = tokio::spawn(run.run(input_rx, leave_rx, Box::new(|_| {})));
    // Let it fail an attach and start backing off, then close the pane.
    // Waited for, not slept: under load the first ssh can take longer
    // than any fixed wait to start.
    until_logged(&log, "attach").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    leave_tx.send(Leave::End).unwrap();
    let ended = tokio::time::timeout(Duration::from_secs(20), task)
        .await
        .expect("the run ends")
        .unwrap();
    assert_eq!(ended, None, "closed, not the session's own end");
    let calls = std::fs::read_to_string(&log).unwrap();
    assert!(calls.lines().any(|l| l.starts_with("attach")), "{calls:?}");
    assert_eq!(
        calls.lines().last(),
        Some("end --session"),
        "the last ssh ends the session: {calls:?}"
    );
}

/// A host slow to answer: attach hangs, as ssh does while its connection
/// is being made; `end` answers at once.
const FAKE_SSH_SLOW: &str = r#"
import os, sys, time
remote = sys.argv[-1].split()
with open(os.environ['FAKE_SSH_LOG_SLOW'], 'ab') as log:
    log.write((' '.join(remote[1:3]) + '\n').encode())
if remote[1] == 'end':
    sys.exit(0)
time.sleep(30)
sys.exit(255)
"#;

/// Closed while its ssh is still trying to reach the session (the host
/// slow or down, a reconnect under way): the End went down an ssh that
/// never reached the session, which ran on. The test above caught this
/// only when the close happened to land mid-attach, under load.
#[tokio::test]
async fn closing_while_still_connecting_ends_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let Some(ssh_path) = fake_ssh(dir.path(), "fake_ssh_slow", FAKE_SSH_SLOW) else {
        eprintln!("skipped: no python to stand in for ssh");
        return;
    };
    let log = dir.path().join("log_slow");
    std::env::set_var("FAKE_SSH_LOG_SLOW", &log);

    let state = crate::server::tests::test_state();
    let run = test_run(&state, "durable-slow-block", "slowhost", "amx-slow", ssh_path);
    let (_input_tx, input_rx) = mpsc::unbounded_channel();
    let (leave_tx, leave_rx) = watch::channel(Leave::Stay);
    let task = tokio::spawn(run.run(input_rx, leave_rx, Box::new(|_| {})));
    // Close the pane while its first attach is still connecting.
    until_logged(&log, "attach").await;
    leave_tx.send(Leave::End).unwrap();
    let ended = tokio::time::timeout(Duration::from_secs(20), task)
        .await
        .expect("the run ends")
        .unwrap();
    assert_eq!(ended, None, "closed, not the session's own end");
    let calls = std::fs::read_to_string(&log).unwrap();
    assert_eq!(calls.lines().collect::<Vec<_>>(), ["attach --session", "end --session"], "{calls:?}");
}

/// A host without the helper: attach fails with 127 until the helper is
/// installed; the probe, upload and install steps behave as on a real
/// host, the install step checking the uploaded file's hash itself.
const FAKE_SSH_FRESH_HOST: &str = r#"
import hashlib, os, struct, sys
remote = sys.argv[-1]
d = os.environ['FAKE_HOST_DIR']
log = open(os.path.join(d, 'log'), 'ab', buffering=0)
helper = os.path.join(d, 'helper')
if remote.startswith('sh -c') and 'uname -sm' in remote:
    log.write(b'probe\n')
    sys.stdout.write('Linux x86_64\n')
elif remote.startswith('sh -c') and 'cat >' in remote:
    log.write(b'upload\n')
    open(helper + '.tmp', 'wb').write(sys.stdin.buffer.read())
elif remote.startswith('sh -c') and 'mv -f' in remote:
    log.write(b'install\n')
    got = hashlib.sha256(open(helper + '.tmp', 'rb').read()).hexdigest()
    if got in remote:
        os.replace(helper + '.tmp', helper)
        sys.stdout.write('ok\n')
    else:
        sys.stdout.write('mismatch ' + got + '\n')
elif ' attach ' in remote:
    if not os.path.exists(helper):
        log.write(b'attach-missing\n')
        sys.exit(127)
    log.write(b'attach\n')
    out = sys.stdout.buffer
    def frame(kind, payload):
        out.write(struct.pack('>BI', kind, len(payload)) + payload)
        out.flush()
    frame(14, struct.pack('>Q', 0) + bytes([1]) + b'amx-fresh')
    frame(10, struct.pack('>Q', 0) + b'ready\r\n')
    frame(12, struct.pack('>i', 0))
    import time; time.sleep(5)
"#;

/// A first durable pane on a host without the helper installs it (probe,
/// upload, hash-checked install) and then attaches.
#[tokio::test]
async fn a_host_without_the_helper_gets_it_installed_then_attaches() {
    let dir = tempfile::tempdir().unwrap();
    let Some(ssh_path) = fake_ssh(dir.path(), "fake_ssh_fresh", FAKE_SSH_FRESH_HOST) else {
        eprintln!("skipped: no python to stand in for ssh");
        return;
    };
    std::env::set_var("FAKE_HOST_DIR", dir.path());
    // The user says Install when asked.
    crate::backend::remote::helper_consent::install_answering_yes();
    // The "release" build, from a local folder.
    let builds = dir.path().join("builds");
    std::fs::create_dir_all(&builds).unwrap();
    let name = crate::backend::remote::helper_install::asset_name(
        env!("CARGO_PKG_VERSION"),
        "x86_64-unknown-linux-musl",
    );
    std::fs::write(builds.join(name), b"the helper").unwrap();
    let _env = HELPER_DIR_ENV.lock().await;
    std::env::set_var("AGENTMUX_REMOTE_HELPER_DIR", &builds);

    let state = crate::server::tests::test_state();
    let run = test_run(&state, "durable-fresh-block", "freshhost", "amx-fresh", ssh_path);
    let (_input_tx, input_rx) = mpsc::unbounded_channel();
    let (_leave_tx, leave_rx) = watch::channel(Leave::Stay);
    let ended = tokio::time::timeout(
        Duration::from_secs(30),
        run.run(input_rx, leave_rx, Box::new(|_| {})),
    )
    .await
    .expect("the run ends");
    std::env::remove_var("AGENTMUX_REMOTE_HELPER_DIR");
    assert_eq!(
        ended,
        Some(Done {
            code: 0,
            session_over: true
        })
    );
    let log = std::fs::read_to_string(dir.path().join("log")).unwrap();
    assert_eq!(log, "attach-missing\nprobe\nupload\ninstall\nattach\n");
    let term = state
        .filestore
        .read_file("durable-fresh-block", "term")
        .unwrap()
        .unwrap();
    let text = String::from_utf8_lossy(&term).into_owned();
    assert!(
        text.contains("Installing AgentMux's helper on freshhost") && text.contains("ready"),
        "{text:?}"
    );
}

/// A host the helper has no build for: its probe answers FreeBSD, and
/// attach finds no helper (127). As real ssh does, attach closes its
/// output a moment before it exits: killing it at the end of its output
/// lost the 127, and the pane reconnected for ever instead of installing.
const FAKE_SSH_NO_BUILD: &str = r#"
import os, sys, time
remote = sys.argv[-1]
with open(os.environ['FAKE_SSH_LOG_NO_BUILD'], 'ab') as log:
    if 'uname -sm' in remote:
        log.write(b'probe\n')
        sys.stdout.write('FreeBSD amd64\n')
        sys.exit(0)
    log.write(b'attach-missing\n')
os.close(1)
os.close(2)
time.sleep(0.5)
os._exit(127)
"#;

/// When durable is only offered (nothing set, no helper yet), a helper
/// that can't be installed leaves a plain SSH terminal, and the host's
/// next panes are plain until srv restarts. When the user turned durable
/// on, the pane stops and says why, as before.
#[tokio::test]
async fn a_helper_that_cant_be_installed_leaves_a_plain_terminal_unless_durable_was_asked_for() {
    let dir = tempfile::tempdir().unwrap();
    let Some(ssh_path) = fake_ssh(dir.path(), "fake_ssh_no_build", FAKE_SSH_NO_BUILD) else {
        eprintln!("skipped: no python to stand in for ssh");
        return;
    };
    std::env::set_var("FAKE_SSH_LOG_NO_BUILD", dir.path().join("log"));
    crate::backend::remote::helper_consent::install_answering_yes();
    let state = crate::server::tests::test_state();
    let meta = |conn: &str, durable: Option<bool>| {
        let mut m = MetaMapType::new();
        m.insert(super::super::META_KEY_CONNECTION.into(), serde_json::json!(conn));
        if let Some(d) = durable {
            m.insert(META_KEY_DURABLE.into(), serde_json::json!(d));
        }
        m
    };
    let run_once = |block: &'static str, conn: &'static str, by_default: bool| {
        let mut run = test_run(&state, block, conn, "amx-no-build", ssh_path.clone());
        run.by_default = by_default;
        let plain = run.plain.clone();
        async move {
            let (_input_tx, input_rx) = mpsc::unbounded_channel();
            let (_leave_tx, leave_rx) = watch::channel(Leave::Stay);
            let ended = tokio::time::timeout(Duration::from_secs(20), run.run(input_rx, leave_rx, Box::new(|_| {})))
                .await
                .expect("the run ends");
            (ended, plain.load(Ordering::SeqCst))
        }
    };
    let text = |block| String::from_utf8_lossy(&state.filestore.read_file(block, "term").unwrap().unwrap()).into_owned();

    let (ended, plain) = run_once("durable-offered-block", "offered-bsdhost", true).await;
    assert_eq!((ended, plain), (None, true), "a plain terminal instead");
    let note = text("durable-offered-block");
    assert!(note.contains("plain SSH terminal") && note.contains("FreeBSD"), "{note:?}");
    assert_eq!(mode(&meta("offered-bsdhost", None), None), Mode::Plain, "not durable by default again");
    assert_eq!(mode(&meta("offered-bsdhost", Some(true)), None), Mode::Chosen, "unless asked for");

    let (ended, plain) = run_once("durable-asked-block", "asked-bsdhost", false).await;
    assert_eq!(ended, Some(Done { code: EXIT_COMMAND_NOT_FOUND, session_over: true }));
    assert!(!plain);
    assert!(text("durable-asked-block").contains("can't stay alive"));
}

#[test]
fn a_refused_login_is_told_from_a_dropped_link() {
    assert!(login_refused(
        "user@box: Permission denied (publickey,password)."
    ));
    assert!(login_refused("Host key verification failed."));
    assert!(login_refused(
        "@    WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!     @"
    ));
    assert!(!login_refused(
        "ssh: connect to host box port 22: Connection timed out"
    ));
    assert!(!login_refused("Connection reset by peer"));
    assert!(!login_refused(""));
}

/// A Windows host: its `cmd` doesn't know the helper's command.
const FAKE_SSH_WINDOWS: &str = r#"
import os, sys
with open(os.environ['FAKE_SSH_LOG4'], 'ab') as log:
    log.write((sys.argv[-1].split()[1] + '\n').encode())
sys.stderr.write("'~' is not recognized as an internal or external command,\noperable program or batch file.\n")
sys.exit(1)
"#;

/// A Windows host makes the pane a plain SSH terminal at once, instead
/// of reconnecting for ever: the run says so and ends without a `done`,
/// for the controller to restart the pane (its `plain` flag).
#[tokio::test]
async fn a_windows_host_turns_the_pane_into_a_plain_ssh_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let Some(ssh_path) = fake_ssh(dir.path(), "fake_ssh_windows", FAKE_SSH_WINDOWS) else {
        eprintln!("skipped: no python to stand in for ssh");
        return;
    };
    std::env::set_var("FAKE_SSH_LOG4", dir.path().join("log"));

    let state = crate::server::tests::test_state();
    let block = "durable-windows-block";
    let plain = Arc::new(AtomicBool::new(false));
    let mut run = test_run(&state, block, "windowshost", "amx-windows", ssh_path);
    run.plain = plain.clone();
    let (_input_tx, input_rx) = mpsc::unbounded_channel();
    let (_leave_tx, leave_rx) = watch::channel(Leave::Stay);
    let ended = tokio::time::timeout(
        Duration::from_secs(15),
        run.run(input_rx, leave_rx, Box::new(|_| panic!("no done: the pane is restarted"))),
    )
    .await
    .expect("the run ends at once, without reconnecting");
    assert_eq!(ended, None);
    assert!(plain.load(Ordering::SeqCst), "the pane is to become a plain terminal");
    assert!(crate::backend::remote::helper_consent::plain_instead("windowshost"));
    let mut meta = MetaMapType::new();
    meta.insert(super::super::META_KEY_CONNECTION.into(), serde_json::json!("windowshost"));
    meta.insert(META_KEY_DURABLE.into(), serde_json::json!(true));
    assert!(!wants(&meta, None), "durable no more, whatever a helper record says");
    let log = std::fs::read_to_string(dir.path().join("log")).unwrap();
    assert_eq!(log, "attach\n", "tried once, no install");
    let term = state.filestore.read_file(block, "term").unwrap().unwrap();
    assert!(String::from_utf8_lossy(&term).contains("plain SSH terminal"));
}

/// A Windows host in another language: its `cmd` answers in German, and
/// only `cmd /c echo %OS%` tells it apart.
const FAKE_SSH_WINDOWS_DE: &str = r#"
import os, sys
cmd = sys.argv[-1]
with open(os.environ['FAKE_SSH_LOG5'], 'ab') as log:
    log.write((cmd.split()[0] if cmd.startswith('cmd') else cmd.split()[1]).encode() + b'\n')
if cmd.startswith('cmd /c echo'):
    sys.stdout.write('Windows_NT"\n')
    sys.exit(0)
sys.stderr.write("Der Befehl \"~\" ist entweder falsch geschrieben oder\nkonnte nicht gefunden werden.\n")
sys.exit(1)
"#;

#[tokio::test]
async fn a_windows_host_in_another_language_is_found_by_asking_it() {
    let dir = tempfile::tempdir().unwrap();
    let Some(ssh_path) = fake_ssh(dir.path(), "fake_ssh_windows_de", FAKE_SSH_WINDOWS_DE) else {
        eprintln!("skipped: no python to stand in for ssh");
        return;
    };
    std::env::set_var("FAKE_SSH_LOG5", dir.path().join("log"));

    let state = crate::server::tests::test_state();
    let plain = Arc::new(AtomicBool::new(false));
    let mut run = test_run(&state, "durable-windows-de-block", "windowshost-de", "amx-windows-de", ssh_path);
    run.plain = plain.clone();
    let (_input_tx, input_rx) = mpsc::unbounded_channel();
    let (_leave_tx, leave_rx) = watch::channel(Leave::Stay);
    let ended = tokio::time::timeout(
        Duration::from_secs(20),
        run.run(input_rx, leave_rx, Box::new(|_| panic!("no done: the pane is restarted"))),
    )
    .await
    .expect("the run ends at once, without reconnecting");
    assert_eq!(ended, None);
    assert!(plain.load(Ordering::SeqCst));
    assert!(crate::backend::remote::helper_consent::plain_instead("windowshost-de"));
    let log = std::fs::read_to_string(dir.path().join("log")).unwrap();
    assert_eq!(log, "attach\ncmd\n", "one attach, then the host asked once");
}

#[test]
fn only_a_reply_from_the_host_settles_whether_it_is_windows() {
    use crate::backend::remote::host::HostOutput;
    let out = |code: Option<i32>, stdout: &str| HostOutput {
        code,
        stdout: stdout.to_string(),
        stderr: String::new(),
    };
    assert_eq!(windows_answer(&out(Some(0), "Windows_NT\"\n")), Some(true));
    assert_eq!(windows_answer(&out(Some(127), "")), Some(false), "a POSIX shell without cmd");
    assert_eq!(windows_answer(&out(Some(255), "")), None, "ssh failed: ask again");
    assert_eq!(windows_answer(&out(None, "")), None, "killed: ask again");
}

#[test]
fn windows_shells_are_told_by_their_answer() {
    assert!(windows_shell_refused("'~' is not recognized as an internal or external command,"));
    assert!(windows_shell_refused("The term '~/x' is not recognized as a name of a cmdlet, function"));
    assert!(!windows_shell_refused("bash: ~/.agentmux-remote/bin/x: No such file or directory"));
}

/// The host refuses the login (stderr is the point); each run logs its
/// helper command (`attach` or `end`).
const FAKE_SSH_REFUSED: &str = r#"
import os, sys
with open(os.environ['FAKE_SSH_LOG3'], 'ab') as log:
    log.write((sys.argv[-1].split()[1] + '\n').encode())
sys.stderr.write('user@fakehost: Permission denied (publickey).\n')
sys.exit(255)
"#;

/// A refused login stops the pane at once with the reason, rather than
/// asking again every 30 s, and keeps the session id: the session may
/// still be on the host for the pane's restart to reattach to. Closing
/// the pane then still ends the session there.
#[tokio::test]
async fn a_refused_login_stops_the_pane_and_keeps_its_session() {
    let dir = tempfile::tempdir().unwrap();
    let Some(ssh_path) = fake_ssh(dir.path(), "fake_ssh_refused", FAKE_SSH_REFUSED) else {
        eprintln!("skipped: no python to stand in for ssh");
        return;
    };
    std::env::set_var("FAKE_SSH_LOG3", dir.path().join("log"));

    let state = crate::server::tests::test_state();
    let block = "durable-refused-block";
    let run = test_run(&state, block, "fakehost", "amx-refused", ssh_path);
    let (_input_tx, input_rx) = mpsc::unbounded_channel();
    let (leave_tx, leave_rx) = watch::channel(Leave::Stay);
    let (stopped_tx, stopped_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(run.run(
        input_rx,
        leave_rx,
        Box::new(move |done| {
            let _ = stopped_tx.send(done);
        }),
    ));
    // Well inside the first backoff: no retry was waited for.
    let stopped = tokio::time::timeout(Duration::from_secs(15), stopped_rx)
        .await
        .expect("the pane is shown done")
        .unwrap();
    assert_eq!(
        stopped,
        Done {
            code: EXIT_SSH_FAILED,
            session_over: false
        }
    );
    let log = std::fs::read_to_string(dir.path().join("log")).unwrap();
    assert_eq!(log, "attach\n", "asked once, not again");

    // Closing the pane: one `end` for the session it kept.
    leave_tx.send(Leave::End).unwrap();
    let ended = tokio::time::timeout(Duration::from_secs(15), task)
        .await
        .expect("the run ends")
        .unwrap();
    assert_eq!(ended, None);
    let log = std::fs::read_to_string(dir.path().join("log")).unwrap();
    assert_eq!(log, "attach\nend\n");
    let term = state.filestore.read_file(block, "term").unwrap().unwrap();
    let text = String::from_utf8_lossy(&term).into_owned();
    assert!(
        text.contains("Could not log in to fakehost")
            && text.contains("Permission denied (publickey)"),
        "{text:?}"
    );
}

/// A host whose helper goes away after the pane attached (Remove helper
/// in Remotes, or deleted by hand): the first attach succeeds and then the
/// helper is gone, so the drop finds no helper (127).
const FAKE_SSH_HELPER_REMOVED: &str = r#"
import hashlib, os, struct, sys
remote = sys.argv[-1]
d = os.environ['FAKE_HOST_DIR_REMOVED']
log = open(os.path.join(d, 'log'), 'ab', buffering=0)
helper = os.path.join(d, 'helper')
out = sys.stdout.buffer
def frame(kind, payload):
    out.write(struct.pack('>BI', kind, len(payload)) + payload)
    out.flush()
if remote.startswith('sh -c') and 'uname -sm' in remote:
    log.write(b'probe\n')
    sys.stdout.write('Linux x86_64\n')
elif remote.startswith('sh -c') and 'cat >' in remote:
    log.write(b'upload\n')
    open(helper + '.tmp', 'wb').write(sys.stdin.buffer.read())
elif remote.startswith('sh -c') and 'mv -f' in remote:
    log.write(b'install\n')
    os.replace(helper + '.tmp', helper)
    sys.stdout.write('ok\n')
elif ' attach ' in remote:
    if not os.path.exists(helper):
        log.write(b'attach-missing\n')
        sys.exit(127)
    log.write(b'attach\n')
    removed = os.path.join(d, 'removed')
    frame(14, struct.pack('>Q', 0) + bytes([1]) + b'amx-removed')
    if not os.path.exists(removed):
        open(removed, 'w').close()
        os.remove(helper)
        sys.exit(255)
    frame(12, struct.pack('>i', 0))
    import time; time.sleep(5)
"#;

/// The helper removed from a host after this pane installed it: the pane
/// installs it again on its next attach rather than stopping with "still
/// not runnable", which is only for an install that didn't take.
#[tokio::test]
async fn a_helper_removed_after_attaching_is_installed_again() {
    let dir = tempfile::tempdir().unwrap();
    let Some(ssh_path) = fake_ssh(dir.path(), "fake_ssh_removed", FAKE_SSH_HELPER_REMOVED) else {
        eprintln!("skipped: no python to stand in for ssh");
        return;
    };
    std::env::set_var("FAKE_HOST_DIR_REMOVED", dir.path());
    crate::backend::remote::helper_consent::install_answering_yes();
    let builds = dir.path().join("builds");
    std::fs::create_dir_all(&builds).unwrap();
    let name = crate::backend::remote::helper_install::asset_name(env!("CARGO_PKG_VERSION"), "x86_64-unknown-linux-musl");
    std::fs::write(builds.join(name), b"the helper").unwrap();
    let _env = HELPER_DIR_ENV.lock().await;
    std::env::set_var("AGENTMUX_REMOTE_HELPER_DIR", &builds);
    let run = test_run(&crate::server::tests::test_state(), "durable-removed-block", "removedhost", "amx-removed", ssh_path);
    let (_input_tx, input_rx) = mpsc::unbounded_channel();
    let (_leave_tx, leave_rx) = watch::channel(Leave::Stay);
    let ended = tokio::time::timeout(Duration::from_secs(30), run.run(input_rx, leave_rx, Box::new(|_| {})))
        .await
        .expect("the run ends");
    std::env::remove_var("AGENTMUX_REMOTE_HELPER_DIR");
    assert_eq!(ended, Some(Done { code: 0, session_over: true }));
    let log = std::fs::read_to_string(dir.path().join("log")).unwrap();
    let install = "attach-missing\nprobe\nupload\ninstall\nattach\n";
    assert_eq!(log, install.repeat(2), "installed, removed, installed again");
}

/// A host where the install says ok but the helper still doesn't run (it is
/// gone again before the attach, or can't execute there): attach is always
/// 127.
const FAKE_SSH_NEVER_RUNNABLE: &str = r#"
import os, sys
remote = sys.argv[-1]
log = open(os.path.join(os.environ['FAKE_HOST_DIR_NEVER'], 'log'), 'ab', buffering=0)
if remote.startswith('sh -c') and 'uname -sm' in remote:
    log.write(b'probe\n')
    sys.stdout.write('Linux x86_64\n')
elif remote.startswith('sh -c') and 'cat >' in remote:
    log.write(b'upload\n')
    sys.stdin.buffer.read()
elif remote.startswith('sh -c') and 'mv -f' in remote:
    log.write(b'install\n')
    sys.stdout.write('ok\n')
else:
    log.write(b'attach-missing\n')
    sys.exit(127)
"#;

/// Installed, and still not runnable: a pane durable only by default becomes
/// a plain SSH terminal, as for a failed install, instead of stopping.
#[tokio::test]
async fn a_helper_still_missing_after_installing_leaves_a_plain_terminal_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let Some(ssh_path) = fake_ssh(dir.path(), "fake_ssh_never", FAKE_SSH_NEVER_RUNNABLE) else {
        eprintln!("skipped: no python to stand in for ssh");
        return;
    };
    std::env::set_var("FAKE_HOST_DIR_NEVER", dir.path());
    crate::backend::remote::helper_consent::install_answering_yes();
    let builds = dir.path().join("builds");
    std::fs::create_dir_all(&builds).unwrap();
    let name = crate::backend::remote::helper_install::asset_name(env!("CARGO_PKG_VERSION"), "x86_64-unknown-linux-musl");
    std::fs::write(builds.join(name), b"the helper").unwrap();
    let _env = HELPER_DIR_ENV.lock().await;
    std::env::set_var("AGENTMUX_REMOTE_HELPER_DIR", &builds);
    let state = crate::server::tests::test_state();
    let mut run = test_run(&state, "durable-never-block", "neverhost", "amx-never", ssh_path);
    run.by_default = true;
    let plain = run.plain.clone();
    let (_input_tx, input_rx) = mpsc::unbounded_channel();
    let (_leave_tx, leave_rx) = watch::channel(Leave::Stay);
    let ended = tokio::time::timeout(Duration::from_secs(30), run.run(input_rx, leave_rx, Box::new(|_| {})))
        .await
        .expect("the run ends");
    std::env::remove_var("AGENTMUX_REMOTE_HELPER_DIR");
    assert_eq!((ended, plain.load(Ordering::SeqCst)), (None, true), "a plain terminal instead");
    let term = state.filestore.read_file("durable-never-block", "term").unwrap().unwrap();
    let text = String::from_utf8_lossy(&term).into_owned();
    assert!(text.contains("plain SSH terminal") && text.contains("still"), "{text:?}");
    assert!(crate::backend::remote::helper_consent::not_installed("neverhost"));
}

/// A host ssh can't reach at all (a mistyped name): every attempt is ssh's
/// own failure, 255.
const FAKE_SSH_UNREACHABLE: &str = r#"
import os, sys
with open(os.environ['FAKE_SSH_LOG_UNREACHABLE'], 'ab') as log:
    log.write(b'attempt\n')
sys.stderr.write('ssh: Could not resolve hostname nosuchhost: nodename nor servname provided, or not known\n')
sys.exit(255)
"#;

/// A first terminal on a host it can't reach, durable only by default and
/// on a host the helper never ran on: after a few tries it becomes a plain
/// SSH terminal (which shows ssh's error and ends, as before durable was the
/// default) instead of reconnecting for ever.
#[tokio::test]
async fn a_new_host_never_reached_becomes_a_plain_terminal_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let Some(ssh_path) = fake_ssh(dir.path(), "fake_ssh_unreachable", FAKE_SSH_UNREACHABLE) else {
        eprintln!("skipped: no python to stand in for ssh");
        return;
    };
    let log = dir.path().join("log");
    std::env::set_var("FAKE_SSH_LOG_UNREACHABLE", &log);
    let state = crate::server::tests::test_state();
    let mut run = test_run(&state, "durable-unreachable-block", "nosuchhost", "amx-unreachable", ssh_path);
    run.by_default = true;
    let plain = run.plain.clone();
    let (_input_tx, input_rx) = mpsc::unbounded_channel();
    let (_leave_tx, leave_rx) = watch::channel(Leave::Stay);
    let ended = tokio::time::timeout(Duration::from_secs(20), run.run(input_rx, leave_rx, Box::new(|_| {})))
        .await
        .expect("the run gives up on the host");
    assert_eq!((ended, plain.load(Ordering::SeqCst)), (None, true), "a plain terminal instead");
    let tries = std::fs::read_to_string(&log).unwrap().lines().count();
    assert_eq!(tries, FIRST_REACH_TRIES as usize);
    // The pane's restart asks `wants` again: it must say plain, or the
    // restart refuses and the pane stops instead.
    let mut meta = MetaMapType::new();
    meta.insert(super::super::META_KEY_CONNECTION.into(), serde_json::json!("nosuchhost"));
    assert!(!wants(&meta, None), "the restart becomes a plain terminal");
    let term = state.filestore.read_file("durable-unreachable-block", "term").unwrap().unwrap();
    assert!(String::from_utf8_lossy(&term).contains("Could not resolve hostname"));
}
