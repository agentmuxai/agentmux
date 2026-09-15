// Test fixture compiled by app_server.rs at test runtime. It intentionally uses
// only std so it is a deterministic stand-alone child process on every target.

use std::io::{self, BufRead, Write};

fn main() {
    let mode =
        std::env::var("AGENTMUX_FAKE_APP_SERVER_MODE").unwrap_or_else(|_| "happy".to_string());
    if mode == "exit-before-init" {
        eprintln!("fake server deliberately exited before initialize");
        std::process::exit(23);
    }

    eprintln!("fake server started");
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let initialize = lines.next().transpose().unwrap().unwrap_or_default();
    let valid_initialize = initialize.contains("\"id\":1")
        && initialize.contains("\"method\":\"initialize\"")
        && initialize.contains("\"name\":\"agentmux\"")
        && initialize.contains("\"title\":\"AgentMux\"")
        && initialize.contains("\"experimentalApi\":false")
        && !initialize.contains("\"jsonrpc\"");
    if !valid_initialize {
        eprintln!("invalid initialize: {initialize}");
        std::process::exit(24);
    }

    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    if mode == "protocol-violation" {
        eprintln!("fake server emitted protocol violation");
        stdout.write_all(b"not-json\n").unwrap();
        stdout.flush().unwrap();
        for line in lines {
            if line.is_err() {
                std::process::exit(26);
            }
        }
        return;
    }
    stdout
        .write_all(
            b"{\"method\":\"server/ready\",\"params\":{\"fixture\":true}}\r\n\
              {\"id\":1,\"result\":{\"serverInfo\":{\"name\":\"fake-app-server\"}}}\n",
        )
        .unwrap();
    stdout.flush().unwrap();

    let initialized = lines.next().transpose().unwrap().unwrap_or_default();
    if !initialized.contains("\"method\":\"initialized\"")
        || initialized.contains("\"params\"")
        || initialized.contains("\"jsonrpc\"")
    {
        eprintln!("invalid initialized notification: {initialized}");
        std::process::exit(25);
    }
    if mode == "exit-after-init" {
        eprintln!("fake server failed after initialize");
        std::process::exit(27);
    }
    if mode == "eof-after-init" {
        eprintln!("fake server returned EOF after initialize");
        return;
    }
    if mode == "ignores-shutdown" {
        // Unlike every other mode, this one does NOT exit when stdin
        // closes (the graceful-shutdown signal) — it keeps running until
        // actually killed. Used to exercise AppServerProcess::shutdown()'s
        // force-kill-after-timeout path for real, rather than the fixture
        // cooperatively exiting on its own the moment the pipe closes.
        eprintln!("fake server ignoring shutdown, waiting to be killed");
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }

    // Both remaining modes complete a `thread/start` round-trip so the
    // controller reaches a live session, then diverge — used by
    // app_server_controller.rs's own tests (as opposed to the transport-level
    // tests above, which never get this far).
    if mode == "exit-after-thread-start" || mode == "ready-for-turn" {
        let thread_start = lines.next().transpose().unwrap().unwrap_or_default();
        let thread_request_id = request_id(&thread_start).unwrap_or_else(|| {
            eprintln!("thread/start request missing an id: {thread_start}");
            std::process::exit(28);
        });
        stdout
            .write_all(
                format!(
                    "{{\"id\":{thread_request_id},\"result\":{{\"thread\":{{\"id\":\"fixture-thread\"}}}}}}\n"
                )
                .as_bytes(),
            )
            .unwrap();
        stdout.flush().unwrap();

        if mode == "exit-after-thread-start" {
            eprintln!("fake server crashing after thread/start");
            std::process::exit(42);
        }

        // mode == "ready-for-turn": also complete one `turn/start` round-trip,
        // then idle — simulating a turn that is still in progress server-side
        // when the client attempts to start a second one.
        let turn_start = lines.next().transpose().unwrap().unwrap_or_default();
        let turn_request_id = request_id(&turn_start).unwrap_or_else(|| {
            eprintln!("turn/start request missing an id: {turn_start}");
            std::process::exit(29);
        });
        stdout
            .write_all(
                format!(
                    "{{\"id\":{turn_request_id},\"result\":{{\"turn\":{{\"id\":\"fixture-turn\"}}}}}}\n"
                )
                .as_bytes(),
            )
            .unwrap();
        stdout.flush().unwrap();
    }

    for line in lines {
        if line.is_err() {
            std::process::exit(26);
        }
    }
    eprintln!("fake server observed EOF");
}

/// Extract the bare `"id":<n>` value from a JSON-RPC request line. Deliberately
/// not a real JSON parse — this fixture intentionally uses only std (see the
/// header comment) and every request this fixture handles has `id` as a
/// top-level unsigned integer field.
fn request_id(line: &str) -> Option<&str> {
    let after = line.split("\"id\":").nth(1)?;
    let digits = after.split(|c: char| !c.is_ascii_digit()).next()?;
    (!digits.is_empty()).then_some(digits)
}
