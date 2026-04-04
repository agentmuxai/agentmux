use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

/// Helper: spawn agentmux-srv as a subprocess and parse WAVESRV-ESTART.
/// Returns (child, web_addr, ws_addr, auth_key).
fn spawn_backend() -> (std::process::Child, String, String, String) {
    let auth_key = "integration-test-key-12345";

    let binary = env!("CARGO_BIN_EXE_agentmux-srv");

    let mut child = Command::new(binary)
        .env("WAVETERM_AUTH_KEY", auth_key)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .expect("failed to spawn agentmux-srv");

    let stderr = child.stderr.take().unwrap();
    let reader = BufReader::new(stderr);

    let mut web_addr = String::new();
    let mut ws_addr = String::new();

    for line in reader.lines() {
        let line = line.expect("failed to read stderr");
        if line.contains("WAVESRV-ESTART") {
            let parts: Vec<&str> = line.split_whitespace().collect();
            for part in &parts {
                if let Some(addr) = part.strip_prefix("ws:") {
                    ws_addr = addr.to_string();
                } else if let Some(addr) = part.strip_prefix("web:") {
                    web_addr = addr.to_string();
                }
            }
            break;
        }
    }

    assert!(!web_addr.is_empty(), "failed to parse web addr from ESTART");
    assert!(!ws_addr.is_empty(), "failed to parse ws addr from ESTART");

    (child, web_addr, ws_addr, auth_key.to_string())
}

#[test]
fn health_returns_200() {
    let (mut child, web_addr, _ws_addr, _auth_key) = spawn_backend();

    let url = format!("http://{}/", web_addr);
    let resp = reqwest::blocking::get(&url).expect("health request failed");
    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().unwrap();
    assert_eq!(body["status"], "ok");

    // Clean up
    drop(child.stdin.take());
    let _ = child.kill();
}

#[test]
fn auth_rejects_missing_key() {
    let (mut child, web_addr, _ws_addr, _auth_key) = spawn_backend();

    let client = reqwest::blocking::Client::new();
    let resp = client
        .get(format!("http://{}/wave/service", web_addr))
        .send()
        .expect("request failed");
    assert_eq!(resp.status(), 401);

    drop(child.stdin.take());
    let _ = child.kill();
}

#[test]
fn auth_accepts_valid_header() {
    let (mut child, web_addr, _ws_addr, auth_key) = spawn_backend();

    let client = reqwest::blocking::Client::new();
    let resp = client
        .post(format!("http://{}/wave/service", web_addr))
        .header("X-AuthKey", &auth_key)
        .header("Content-Type", "application/json")
        .body(r#"{"service":"client","method":"GetClientData"}"#)
        .send()
        .expect("request failed");
    assert_eq!(resp.status(), 200); // real handler returns 200

    let body: serde_json::Value = resp.json().unwrap();
    assert!(body["success"].as_bool().unwrap_or(false));

    drop(child.stdin.take());
    let _ = child.kill();
}

#[test]
fn sigterm_exits_process() {
    let (mut child, _web_addr, _ws_addr, _auth_key) = spawn_backend();

    // Send SIGTERM (matching Go's graceful shutdown)
    #[cfg(unix)]
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }

    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => return,
            Ok(None) => {
                if start.elapsed() > std::time::Duration::from_secs(5) {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("child did not exit within 5s after SIGTERM");
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Err(e) => panic!("try_wait error: {}", e),
        }
    }
}
