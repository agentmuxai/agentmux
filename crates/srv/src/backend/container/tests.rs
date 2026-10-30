// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Unit and integration tests of the container manager (moved out of `container.rs`, which has a size ceiling).

use super::*;
use {super::pull::pull_error, crate::backend::container_image::DEFAULT_AGENT_IMAGE};

#[test]
fn test_container_name_for_slug() {
    assert_eq!(container_name_for_slug("my-agent"), "agentmux-my-agent");
    assert_eq!(container_name_for_slug("agent1"), "agentmux-agent1");
}

// ── AGENTMUX_LOCAL_URL rewrite for container reachability ───────────────
// See #2939 workstream 1.

#[test]
fn rewrites_loopback_ip_to_host_docker_internal() {
    assert_eq!(
        rewrite_local_url_for_container("http://127.0.0.1:54321"),
        "http://host.docker.internal:54321"
    );
}

#[test]
fn rewrites_localhost_to_host_docker_internal() {
    assert_eq!(
        rewrite_local_url_for_container("http://localhost:54321"),
        "http://host.docker.internal:54321"
    );
}

#[test]
fn leaves_a_non_loopback_host_unchanged() {
    // Already a real hostname/IP, or already host.docker.internal —
    // must pass through unchanged, not get mangled.
    assert_eq!(
        rewrite_local_url_for_container("http://203.0.113.5:54321"),
        "http://203.0.113.5:54321"
    );
    assert_eq!(
        rewrite_local_url_for_container("http://host.docker.internal:54321"),
        "http://host.docker.internal:54321"
    );
}

// ── Credential + workspace mounts ───────────────────────────────────────
// See docs/reports/REPORT_CONTAINER_AGENT_CREDENTIALS_AND_WORKSPACE_2026_09_02.md

fn mount_at<'a>(mounts: &'a [Mount], target: &str) -> Option<&'a Mount> {
    mounts.iter().find(|m| m.target.as_deref() == Some(target))
}

/// THE fix: the bound account's `.credentials.json` is bind-mounted into the
/// container, so the CLI there reads the same token the account login wrote.
/// Without it a container agent authenticates NEVER — `CLAUDE_CONFIG_DIR` is
/// a host path and is stripped by CONTAINER_ENV_DENYLIST before exec.
#[test]
fn a_bound_accounts_credentials_file_is_bind_mounted_into_the_container() {
    let spec = ContainerMountSpec {
        claude_config_host_dir: Some(r"C:\Users\me\.agentmux\identities\acct\claude".into()),
        workspace_host_dir: None, ..Default::default()
    };
    let mounts = agent_home_mounts("agentmux-x", &spec);

    let creds = mount_at(&mounts, "/home/agent/.claude/.credentials.json")
        .expect("credentials must be mounted");
    assert_eq!(creds.typ, Some(MountTypeEnum::BIND));
    assert_eq!(
        creds.source.as_deref(),
        Some("C:/Users/me/.agentmux/identities/acct/claude/.credentials.json"),
        "backslashes are not accepted in a Docker bind source",
    );
    assert_eq!(creds.read_only, Some(false), "an in-place token refresh must reach the host");
}

/// The named volume must REMAIN at ~/.claude. Binding the account's whole
/// config dir there instead looks tidier and is broken: its `projects` entry
/// is a symlink that does not resolve in-container, and a nested volume to
/// shadow it silently fails to mount (both verified live, 2026-09-02).
/// Keeping the volume is what leaves projects/sessions writable by uid 1000.
#[test]
fn the_claude_dir_itself_stays_the_per_agent_named_volume() {
    let spec = ContainerMountSpec {
        claude_config_host_dir: Some("/host/acct/claude".into()),
        workspace_host_dir: None, ..Default::default()
    };
    let mounts = agent_home_mounts("agentmux-x", &spec);

    let claude = mount_at(&mounts, CONTAINER_CLAUDE_DIR).expect("claude dir mounted");
    assert_eq!(
        claude.typ,
        Some(MountTypeEnum::VOLUME),
        "must NOT become a bind of the host config dir — see agent_home_mounts",
    );
    assert_eq!(claude.source.as_deref(), Some("agentmux-claude-agentmux-x"));
    assert!(
        mount_at(&mounts, "/home/agent/.claude/projects").is_none(),
        "a nested projects mount does not work and must not be reintroduced",
    );
}

/// An agent with no bound account keeps exactly the shape that shipped
/// before — this fix adds a mount, it does not rewrite the existing one.
#[test]
fn without_a_bound_account_the_named_volume_shape_is_unchanged() {
    let mounts = agent_home_mounts("agentmux-x", &ContainerMountSpec::default());

    assert_eq!(mounts.len(), 1, "no credential bind");
    let claude = mount_at(&mounts, CONTAINER_CLAUDE_DIR).expect("claude dir mounted");
    assert_eq!(claude.typ, Some(MountTypeEnum::VOLUME));
    assert_eq!(claude.source.as_deref(), Some("agentmux-claude-agentmux-x"));
}

/// An empty config dir is "no account", not a bind of "/.credentials.json".
#[test]
fn an_empty_config_dir_is_treated_as_no_bound_account() {
    let spec = ContainerMountSpec {
        claude_config_host_dir: Some(String::new()),
        workspace_host_dir: None, ..Default::default()
    };
    assert_eq!(agent_home_mounts("agentmux-x", &spec).len(), 1);
}

// ── Drift detection must be bidirectional ───────────────────────────────
// reagent P1 on PR #2933. `mounts_match` compares against these, so the
// desired-set logic is what the tests pin; the Docker inspect half needs a
// live daemon and is covered by the ignored integration test.

/// Unbinding an agent's account must show up as drift. It is a normal
/// transition that never trips SpawnGateError::MissingCredentials, so if the
/// desired set still claimed a credentials mount — or if a presence-only
/// check ignored the now-absent one — the OLD account's token would stay
/// bind-mounted and writable in the running container forever.
#[test]
fn unbinding_an_account_drops_the_credentials_mount_from_the_desired_set() {
    let bound = ContainerMountSpec {
        claude_config_host_dir: Some("/host/acct/claude".into()),
        workspace_host_dir: None, ..Default::default()
    };
    let unbound = ContainerMountSpec::default();
    let creds_target = format!("{CONTAINER_CLAUDE_DIR}/{CLAUDE_CREDENTIALS_FILE}");

    let while_bound = ContainerManager::desired_owned_mounts("agentmux-x", &bound);
    assert_eq!(
        while_bound.get(&creds_target).map(String::as_str),
        Some("/host/acct/claude/.credentials.json"),
    );

    let after_unbind = ContainerManager::desired_owned_mounts("agentmux-x", &unbound);
    assert!(
        !after_unbind.contains_key(&creds_target),
        "an unbound agent must want NO credentials mount — otherwise the old \
         account stays authenticated inside the running container",
    );
    assert!(
        ContainerManager::owned_mount_targets().contains(&creds_target),
        "and that target must be one we compare, or its absence is never noticed",
    );
}

/// Clearing the working directory is the same shape of transition.
#[test]
fn clearing_the_working_directory_drops_the_workspace_from_the_desired_set() {
    let with_ws = ContainerMountSpec {
        claude_config_host_dir: None,
        workspace_host_dir: Some(r"C:\repo".into()), ..Default::default()
    };
    let desired = ContainerManager::desired_owned_mounts("agentmux-x", &with_ws);
    assert_eq!(desired.get(CONTAINER_WORKSPACE_DIR).map(String::as_str), Some("C:/repo"));

    let without = ContainerManager::desired_owned_mounts("agentmux-x", &ContainerMountSpec::default());
    assert!(!without.contains_key(CONTAINER_WORKSPACE_DIR));
}

/// The comparison set must stay scoped to this module's own mounts, so a
/// user's `container_volumes` can never trigger a rebuild of their container.
#[test]
fn only_this_modules_own_mount_targets_are_compared() {
    let owned = ContainerManager::owned_mount_targets();
    assert!(owned.contains(&CONTAINER_CLAUDE_DIR.to_string()));
    assert!(owned.contains(&CONTAINER_WORKSPACE_DIR.to_string()));
    assert!(owned.contains(&format!("{CONTAINER_CLAUDE_DIR}/{CLAUDE_CREDENTIALS_FILE}")));
    assert!(owned.contains(&crate::identity::github_tokens::CONTAINER_HOOKS_DIR.to_string()));
    assert_eq!(owned.len(), 4, "a user volume target must never appear here");
}

/// macOS keeps OAuth credentials in the Keychain and never writes this file
/// (oauth_probe.rs). Binding a nonexistent source makes Docker refuse to
/// create the container, so an unguarded bind would turn "cannot
/// authenticate on macOS" into "cannot START on macOS".
#[test]
fn a_config_dir_with_no_credentials_file_is_not_offered_for_mounting() {
    let dir = std::env::temp_dir().join("agentmux-creds-guard-empty");
    std::fs::create_dir_all(&dir).unwrap();
    let _ = std::fs::remove_file(dir.join(CLAUDE_CREDENTIALS_FILE));

    assert_eq!(
        credentials_dir_if_file_backed(&dir.to_string_lossy()),
        None,
        "no credentials file (the macOS Keychain case) must not produce a bind",
    );
    assert_eq!(credentials_dir_if_file_backed(""), None);
}

/// …and when the file IS there, it is offered, so the fix still applies on
/// every platform that writes it.
#[test]
fn a_config_dir_holding_a_credentials_file_is_offered_for_mounting() {
    let dir = std::env::temp_dir().join("agentmux-creds-guard-present");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(CLAUDE_CREDENTIALS_FILE), b"{}").unwrap();

    assert_eq!(
        credentials_dir_if_file_backed(&dir.to_string_lossy()).as_deref(),
        Some(dir.to_string_lossy().as_ref()),
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn host_paths_are_normalized_for_the_docker_mount_api() {
    assert_eq!(normalize_host_path(r"C:\Users\me\dir"), "C:/Users/me/dir");
    assert_eq!(normalize_host_path("/already/posix"), "/already/posix");
}

#[test]
fn test_parse_volume_spec_unix_named() {
    let (src, tgt, ro) = parse_volume_spec("myvolume:/data").unwrap();
    assert_eq!(src, "myvolume");
    assert_eq!(tgt, "/data");
    assert!(!ro);
}

#[test]
fn test_parse_volume_spec_unix_bind_readonly() {
    let (src, tgt, ro) = parse_volume_spec("/host/path:/container/path:ro").unwrap();
    assert_eq!(src, "/host/path");
    assert_eq!(tgt, "/container/path");
    assert!(ro);
}

#[test]
fn test_parse_volume_spec_windows_bind() {
    // Drive-letter path must not be split at the drive colon.
    let (src, tgt, ro) = parse_volume_spec("C:\\Users\\me\\repo:/workspace").unwrap();
    assert_eq!(src, "C:\\Users\\me\\repo");
    assert_eq!(tgt, "/workspace");
    assert!(!ro);
}

#[test]
fn test_parse_volume_spec_windows_bind_readonly() {
    let (src, tgt, ro) = parse_volume_spec("C:/Users/me/repo:/workspace:ro").unwrap();
    assert_eq!(src, "C:/Users/me/repo");
    assert_eq!(tgt, "/workspace");
    assert!(ro);
}

#[test]
fn test_parse_volume_spec_malformed() {
    assert!(parse_volume_spec("nocodon").is_none());
}

/// Docker-gated integration test for the container lifecycle + exec path.
/// Requires a reachable Docker daemon (Colima/Docker Desktop), so it is
/// `#[ignore]` by default. Run with:
///   DOCKER_HOST=unix://$HOME/.colima/default/docker.sock \
///     cargo test -p agentmux-srv -- --ignored --nocapture itest_container
///
/// Validates the foundation Phase 2 rests on against a real daemon:
/// `ensure_running` create→reuse, env delivered into the exec via the Docker
/// socket (`CreateExecOptions.env`, NOT argv — the CWE-214 guard), and the
/// stdin→stdout exec I/O contract `spawn_container_turn` depends on.
#[tokio::test]
#[ignore]
async fn itest_container_lifecycle_exec_env_and_io() {
    use tokio::io::AsyncWriteExt as _;

    async fn drain_stdout<S>(mut out: S) -> String
    where
        S: futures_util::Stream<
                Item = Result<bollard::container::LogOutput, bollard::errors::Error>,
            > + Unpin,
    {
        use futures_util::StreamExt as _;
        let mut s = String::new();
        while let Some(item) = out.next().await {
            if let Ok(bollard::container::LogOutput::StdOut { message }) = item {
                s.push_str(&String::from_utf8_lossy(&message));
            }
        }
        s
    }

    let cm = ContainerManager::connect().expect("connect to docker (set DOCKER_HOST)");
    cm.check_available().await.expect("docker daemon must be reachable");

    let name = "agentmux-itest-1357";
    // Public, pullable image with a long-lived default CMD (nginx daemon) and
    // a shell — `create_and_start` sets no cmd, so the image default must keep
    // PID 1 alive between exec turns (the persistent-container model).
    let image = "nginx:alpine";
    let _ = cm.remove(name, true).await; // clean slate; ignore if absent

    // create path: pull + create + start
    cm.ensure_running_observed(name, image, &[], &[], &ContainerMountSpec::default(), &|_| {}).await.expect("ensure_running (create)");
    // reuse path: already running → must no-op, not error or re-create
    cm.ensure_running_observed(name, image, &[], &[], &ContainerMountSpec::default(), &|_| {}).await.expect("ensure_running (reuse)");

    // env reaches the in-container process via the Docker socket, not argv.
    // Drop the (unused) stdin half first: bollard's exec output stream does
    // not EOF while the write-half is held open — the same reason
    // spawn_container_turn drops `input` before reading output.
    let ExecSession { input, output, .. } = cm
        .exec(
            name,
            &["sh".into(), "-c".into(), "printf %s \"$ITEST_KEY\"".into()],
            None,
            &[("ITEST_KEY".into(), "val-42".into())],
            false,
        )
        .await
        .expect("exec (env)");
    drop(input);
    let out = drain_stdout(output).await;
    assert!(out.contains("val-42"), "env must reach container; got {out:?}");

    // stdin → stdout I/O contract: write a newline-terminated message and
    // read it back. Use `read` (completes on the first '\n', then the shell
    // exits) rather than `cat` (which blocks until stdin EOF) — `claude`
    // likewise consumes newline-delimited JSON without waiting for EOF, and
    // bollard's hijacked exec stream does not reliably half-close stdin on
    // `drop(input)`, so an EOF-dependent reader would hang.
    let ExecSession { mut input, output, .. } = cm
        .exec(
            name,
            &["sh".into(), "-c".into(), "read line; printf 'got:%s' \"$line\"".into()],
            None,
            &[],
            true, // this test is specifically the stdin contract
        )
        .await
        .expect("exec (io)");
    input.write_all(b"hello-stdin\n").await.expect("write stdin");
    input.flush().await.expect("flush stdin");
    drop(input);
    let out = drain_stdout(output).await;
    assert!(out.contains("got:hello-stdin"), "stdin must reach the container process; got {out:?}");

    // cleanup
    cm.remove(name, true).await.expect("remove");
}

fn server_error(status_code: u16, message: &str) -> bollard::errors::Error {
    bollard::errors::Error::DockerResponseServerError { status_code, message: message.to_string() }
}

#[test]
fn a_refused_pull_becomes_a_plain_message_not_a_docker_api_error() {
    let err = pull_error(
        "ghcr.io/agentmuxai/agent-claude:latest",
        &server_error(500, "Head \"https://ghcr.io/v2/agentmuxai/agent-claude/manifests/latest\": denied"),
    );
    assert!(matches!(&err, ContainerError::ImagePull { kind: PullFailure::Denied, .. }), "{err:?}");
    let msg = err.user_message();
    assert!(msg.contains("ghcr.io/agentmuxai/agent-claude:latest"), "{msg}");
    assert!(msg.contains("refused access"), "{msg}");
    assert!(!msg.contains("Docker API error") && !msg.contains("manifests/latest"), "{msg}");
}

#[test]
fn a_missing_image_and_an_unreachable_registry_each_get_their_own_message() {
    let missing = pull_error("x/y:1", &server_error(404, "manifest for x/y:1 not found"));
    assert!(missing.user_message().contains("was not found"), "{}", missing.user_message());

    let offline = pull_error("x/y:1", &server_error(500, "Get \"https://registry-1.docker.io/v2/\": dial tcp: lookup registry-1.docker.io: no such host"));
    assert!(offline.user_message().contains("couldn't reach the registry"), "{}", offline.user_message());
}

#[test]
fn the_image_pull_error_keeps_the_daemons_text_for_logs() {
    let err = pull_error("x/y:1", &server_error(500, "denied: some detail"));
    assert!(err.to_string().contains("some detail"), "{err}");
}

#[test]
fn a_stopped_daemon_and_an_install_failure_read_plainly() {
    let down = ContainerError::NotAvailable("connect failed".into()).user_message();
    assert!(down.contains("Docker isn't responding"), "{down}");

    let install = ContainerError::CliInstall { command: "claude".into(), detail: "npm error code ENOTFOUND".into() }.user_message();
    assert!(install.contains("couldn't install claude"), "{install}");
    assert!(install.contains("ENOTFOUND"), "{install}");
    assert!(install.starts_with("Couldn't start the container"), "{install}");
}

#[test]
fn the_cli_directory_is_a_per_agent_named_volume_outside_the_drift_set() {
    let mount = cli_dir_mount("agentmux-abc");
    assert_eq!(mount.target.as_deref(), Some("/home/agent/.agentmux/cli"));
    assert_eq!(mount.source.as_deref(), Some("agentmux-cli-agentmux-abc"));
    assert_eq!(mount.typ, Some(MountTypeEnum::VOLUME));
    assert!(
        !ContainerManager::owned_mount_targets().contains(&CONTAINER_CLI_DIR.to_string()),
        "adding it to the drift set would recreate every existing container"
    );
}

/// Docker-gated: an install that outruns the host-side limit is killed and
/// confirmed gone before `provision_cli` returns, and leaves the volume
/// usable: no marker, no live tree, and a retry installs cleanly. Same setup
/// as the other install test (AGENTMUX_ITEST_BASE_IMAGE, network to npm).
#[tokio::test]
#[ignore]
async fn itest_provision_cli_timeout_stops_the_install_before_returning() {
    let image = std::env::var("AGENTMUX_ITEST_BASE_IMAGE").expect("set AGENTMUX_ITEST_BASE_IMAGE to a built base image");
    let cm = ContainerManager::connect().expect("connect to docker");
    cm.check_available().await.expect("docker daemon must be reachable");

    let name = "agentmux-itest-provision-timeout";
    let _ = cm.remove(name, true).await;
    let _ = cm.inner.docker.remove_volume(&format!("agentmux-cli-{name}"), None).await;
    let _ = cm.inner.docker.remove_volume(&format!("agentmux-claude-{name}"), None).await;
    cm.ensure_running_observed(name, &image, &[], &[], &ContainerMountSpec::default(), &|_| {}).await.expect("ensure_running");
    let install = CliInstall::for_provider("claude", "claude").unwrap();

    // The host gives up after 3 s; the script's own limit is far away, so
    // only the host-side backstop can stop it.
    let tight = InstallLimits {
        run: std::time::Duration::from_secs(3),
        script: std::time::Duration::from_secs(600),
    };
    let err = cm.provision_cli_with(name, &install, tight, || {}).await.unwrap_err();
    let msg = err.user_message();
    assert!(msg.contains("was stopped"), "{msg}");

    // Confirmed gone: nothing from the install is still running.
    let alive = cm
        .exec_collect(
            name,
            &["sh".into(), "-c".into(), "pgrep -f '[n]pm install --prefix' >/dev/null && echo alive || echo gone".into()],
            std::time::Duration::from_secs(30),
        )
        .await
        .expect("pgrep");
    assert!(alive.output.contains("gone"), "{}", alive.output);

    // The volume is not left looking installed.
    let state = cm
        .exec_collect(
            name,
            &["sh".into(), "-c".into(), "ls -A /home/agent/.agentmux/cli | tr '\\n' ' '".into()],
            std::time::Duration::from_secs(30),
        )
        .await
        .expect("ls");
    assert!(!state.output.contains(".installed-") && !state.output.contains("node_modules"), "{}", state.output);

    // And the retry the pane suggests installs cleanly.
    let retry = cm.provision_cli(name, &install, || {}).await.expect("retry installs");
    assert!(matches!(retry, CliProvision::Installed { .. }), "{retry:?}");

    // The script's own limit is the other path: it kills npm itself.
    let _ = cm.remove(name, true).await;
    let _ = cm.inner.docker.remove_volume(&format!("agentmux-cli-{name}"), None).await;
    cm.ensure_running_observed(name, &image, &[], &[], &ContainerMountSpec::default(), &|_| {}).await.expect("recreate");
    let script_limited = InstallLimits {
        run: std::time::Duration::from_secs(300),
        script: std::time::Duration::from_secs(2),
    };
    let err = cm.provision_cli_with(name, &install, script_limited, || {}).await.unwrap_err();
    assert!(err.user_message().contains("took too long"), "{}", err.user_message());

    cm.remove(name, true).await.expect("cleanup");
    let _ = cm.inner.docker.remove_volume(&format!("agentmux-cli-{name}"), None).await;
    let _ = cm.inner.docker.remove_volume(&format!("agentmux-claude-{name}"), None).await;
}

/// Docker-gated: a legacy image the registry refuses falls back to the base
/// image. Needs the base image already present under its published name
/// (`docker tag <built-image> ghcr.io/agentmuxai/agent-base:latest`), so the
/// fallback pull is satisfied locally, and network access so the legacy pull
/// is really refused. Run with:
///   cargo test -p agentmux-srv --bin agentmux-srv -- --ignored --nocapture itest_legacy_image_falls_back
#[tokio::test]
#[ignore]
async fn itest_legacy_image_falls_back_to_the_base_image() {
    let cm = ContainerManager::connect().expect("connect to docker");
    cm.check_available().await.expect("docker daemon must be reachable");
    assert!(cm.has_image_locally(DEFAULT_AGENT_IMAGE).await, "tag a built base image as {DEFAULT_AGENT_IMAGE} first");

    let legacy_but_absent = "ghcr.io/agentmuxai/agent-claude:itest-no-such-tag";
    assert!(!cm.has_image_locally(legacy_but_absent).await);
    let chosen = cm.pull_with_fallback(legacy_but_absent, &|_| {}).await.expect("fallback");
    assert_eq!(chosen, DEFAULT_AGENT_IMAGE);

    // A custom image gets no fallback: its refusal is reported as it is.
    let err = cm.pull_with_fallback("ghcr.io/agentmuxai/agent-claude-not-legacy:itest", &|_| {}).await.unwrap_err();
    assert!(matches!(err, ContainerError::ImagePull { kind: PullFailure::Denied, .. }), "{err:?}");
    println!("{}", err.user_message());
}

/// Docker-gated: installs the pinned CLI into a container made from the real
/// base image, then proves it is idempotent and that a turn-style command finds
/// the binary. Needs a built base image, named by AGENTMUX_ITEST_BASE_IMAGE
/// (`docker build -f docker/Dockerfile.agent-agentmux -t <tag> .`) and network
/// access to npm. Run with:
///   AGENTMUX_ITEST_BASE_IMAGE=<tag> cargo test -p agentmux-srv --bin agentmux-srv -- --ignored --nocapture itest_provision_cli
#[tokio::test]
#[ignore]
async fn itest_provision_cli_installs_once_and_a_turn_can_run_it() {
    let image = std::env::var("AGENTMUX_ITEST_BASE_IMAGE").expect("set AGENTMUX_ITEST_BASE_IMAGE to a built base image");
    let cm = ContainerManager::connect().expect("connect to docker");
    cm.check_available().await.expect("docker daemon must be reachable");

    let name = "agentmux-itest-provision-cli";
    let _ = cm.remove(name, true).await;
    let _ = cm.inner.docker.remove_volume(&format!("agentmux-cli-{name}"), None).await;
    let _ = cm.inner.docker.remove_volume(&format!("agentmux-claude-{name}"), None).await;

    cm.ensure_running_observed(name, &image, &[], &[], &ContainerMountSpec::default(), &|_| {}).await.expect("ensure_running");
    let install = CliInstall::for_provider("claude", "claude").unwrap();

    let mut announced = 0;
    let first = cm.provision_cli(name, &install, || announced += 1).await.expect("first provision");
    assert!(matches!(first, CliProvision::Installed { .. }), "{first:?}");
    assert_eq!(announced, 1, "the install is announced once");

    // Same manager: answered from the in-memory cache, nothing announced.
    let again = cm.provision_cli(name, &install, || announced += 1).await.expect("second provision");
    assert_eq!(again, CliProvision::Present);
    assert_eq!(announced, 1);

    // A fresh manager has no cache: the marker on the volume answers.
    let fresh = ContainerManager::connect().expect("connect");
    let after_restart = fresh.provision_cli(name, &install, || announced += 1).await.expect("provision after restart");
    assert_eq!(after_restart, CliProvision::Present);
    assert_eq!(announced, 1, "a marker on the volume means no reinstall");

    // A turn-style command (the PATH append the wrapper does) runs the CLI.
    let out = fresh
        .exec_collect(
            name,
            &["sh".into(), "-c".into(), format!("PATH=\"$PATH:{}\"; export PATH; claude --version", crate::backend::container_cli::cli_bin_dir())],
            std::time::Duration::from_secs(60),
        )
        .await
        .expect("run claude");
    assert_eq!(out.exit, Some(0), "{}", out.output);
    assert!(out.output.contains(&install.version), "{}", out.output);

    // The container is recreated (an image update, say): the volume keeps the install.
    cm.remove(name, true).await.expect("remove");
    cm.ensure_running_observed(name, &image, &[], &[], &ContainerMountSpec::default(), &|_| {}).await.expect("recreate");
    let recreated = cm.provision_cli(name, &install, || announced += 1).await.expect("provision after recreate");
    assert_eq!(recreated, CliProvision::Present);
    assert_eq!(announced, 1, "the volume survives the container");

    cm.remove(name, true).await.expect("cleanup");
    let _ = cm.inner.docker.remove_volume(&format!("agentmux-cli-{name}"), None).await;
    let _ = cm.inner.docker.remove_volume(&format!("agentmux-claude-{name}"), None).await;
}

/// Docker-gated integration test for the dev-proxy network attachment
/// added by docs/specs/SPEC_NATIVE_CONTAINER_DEV_PROXY_2026_09_19.md.
/// Requires a reachable Docker daemon; `#[ignore]` by default. Run with:
///   cargo test -p agentmux-srv -- --ignored --nocapture itest_dev_proxy
///
/// Confirms, against a REAL daemon (not a mock): `ensure_running`
/// creates the shared `AGENTMUX_DOCKER_NETWORK` and attaches the
/// container to it, `agent_network_ip` resolves a real IP on that
/// network, and `remove` clears the container's dev-proxy
/// registrations via the attached registry — the same path
/// `bootstrap::build_app_state` wires up for real srv instances.
#[tokio::test]
#[ignore]
async fn itest_dev_proxy_network_attach_and_ip_resolution() {
    let cm = ContainerManager::connect().expect("connect to docker (set DOCKER_HOST)");
    cm.check_available().await.expect("docker daemon must be reachable");

    let name = "agentmux-itest-devproxy";
    let image = "nginx:alpine";
    let _ = cm.remove(name, true).await; // clean slate; ignore if absent

    let registry = crate::backend::dev_proxy::DevProxyRegistry::new();
    cm.attach_dev_proxy_registry(registry.clone());

    cm.ensure_running_observed(name, image, &[], &[], &ContainerMountSpec::default(), &|_| {})
        .await
        .expect("ensure_running (create)");

    let ip = cm
        .agent_network_ip(name)
        .await
        .expect("container must have an IP on AGENTMUX_DOCKER_NETWORK after ensure_running");
    assert!(
        ip.parse::<std::net::Ipv4Addr>().is_ok(),
        "agent_network_ip must return a real IPv4 address, got {ip:?}"
    );

    // Simulate a real registration (what `handle_register_dev_server`
    // does) so we can prove `remove` actually clears it below.
    let addr: std::net::SocketAddr = format!("{ip}:3000").parse().expect("valid addr");
    registry.register_from("itest-proj", "itest-devproxy", Some(name), addr).await;
    assert_eq!(
        registry.lookup("itest-proj-itest-devproxy").await,
        Some(addr.into()),
        "registration must be visible via lookup before cleanup"
    );

    // cleanup — also exercises `cleanup_dev_proxy_routes`.
    cm.remove(name, true).await.expect("remove");

    assert_eq!(
        registry.lookup("itest-proj-itest-devproxy").await,
        None,
        "remove() must clear this agent's dev-proxy registrations, not leave a stale route"
    );
}

/// A container agent that switches account must be recreated with the new account's credentials:
/// the file it has mounted is the old account's, so the drift check sees a changed source.
#[test]
fn a_container_made_for_one_account_has_drift_for_another_account() {
    use bollard::models::{ContainerInspectResponse, MountPoint};
    let spec_of = |dir: &str| ContainerMountSpec { claude_config_host_dir: Some(dir.into()), workspace_host_dir: Some("/work/a".into()), ..Default::default() };
    let old = spec_of("/ids/acct-1/claude");
    let made_for = |spec: &ContainerMountSpec| ContainerInspectResponse {
        mounts: Some(
            ContainerManager::desired_owned_mounts("agentmux-a-1", spec)
                .into_iter()
                .map(|(target, source)| {
                    let named = !source.contains('/');
                    MountPoint { destination: Some(target), name: named.then(|| source.clone()), source: Some(source), ..Default::default() }
                })
                .collect(),
        ),
        ..Default::default()
    };
    let details = made_for(&old);
    assert!(ContainerManager::mounts_match(&details, "agentmux-a-1", &old));
    assert!(!ContainerManager::mounts_match(&details, "agentmux-a-1", &spec_of("/ids/acct-2/claude")), "the new account's login is not mounted yet");
    // And one that has no account any more.
    let none = ContainerMountSpec { workspace_host_dir: Some("/work/a".into()), ..Default::default() };
    assert!(!ContainerManager::mounts_match(&details, "agentmux-a-1", &none));
}
