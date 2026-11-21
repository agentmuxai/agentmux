// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use crate::data_env_tests::DataEnv;

#[test]
fn agy_probes_run_without_the_clis_own_update() {
    assert!(super::is_agy("/home/x/.local/bin/agy"));
    assert!(super::is_agy("/home/x/.local/bin/AGY"));
}

#[cfg(windows)]
#[test]
fn agy_exe_on_windows_is_found_by_its_name() {
    assert!(super::is_agy("C:\\Users\\x\\AppData\\Local\\agy\\bin\\agy.exe"));
}

#[test]
fn other_clis_are_left_alone() {
    assert!(!super::is_agy("/usr/bin/claude"));
    assert!(!super::is_agy("/opt/tools/agyx"));
}


/// The strict order: a copy on PATH is never "found" (a launch would not run
/// it), for an npm provider (claude) or any other (goose), unless the person
/// listed the provider in `cli:usepath`; then it is, as their own copy.
#[tokio::test]
async fn locate_provider_cli_uses_a_copy_on_path_only_when_the_person_asked() {
    let _guard = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // No managed installs: an empty AgentMux home.
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("AGENTMUX_HOME_OVERRIDE", home.path());
    // `claude` (npm-managed) and `goose` (no npm package; AgentMux's own copy is absent here) both on PATH, first.
    let bin = tempfile::tempdir().unwrap();
    for cli in ["claude", "goose"] {
        let file = if cfg!(windows) { bin.path().join(format!("{cli}.cmd")) } else { bin.path().join(cli) };
        std::fs::write(&file, if cfg!(windows) { "@echo off\r\n" } else { "#!/bin/sh\n" }).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs = vec![bin.path().to_path_buf()];
    dirs.extend(std::env::split_paths(&old_path));
    std::env::set_var("PATH", std::env::join_paths(dirs).unwrap());

    let claude_off = super::locate_provider_cli_with("claude", false).await;
    let goose_off = super::locate_provider_cli_with("goose", false).await;
    let claude_on = super::locate_provider_cli_with("claude", true).await;
    let goose = super::locate_provider_cli_with("goose", true).await;
    let unknown = super::locate_provider_cli_with("no-such-provider", true).await;

    std::env::set_var("PATH", old_path);
    std::env::remove_var("AGENTMUX_HOME_OVERRIDE");

    assert_eq!((claude_off, goose_off), (None, None), "a copy on PATH is not the one a launch runs");
    assert_eq!(claude_on.expect("listed: claude on PATH is found").source, super::CliSource::SystemPath);
    let goose = goose.expect("listed: goose on PATH is found");
    assert_eq!(goose.source, super::CliSource::SystemPath);
    // The same folder, whichever way each side spells it. `where` returns the long form of a
    // path even when PATH holds the 8.3 short one (GitHub's Windows runners put TEMP under
    // `C:\Users\RUNNER~1`), so comparing the strings fails the nightly while both name one file.
    let found = std::fs::canonicalize(std::path::Path::new(&goose.path).parent().unwrap()).unwrap();
    assert_eq!(found, std::fs::canonicalize(bin.path()).unwrap(), "{}", goose.path);
    assert_eq!(unknown, None);
}

/// `where` lists every match; a `.cmd` shim first must not hide the
/// `.exe` after it, which the prompt-argument branch then picks.
#[test]
fn every_spawnable_match_is_kept_in_path_order() {
    let out = [r"C:\Users\u\bin\agy", r"C:\Users\u\bin\agy.cmd", r"D:\tools\agy.exe", ""].join("\r\n");
    assert_eq!(
        super::spawnable_candidates(&out, true),
        vec![r"C:\Users\u\bin\agy.cmd".to_string(), r"D:\tools\agy.exe".to_string()]
    );
    let first_real = super::spawnable_candidates(&out, true)
        .into_iter()
        .find(|p| !super::is_script_wrapper(p));
    assert_eq!(first_real.as_deref(), Some(r"D:\tools\agy.exe"));
    assert_eq!(super::spawnable_candidates("/usr/bin/agy\n", false), vec!["/usr/bin/agy".to_string()]);
}

#[test]
fn pi_is_signed_in_only_when_it_lists_models() {
    let none = "No models available. Use /login to log into a provider via OAuth or API key.";
    assert!(!super::pi_lists_models(true, none, ""));
    assert!(!super::pi_lists_models(true, "", none));
    assert!(!super::pi_lists_models(false, "", ""));
    assert!(super::pi_lists_models(true, "anthropic/claude-sonnet-4-6
", ""));
}

#[tokio::test]
async fn pi_acp_without_its_pi_is_refused_not_read_as_signed_in() {
    let dir = tempfile::tempdir().unwrap();
    let acp = dir.path().join(if cfg!(windows) { "pi-acp.cmd" } else { "pi-acp" });
    std::fs::write(&acp, "").unwrap();
    let args = vec!["--list-models".to_string()];
    let err = super::run_auth_check(&acp.to_string_lossy(), &args, &Default::default()).await.unwrap_err();
    assert!(err.contains("not installed beside pi-acp"), "{err}");
}

#[tokio::test]
async fn an_empty_check_command_is_refused_not_read_as_signed_in() {
    let err = super::run_auth_check("pi-acp", &[], &Default::default()).await.unwrap_err();
    assert!(err.contains("no sign-in check"), "{err}");
}

/// A prompt passed through one of these is re-parsed by `cmd.exe`.
#[test]
fn batch_scripts_are_script_wrappers_and_executables_are_not() {
    for p in [r"C:\Users\u\bin\agy.cmd", r"C:\x\AGY.CMD", r"C:\x\run.bat"] {
        assert!(super::is_script_wrapper(p), "{p}");
    }
    for p in [r"C:\Users\u\AppData\Local\agy\bin\agy.exe", "/home/u/.local/bin/agy", r"C:\x\cmd\agy.exe"] {
        assert!(!super::is_script_wrapper(p), "{p}");
    }
}
use super::*;

fn resolve_cmd(block_id: &str, notice_block_id: Option<&str>) -> CommandResolveCliData {
    CommandResolveCliData {
        provider_id: "qwen".to_string(),
        cli_command: "qwen".to_string(),
        npm_package: "@qwen-code/qwen-code".to_string(),
        pinned_version: "0.24.0".to_string(),
        windows_install_command: String::new(),
        unix_install_command: String::new(),
        block_id: block_id.to_string(),
        notice_block_id: notice_block_id.map(str::to_string),
        agent_id: None,
    }
}

/// CLI notices go to the pane being launched: a quick fork resolves from
/// its source pane (`block_id`) but names its new pane (Codex on #4128).
#[test]
fn cli_notices_go_to_the_launch_target() {
    assert_eq!(notice_block_id(&resolve_cmd("source", Some("fork"))), "fork");
    assert_eq!(notice_block_id(&resolve_cmd("source", None)), "source");
    assert_eq!(notice_block_id(&resolve_cmd("source", Some("  "))), "source");
}

/// An unknown provider is refused before anything touches the disk.
#[tokio::test]
async fn ensure_provider_auth_dir_rejects_an_unknown_provider() {
    let state = crate::server::tests::test_state();
    let (engine, mut out) = WshRpcEngine::new();
    register_cli_handlers(&engine, &state);
    engine.handle_message(crate::backend::rpc_types::RpcMessage {
        command: COMMAND_ENSURE_PROVIDER_AUTH_DIR.to_string(),
        reqid: "r1".to_string(),
        data: Some(serde_json::json!({ "provider_id": "no-such-provider" })),
        ..Default::default()
    });
    let resp = tokio::time::timeout(std::time::Duration::from_secs(2), out.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(resp.data.is_none(), "{resp:?}");
    assert!(resp.error.contains("unknown provider"), "{resp:?}");
}

/// `resolvecli` installs the registry's package for an id; an id outside the
/// registry that names a package is refused, not installed.
#[tokio::test]
async fn resolvecli_does_not_install_a_package_for_an_unknown_provider() {
    let state = crate::server::tests::test_state();
    let (engine, mut out) = WshRpcEngine::new();
    register_cli_handlers(&engine, &state);
    let mut cmd = resolve_cmd("b1", None);
    cmd.provider_id = "no-such-agent".to_string();
    cmd.cli_command = "no-such-agent".to_string();
    cmd.npm_package = "left-pad".to_string();
    engine.handle_message(crate::backend::rpc_types::RpcMessage {
        command: COMMAND_RESOLVE_CLI.to_string(),
        reqid: "r1".to_string(),
        data: Some(serde_json::to_value(&cmd).unwrap()),
        ..Default::default()
    });
    let resp = tokio::time::timeout(std::time::Duration::from_secs(5), out.recv()).await.unwrap().unwrap();
    assert!(resp.data.is_none(), "{resp:?}");
    assert!(resp.error.contains("not a known agent CLI"), "{resp:?}");
}

/// All seven cli/toolchain/widget commands record their request and
/// response types.
///
/// The two widget commands are the interesting entries. They read their
/// payload field-by-field off a `serde_json::Value` and turn a bad port or
/// path into an `ok: false` ANSWER rather than an error, so they cannot
/// take a struct request. Earlier slices concluded such handlers had to
/// stay on `register_handler` entirely -- but `Value` deserializes from
/// anything, so registering it AS the request type is exactly as tolerant
/// while still recording the response. That is what this asserts.
#[tokio::test]
async fn register_typed_records_every_cli_command() {
    let state = crate::server::tests::test_state();
    let (engine, _rx) = WshRpcEngine::new();
    register_cli_handlers(&engine, &state);
    let schema = engine.schema_json();
    let rows = schema.as_array().unwrap();
    let find = |cmd: &str| {
        rows.iter()
            .find(|r| r["command"] == cmd)
            .unwrap_or_else(|| panic!("{cmd} missing from the schema"))
    };

    for (cmd, req, resp) in [
        (COMMAND_RESOLVE_CLI, "CommandResolveCliData", "ResolveCliResult"),
        (COMMAND_CHECK_CLI_AUTH, "CommandCheckCliAuthData", "CheckCliAuthResult"),
        ("runclilogin", "CommandRunCliLoginData", "RunCliLoginResult"),
        (COMMAND_ENSURE_PROVIDER_AUTH_DIR, "EnsureProviderAuthDirReq", "EnsureProviderAuthDirResult"),
        ("toolchain.versions", "ToolchainVersionsReq", "HashMap<alloc::string::String, core::option::Option<alloc::string::String>>"),
    ] {
        let row = find(cmd);
        assert_eq!(row["requestName"], req, "{cmd} request");
        assert!(
            row["responseName"].as_str().unwrap().contains(resp.split('<').next().unwrap()),
            "{cmd} response should mention {resp}, got {:?}",
            row["responseName"],
        );
    }

    // The widget pair: `Value` request, named response.
    for cmd in ["widget.health", "widget.api"] {
        let row = find(cmd);
        assert_eq!(
            row["requestName"], "Value",
            "{cmd} keeps a Value request so a bad port stays an answer, not an error",
        );
    }
    assert_eq!(find("widget.health")["responseName"], "WidgetHealthResult");
    assert_eq!(find("widget.api")["responseName"], "WidgetApiResult");

    // `toolchain.env` takes no argument, so `Option<_>` of an empty struct.
    let env = find("toolchain.env");
    let env_req = env["requestName"].as_str().unwrap();
    assert!(
        env_req.starts_with("core::option::Option<") && env_req.contains("ToolchainEnvReq"),
        "toolchain.env should take Option<ToolchainEnvReq>, got {env_req:?}",
    );
    assert_eq!(env["responseName"], "ToolchainEnvResult");
}

/// A `Value` request really does accept everything, which is the whole
/// reason it is safe to use here. If this ever stops holding, the two
/// widget commands silently start rejecting payloads they used to answer.
#[test]
fn a_value_request_deserializes_from_any_payload() {
    for payload in [
        serde_json::json!({ "port": 3000 }),
        serde_json::json!({ "port": "not a number" }),
        serde_json::json!({}),
        serde_json::json!([1, 2, 3]),
        serde_json::Value::Null,
    ] {
        serde_json::from_value::<serde_json::Value>(payload.clone())
            .unwrap_or_else(|e| panic!("{payload} should deserialize as Value, got {e}"));
    }
}

/// `toolchain.env`'s `pathSource` is camelCase on the wire while every
/// other field in that response is lowercase, so it needs BOTH
/// `serde(rename)` and `ts(rename)` -- ts-rs does not read the serde one.
/// Without the second the generated key is `path_source`, which nothing
/// sends and nothing reads, and every gate still passes.
#[test]
fn toolchain_env_reports_path_source_as_camel_case() {
    let v = serde_json::to_value(ToolchainEnvResult {
        path: "/usr/bin".to_string(),
        path_source: "inherited".to_string(),
        os: "linux".to_string(),
        arch: "x86_64".to_string(),
    })
    .unwrap();
    assert_eq!(v["pathSource"], "inherited");
    assert!(v.get("path_source").is_none(), "the Rust field name must not reach the wire: {v}");

    // Match the DECLARATION, not any mention: the doc comment on this
    // field names `path_source` on purpose (explaining why the rename
    // exists), and a bare substring check matched that instead of the
    // field and failed on a correct binding.
    let ts = <ToolchainEnvResult as ts_rs::TS>::inline(&ts_rs::Config::from_env());
    assert!(ts.contains("pathSource: string"), "generated TS lost the rename: {ts}");
    assert!(
        !ts.contains("path_source: string"),
        "generated TS kept the Rust name as a field: {ts}",
    );
}

// ---- Characterisation tests for the handlers behind `register_cli_handlers` ----
// They pin the answers each command gives today, through the engine, so the
// handlers can be moved out of the register function without a change.

async fn call(command: &str, data: serde_json::Value) -> crate::backend::rpc_types::RpcMessage {
    let state = crate::server::tests::test_state();
    let (engine, mut out) = WshRpcEngine::new();
    register_cli_handlers(&engine, &state);
    engine.handle_message(crate::backend::rpc_types::RpcMessage {
        command: command.to_string(),
        reqid: "r1".to_string(),
        data: Some(data),
        ..Default::default()
    });
    tokio::time::timeout(std::time::Duration::from_secs(20), out.recv())
        .await
        .expect("the command answers")
        .expect("the engine is open")
}

fn answer<T: serde::de::DeserializeOwned>(resp: &crate::backend::rpc_types::RpcMessage) -> T {
    assert!(resp.error.is_empty(), "{resp:?}");
    serde_json::from_value(resp.data.clone().expect("data")).expect("typed answer")
}

/// A one-shot HTTP server on 127.0.0.1: answers the first request with `reply`
/// and hands back the request it read.
async fn serve_once(reply: &'static str) -> (u16, tokio::sync::oneshot::Receiver<String>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 8192];
        let mut got = Vec::new();
        loop {
            let n = sock.read(&mut buf).await.unwrap();
            got.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&got).to_string();
            if n == 0 || text.find("\r\n\r\n").is_some_and(|h| {
                let len = text
                    .lines()
                    .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                    .unwrap_or(0);
                text.len() >= h + 4 + len
            }) {
                break;
            }
        }
        sock.write_all(reply.as_bytes()).await.unwrap();
        let _ = sock.shutdown().await;
        let _ = tx.send(String::from_utf8_lossy(&got).to_string());
    });
    (port, rx)
}

/// A port nothing listens on.
async fn closed_port() -> u16 {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    l.local_addr().unwrap().port()
}

#[tokio::test]
async fn toolchain_env_reports_the_process_path_and_platform() {
    let _guard = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let resp = call("toolchain.env", serde_json::Value::Null).await;
    let env: ToolchainEnvResult = answer(&resp);
    assert_eq!(env.path, std::env::var("PATH").unwrap_or_default());
    assert_eq!(env.os, std::env::consts::OS);
    assert_eq!(env.arch, std::env::consts::ARCH);
    assert!(!env.path_source.is_empty());
}

/// The login flow lives in the CEF host; this command answers empty and spawns nothing.
#[tokio::test]
async fn runclilogin_answers_empty_and_starts_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("ran");
    let script = dir.path().join("login.sh");
    std::fs::write(&script, format!("#!/bin/sh\ntouch {}\n", marker.display())).unwrap();
    let resp = call(
        "runclilogin",
        serde_json::json!({ "cli_path": script.to_string_lossy(), "login_args": ["login"] }),
    )
    .await;
    let r: RunCliLoginResult = answer(&resp);
    assert_eq!((r.auth_url, r.raw_output), (None, String::new()));
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(!marker.exists(), "the srv side must not run the login CLI");
}

#[tokio::test]
async fn toolchain_versions_with_no_packages_answers_an_empty_map() {
    let resp = call("toolchain.versions", serde_json::json!({ "packages": [] })).await;
    let map: std::collections::HashMap<String, Option<String>> = answer(&resp);
    assert!(map.is_empty());
}

#[tokio::test]
async fn toolchain_prune_is_a_dry_run_unless_told_otherwise() {
    let _guard = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = DataEnv::new();
    let none = call("toolchain.prune", serde_json::Value::Null).await;
    let explicit = call("toolchain.prune", serde_json::json!({ "dry_run": true })).await;
    let real = call("toolchain.prune", serde_json::json!({ "dry_run": false })).await;
    let (none, explicit, real): (ToolchainPruneResult, ToolchainPruneResult, ToolchainPruneResult) =
        (answer(&none), answer(&explicit), answer(&real));
    assert!(none.dry_run && explicit.dry_run, "no argument is a dry run");
    assert!(!real.dry_run);
    assert!(none.candidates.is_empty() && none.removed.is_empty());
    assert_eq!(none.max_idle_days, 30);
}

#[tokio::test]
async fn widget_health_rejects_a_bad_port_without_an_error() {
    for payload in [
        serde_json::json!({}),
        serde_json::json!({ "port": 0 }),
        serde_json::json!({ "port": 70000 }),
        serde_json::json!({ "port": "3000" }),
        serde_json::json!([1, 2]),
    ] {
        let resp = call("widget.health", payload.clone()).await;
        let r: WidgetHealthResult = answer(&resp);
        assert_eq!((r.healthy, r.status_code), (false, None), "{payload}");
    }
}

#[tokio::test]
async fn widget_health_reads_the_status_and_optionally_the_body() {
    let ok = "HTTP/1.1 200 OK\r\nContent-Length: 13\r\nConnection: close\r\n\r\nhello grafana";
    let (port, req) = serve_once(ok).await;
    let r: WidgetHealthResult =
        answer(&call("widget.health", serde_json::json!({ "port": port, "health_check_path": "/api/health" })).await);
    assert_eq!((r.healthy, r.status_code), (true, Some(200)));
    assert!(req.await.unwrap().starts_with("GET /api/health HTTP/1.1"));

    let (port, _) = serve_once(ok).await;
    let r: WidgetHealthResult = answer(
        &call("widget.health", serde_json::json!({ "port": port, "health_check_body_contains": "grafana" })).await,
    );
    assert_eq!((r.healthy, r.status_code), (true, Some(200)), "the needle is in the body");

    let (port, _) = serve_once(ok).await;
    let r: WidgetHealthResult = answer(
        &call("widget.health", serde_json::json!({ "port": port, "health_check_body_contains": "flowise" })).await,
    );
    assert_eq!((r.healthy, r.status_code), (false, Some(200)), "another service on the port");

    let down = "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    let (port, _) = serve_once(down).await;
    let r: WidgetHealthResult = answer(&call("widget.health", serde_json::json!({ "port": port })).await);
    assert_eq!((r.healthy, r.status_code), (false, Some(503)));

    let r: WidgetHealthResult = answer(&call("widget.health", serde_json::json!({ "port": closed_port().await })).await);
    assert_eq!((r.healthy, r.status_code), (false, None), "refused is an answer, not an error");
}

#[tokio::test]
async fn widget_api_refuses_a_bad_port_or_a_path_that_could_leave_localhost() {
    let reject = |why: &str, r: WidgetApiResult| {
        assert!(!r.ok && r.status_code.is_none() && r.body.is_none(), "{why}: {r:?}");
        r.error.unwrap_or_default()
    };
    for payload in [serde_json::json!({}), serde_json::json!({ "port": 0 }), serde_json::json!({ "port": 65536 })] {
        let r: WidgetApiResult = answer(&call("widget.api", payload.clone()).await);
        assert_eq!(reject(&payload.to_string(), r), "invalid port");
    }
    for path in ["x", "/a@evil.com", "/a\\b", "//evil.com/x"] {
        let r: WidgetApiResult = answer(&call("widget.api", serde_json::json!({ "port": 3000, "path": path })).await);
        assert_eq!(reject(path, r), "invalid path", "{path}");
    }
}

#[tokio::test]
async fn widget_api_forwards_the_request_and_returns_the_status_and_body() {
    let reply = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 4\r\nConnection: close\r\n\r\nboom";
    let (port, req) = serve_once(reply).await;
    let r: WidgetApiResult = answer(&call("widget.api", serde_json::json!({ "port": port, "path": "/status?x=1" })).await);
    assert_eq!((r.ok, r.status_code, r.body.as_deref(), r.error), (true, Some(500), Some("boom"), None), "a 500 is still ok: true");
    assert!(req.await.unwrap().starts_with("GET /status?x=1 HTTP/1.1"));

    let (port, req) = serve_once("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").await;
    let payload = serde_json::json!({
        "port": port, "path": "/prompt", "method": "post",
        "headers": { "X-Widget": "yes", "X-Number": 7, "bad name": "x" },
        "body": "{\"a\":1}",
    });
    let r: WidgetApiResult = answer(&call("widget.api", payload).await);
    assert_eq!((r.ok, r.status_code, r.body.as_deref()), (true, Some(200), Some("ok")));
    let sent = req.await.unwrap().to_ascii_lowercase();
    assert!(sent.starts_with("post /prompt http/1.1"), "{sent}");
    assert!(sent.contains("x-widget: yes"), "{sent}");
    assert!(!sent.contains("x-number") && !sent.contains("bad name"), "only valid string headers are sent: {sent}");
    assert!(sent.contains("content-type: application/json") && sent.ends_with("{\"a\":1}"), "{sent}");

    let r: WidgetApiResult = answer(&call("widget.api", serde_json::json!({ "port": closed_port().await })).await);
    assert!(!r.ok && r.status_code.is_none() && r.body.is_none() && r.error.is_some(), "{r:?}");
}

#[cfg(unix)]
fn script(dir: &std::path::Path, name: &str, body: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.to_string_lossy().into_owned()
}

/// A CLI that is not Claude is asked once and the verdict is its exit status or its JSON.
#[cfg(unix)]
#[tokio::test]
async fn checkcliauth_reads_the_exit_status_or_the_json_of_a_plain_cli() {
    let dir = tempfile::tempdir().unwrap();
    let yes = script(dir.path(), "tool-yes", "echo signed in");
    let no = script(dir.path(), "tool-no", "echo nope >&2; exit 1");
    let json = script(
        dir.path(),
        "tool-json",
        r#"echo '{"loggedIn":true,"emailAddress":"someone@example.com","authMethod":"oauth"}'"#,
    );
    let check = |cli: &str| serde_json::json!({ "cli_path": cli, "auth_check_args": ["status"] });
    let r: CheckCliAuthResult = answer(&call(COMMAND_CHECK_CLI_AUTH, check(&yes)).await);
    assert_eq!((r.authenticated, r.raw_output.trim(), r.email), (true, "signed in", None));
    let r: CheckCliAuthResult = answer(&call(COMMAND_CHECK_CLI_AUTH, check(&no)).await);
    assert_eq!((r.authenticated, r.raw_output.trim()), (false, "nope"), "stderr is the output when stdout is empty");
    let r: CheckCliAuthResult = answer(&call(COMMAND_CHECK_CLI_AUTH, check(&json)).await);
    assert_eq!(
        (r.authenticated, r.email.as_deref(), r.auth_method.as_deref()),
        (true, Some("someone@example.com"), Some("oauth"))
    );
    let resp = call(COMMAND_CHECK_CLI_AUTH, serde_json::json!({ "cli_path": yes, "auth_check_args": [] })).await;
    assert!(resp.error.contains("no sign-in check"), "{resp:?}");
}

/// Claude: no credentials in the provider dir means signed out without running the CLI (not
/// on macOS, where the Keychain holds them); credentials present means the CLI decides.
#[cfg(unix)]
#[tokio::test]
async fn checkcliauth_for_claude_looks_at_the_provider_dir_before_the_cli() {
    let dir = tempfile::tempdir().unwrap();
    let ran = dir.path().join("ran");
    let claude = script(
        dir.path(),
        "claude",
        &format!("touch {}; echo '{{\"loggedIn\":true,\"emailAddress\":\"someone@example.com\"}}'", ran.display()),
    );
    let config = dir.path().join("config");
    std::fs::create_dir(&config).unwrap();
    let check = serde_json::json!({
        "cli_path": claude, "auth_check_args": ["auth", "status", "--json"],
        "auth_env": { "CLAUDE_CONFIG_DIR": config.to_string_lossy() },
    });
    #[cfg(not(target_os = "macos"))]
    {
        let r: CheckCliAuthResult = answer(&call(COMMAND_CHECK_CLI_AUTH, check.clone()).await);
        assert_eq!((r.authenticated, r.raw_output.as_str()), (false, "no credentials found"));
        assert!(!ran.exists(), "no credentials: the CLI is not run");
        std::fs::write(config.join(".credentials.json"), r#"{"claudeAiOauth":{"accessToken":"","refreshToken":""}}"#).unwrap();
        let r: CheckCliAuthResult = answer(&call(COMMAND_CHECK_CLI_AUTH, check.clone()).await);
        assert!(!r.authenticated && !ran.exists(), "empty tokens are no credentials");
    }
    std::fs::write(config.join(".credentials.json"), r#"{"claudeAiOauth":{"refreshToken":"r"}}"#).unwrap();
    let r: CheckCliAuthResult = answer(&call(COMMAND_CHECK_CLI_AUTH, check).await);
    assert_eq!((r.authenticated, r.email.as_deref()), (true, Some("someone@example.com")));
    assert!(ran.exists(), "tokens present: the CLI validates them");
}

/// `resolvecli` for a tool that is not in the provider registry and names no package:
/// its own install dir, else the PATH; neither is a refusal that says how to install it.
#[cfg(unix)]
#[tokio::test]
async fn resolvecli_finds_a_core_tool_on_the_path_or_says_how_to_install_it() {
    let _guard = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = DataEnv::new();
    let bin = tempfile::tempdir().unwrap();
    let tool = script(bin.path(), "coretool-xyz", "echo coretool 1.2.3");
    let old_path = std::env::var_os("PATH").unwrap_or_default();
    let mut dirs = vec![bin.path().to_path_buf()];
    dirs.extend(std::env::split_paths(&old_path));
    std::env::set_var("PATH", std::env::join_paths(dirs).unwrap());

    let mut cmd = resolve_cmd("b1", None);
    cmd.provider_id = "coretool-xyz".to_string();
    cmd.cli_command = "coretool-xyz".to_string();
    cmd.npm_package = String::new();
    cmd.unix_install_command = "brew install coretool-xyz".to_string();
    let found = call(COMMAND_RESOLVE_CLI, serde_json::to_value(&cmd).unwrap()).await;
    cmd.provider_id = "coretool-missing".to_string();
    cmd.cli_command = "coretool-missing".to_string();
    let missing = call(COMMAND_RESOLVE_CLI, serde_json::to_value(&cmd).unwrap()).await;

    std::env::set_var("PATH", old_path);
    let found: ResolveCliResult = answer(&found);
    assert_eq!((found.cli_path.as_str(), found.source.as_str()), (tool.as_str(), "system_path"));
    assert!(found.version.contains("1.2.3"), "{found:?}");
    assert!(missing.data.is_none());
    assert!(missing.error.contains("AMX-CLI-004") && missing.error.contains("brew install coretool-xyz"), "{missing:?}");
}

/// An npm-managed provider that is not installed, asked about without its package, is a
/// look: "not installed", with nothing downloaded.
#[tokio::test]
async fn resolvecli_without_a_package_does_not_install_an_npm_provider() {
    let _guard = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _data = DataEnv::new();
    let mut cmd = resolve_cmd("b1", None);
    cmd.npm_package = String::new();
    let resp = call(COMMAND_RESOLVE_CLI, serde_json::to_value(&cmd).unwrap()).await;
    assert!(resp.data.is_none());
    assert!(resp.error.contains("AMX-CLI-001"), "{resp:?}");
}
