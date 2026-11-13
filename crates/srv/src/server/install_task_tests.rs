// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What an npm install of an agent CLI does from `install.start` to its last event,
//! run against a fake `npm` on `PATH`: the lines it publishes, the terminal `done`,
//! the files it leaves, and that the session and the provider are released.
//!
//! Written before `spawn_install_task` was split into named steps, to hold its
//! behaviour still. Unix only: the fake `npm` is a shell script.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use agentmux_common::{DataPaths, RuntimeMode};

use super::install_handlers::InstallSessionRegistry;
use super::install_task::spawn_install_task;
use crate::backend::cli_install;
use crate::backend::providers::ProviderConfig;
use crate::test_support::{broker_recording, RecordingWpsClient, ISOLATED_AUTH_ENV_LOCK};

/// One run: what is on disk and on `PATH` before it, and what interrupts it.
#[derive(Default)]
struct Scenario {
    /// The fake `npm`'s steps after it reads `--prefix` into `$prefix`; `None`: no `npm` on `PATH`.
    npm: Option<&'static str>,
    /// A finished install of the provider is already in the shared dir.
    finished_install: bool,
    /// A half-written install (a stray file, no shim, no marker) is in the shared dir.
    leftover: bool,
    /// The launcher's env vars are not set.
    no_data_env: bool,
    /// Another instance holds the install lock for the whole run.
    lock_held: bool,
    /// Send `install.cancel` once an event contains this text.
    cancel_on: Option<&'static str>,
    /// Send `install.cancel` this long after the start.
    cancel_after_ms: Option<u64>,
}

struct Outcome {
    /// `line:<stream>:<text>` and `done:<ok>:<error as JSON>`, in the order published.
    events: Vec<String>,
    scopes_ok: bool,
    provider: &'static ProviderConfig,
    pin: String,
    shared_dir: PathBuf,
    /// The provider could be claimed again afterwards.
    released: bool,
    /// The session was no longer in the registry afterwards.
    session_gone: bool,
    tmp: tempfile::TempDir,
}

/// Puts the process env back as the test found it.
struct EnvRestore(Vec<(String, Option<std::ffi::OsString>)>);

impl EnvRestore {
    fn set(keys: &[String]) -> Self {
        Self(keys.iter().map(|k| (k.clone(), std::env::var_os(k))).collect())
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        for (k, v) in &self.0 {
            match v {
                Some(v) => std::env::set_var(k, v),
                None => std::env::remove_var(k),
            }
        }
    }
}

fn npm_provider() -> &'static ProviderConfig {
    crate::backend::providers::all_providers()
        .filter(|p| !p.npm_package.is_empty() && !p.pinned_version.is_empty())
        .filter(|p| !crate::backend::cli_download::is_managed_install(p.id))
        .min_by_key(|p| p.id)
        .expect("a provider installed from npm")
}

fn data_paths(root: &Path) -> DataPaths {
    let instance = root.join("channels").join("c");
    let version = instance.join("versions").join("1.0.0");
    DataPaths {
        home_dir: version.join("data"),
        instance_dir: instance.clone(),
        channel: "c".to_string(),
        data_dir: version.join("data"),
        config_dir: instance.join("config"),
        logs_dir: version.join("logs"),
        cef_cache_dir: version.join("cef-cache"),
        agents_dir: instance.join("agents"),
        instance_runtime_dir: version.join("runtime"),
        shared_dir: root.join("shared"),
        mode: RuntimeMode::Portable,
    }
}

fn write_fake_npm(bin_dir: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(bin_dir).unwrap();
    let script = format!(
        "#!/bin/sh\nprefix=\"\"\nwhile [ $# -gt 0 ]; do\n  if [ \"$1\" = \"--prefix\" ]; then prefix=\"$2\"; fi\n  shift\ndone\n{body}\n"
    );
    let path = bin_dir.join("npm");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn touch_shim(dir: &Path, cli_command: &str) {
    let bin = dir.join("node_modules").join(".bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join(cli_command), "#!/bin/sh\n").unwrap();
}

fn summarize(client: &RecordingWpsClient) -> Vec<String> {
    client
        .received_events()
        .into_iter()
        .map(|(_, e)| {
            let d = e.data.unwrap_or_default();
            if d["op"] == "done" {
                format!("done:{}:{}", d["ok"], d["error"])
            } else {
                format!("line:{}:{}", d["stream"].as_str().unwrap(), d["line"].as_str().unwrap())
            }
        })
        .collect()
}

fn run(s: Scenario) -> Outcome {
    let _env_lock = ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::tempdir().unwrap();
    let paths = data_paths(tmp.path());
    let vars = paths.to_env_vars();
    let mut keys: Vec<String> = vars.iter().map(|(k, _)| k.to_string()).collect();
    keys.extend(["AGENTMUX_CLONE_ID".to_string(), "PATH".to_string()]);
    let _restore = EnvRestore::set(&keys);
    std::env::remove_var("AGENTMUX_CLONE_ID");
    for (k, v) in &vars {
        if s.no_data_env {
            std::env::remove_var(k);
        } else {
            std::env::set_var(k, v);
        }
    }
    let bin_dir = tmp.path().join("fake-bin");
    match s.npm {
        Some(body) => {
            write_fake_npm(&bin_dir, body);
            let old = std::env::var_os("PATH").unwrap_or_default();
            let joined = std::env::join_paths(std::iter::once(bin_dir.clone()).chain(std::env::split_paths(&old))).unwrap();
            std::env::set_var("PATH", joined);
        }
        None => {
            std::fs::create_dir_all(&bin_dir).unwrap();
            std::env::set_var("PATH", &bin_dir);
        }
    }

    let provider = npm_provider();
    let pin = provider.pinned_version.to_string();
    let shared_dir = cli_install::install_dir(&paths, provider.id, &pin);
    if s.finished_install {
        touch_shim(&shared_dir, provider.cli_command);
        cli_install::mark_complete(&shared_dir, &pin).unwrap();
    }
    if s.leftover {
        std::fs::create_dir_all(&shared_dir).unwrap();
        std::fs::write(shared_dir.join("stray"), "half-written").unwrap();
    }
    let _lock = s.lock_held.then(|| cli_install::try_lock_install(&paths, provider.id, &pin).unwrap().expect("the lock is free"));

    let (broker, client) = broker_recording("install_chunk");
    let broker = Arc::new(broker);
    let registry = InstallSessionRegistry::new();
    let session = "install-test-session".to_string();

    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let (released, session_gone) = rt.block_on(async {
        assert!(registry.try_claim_provider(provider.id));
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        registry.insert(session.clone(), cancel_tx);
        spawn_install_task(
            broker.clone(),
            registry.clone(),
            session.clone(),
            provider.id.to_string(),
            provider.cli_command.to_string(),
            provider.npm_package.to_string(),
            provider.pinned_version.to_string(),
            cancel_rx,
        );
        let started = std::time::Instant::now();
        let mut cancelled = false;
        loop {
            let events = summarize(&client);
            let due = s.cancel_on.is_some_and(|n| events.iter().any(|e| e.contains(n)))
                || s.cancel_after_ms.is_some_and(|ms| started.elapsed().as_millis() as u64 >= ms);
            if due && !cancelled {
                cancelled = true;
                assert!(registry.cancel(&session));
            }
            if events.iter().any(|e| e.starts_with("done:")) {
                break;
            }
            assert!(started.elapsed().as_secs() < 30, "no done event: {events:?}");
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        // The session and the provider are released right after the last event.
        let mut released = false;
        for _ in 0..250 {
            if registry.try_claim_provider(provider.id) {
                released = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        (released, !registry.cancel(&session))
    });

    let scope = format!("install:{session}");
    let scopes_ok = client.received_events().iter().all(|(_, e)| e.scopes == vec![scope.clone()]);
    Outcome { events: summarize(&client), scopes_ok, provider, pin, shared_dir, released, session_gone, tmp }
}

fn last(o: &Outcome) -> &str {
    o.events.last().expect("some event")
}

#[test]
fn a_successful_install_runs_npm_in_the_shared_dir_and_marks_it_complete() {
    let o = run(Scenario {
        npm: Some(
            "echo 'added 1 package'\necho 'npm warn deprecated' >&2\nmkdir -p \"$prefix/node_modules/.bin\"\nprintf '#!/bin/sh\\n' > \"$prefix/node_modules/.bin/$CLI\"\n"
                .replace("$CLI", npm_provider().cli_command)
                .leak(),
        ),
        ..Default::default()
    });
    assert!(o.scopes_ok, "every event is scoped to the session");
    let first = &o.events[0];
    assert!(first.starts_with("line:stdout:$ npm install "), "{first}");
    assert!(first.contains(o.provider.npm_package), "{first}");
    assert!(first.contains(&format!("--prefix {}", o.shared_dir.display())), "{first}");
    for flag in ["--no-audit", "--no-fund", "--progress=false", "--loglevel=verbose", "--legacy-peer-deps=false", "--registry="] {
        assert!(first.contains(flag), "{flag} in {first}");
    }
    assert!(o.events.contains(&"line:stdout:added 1 package".to_string()), "{:?}", o.events);
    assert!(o.events.contains(&"line:stderr:npm warn deprecated".to_string()), "{:?}", o.events);
    assert_eq!(last(&o), "done:true:null");
    assert_eq!(o.events.iter().filter(|e| e.starts_with("done:")).count(), 1);
    assert_eq!(std::fs::read_to_string(o.shared_dir.join(cli_install::COMPLETE_MARKER)).unwrap(), o.pin);
    assert!(o.shared_dir.join(".agentmux-empty-user.npmrc").is_file(), "npm ran with an empty user config");
    assert!(o.released && o.session_gone);
}

#[test]
fn a_finished_install_by_another_instance_is_reused_without_running_npm() {
    let sentinel = "touch \"$prefix/ran-npm\"";
    let o = run(Scenario { npm: Some(sentinel), finished_install: true, ..Default::default() });
    assert_eq!(o.events, vec!["line:stdout:already installed by another AgentMux instance".to_string(), "done:true:null".to_string()]);
    assert!(!o.shared_dir.join("ran-npm").exists());
    assert!(o.scopes_ok && o.released && o.session_gone);
}

#[test]
fn an_incomplete_install_is_cleared_before_npm_runs() {
    let body = format!(
        "if [ -e \"$prefix/stray\" ]; then echo 'stray file still there' >&2; exit 9; fi\nmkdir -p \"$prefix/node_modules/.bin\"\nprintf '#!/bin/sh\\n' > \"$prefix/node_modules/.bin/{}\"\n",
        npm_provider().cli_command
    );
    let o = run(Scenario { npm: Some(Box::leak(body.into_boxed_str())), leftover: true, ..Default::default() });
    assert_eq!(last(&o), "done:true:null", "{:?}", o.events);
    assert!(!o.shared_dir.join("stray").exists());
    assert!(o.released && o.session_gone);
}

#[test]
fn a_failing_npm_ends_the_install_with_its_exit_code_and_leaves_no_marker() {
    let o = run(Scenario { npm: Some("echo 'npm error code E404' >&2\nexit 3"), ..Default::default() });
    assert!(o.events.contains(&"line:stderr:npm error code E404".to_string()), "{:?}", o.events);
    assert_eq!(last(&o), "done:false:\"npm exited Some(3)\"");
    assert!(!o.shared_dir.join(cli_install::COMPLETE_MARKER).exists());
    assert!(o.scopes_ok && o.released && o.session_gone);
}

#[test]
fn an_npm_that_succeeds_without_the_cli_shim_is_a_failed_install() {
    let o = run(Scenario { npm: Some("echo 'added 0 packages'"), ..Default::default() });
    let done = last(&o);
    let want = format!("npm install reported success but {} not found in {}/node_modules/.bin/", o.provider.cli_command, o.shared_dir.display());
    assert_eq!(done, format!("done:false:{}", serde_json::json!(want)));
    assert!(!o.shared_dir.join(cli_install::COMPLETE_MARKER).exists());
    assert!(o.released && o.session_gone);
}

#[test]
fn no_npm_on_the_path_fails_at_the_spawn() {
    let o = run(Scenario::default());
    assert!(o.events[0].starts_with("line:stdout:$ npm install "), "{:?}", o.events);
    assert!(last(&o).starts_with("done:false:\"spawn npm: "), "{}", last(&o));
    assert!(o.released && o.session_gone);
}

#[test]
fn cancelling_a_running_install_kills_npm_and_removes_the_partial_dir() {
    let body = "mkdir -p \"$prefix/node_modules\"\ntouch \"$prefix/node_modules/partial\"\necho started\nexec sleep 60";
    let o = run(Scenario { npm: Some(body), cancel_on: Some("line:stdout:started"), ..Default::default() });
    assert_eq!(last(&o), "done:false:\"cancelled\"");
    assert!(!o.shared_dir.exists(), "the partial install is removed");
    assert!(o.scopes_ok && o.released && o.session_gone);
}

#[test]
fn cancelling_while_another_instance_holds_the_lock_ends_it_without_running_npm() {
    let o = run(Scenario { npm: Some("touch \"$prefix/ran-npm\""), lock_held: true, cancel_after_ms: Some(400), ..Default::default() });
    assert_eq!(o.events, vec!["done:false:\"cancelled\"".to_string()]);
    assert!(!o.shared_dir.join("ran-npm").exists());
    assert!(o.scopes_ok && o.released && o.session_gone);
}

#[test]
fn without_the_launchers_env_there_is_no_dir_to_install_into() {
    let o = run(Scenario { npm: Some("exit 0"), no_data_env: true, ..Default::default() });
    assert_eq!(o.events, vec!["done:false:\"cannot determine home directory\"".to_string()]);
    assert!(o.scopes_ok && o.released && o.session_gone);
    drop(o.tmp);
}
