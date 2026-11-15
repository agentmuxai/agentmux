// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A one-shot, locked-down container: a command that runs once, with nothing but what it is given,
//! and whose output comes back (a Drone Run step, docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.6).
//!
//! It is not an agent's container. It has no network, no host mounts, a read-only root, no
//! capabilities, no privilege gain, a non-root user, and limits on memory, processes and CPU. The
//! command is an argv run directly (the image's entrypoint is replaced), never through a shell. The
//! container is removed when the command ends, and when the caller gives up (a timeout, a stop).

use std::collections::HashMap;
use std::time::Instant;

use bollard::container::{AttachContainerOptions, Config, CreateContainerOptions, LogOutput, RemoveContainerOptions, StartContainerOptions, WaitContainerOptions};
use bollard::models::{HostConfig, HostConfigLogConfig};
use bollard::Docker;
use futures_util::StreamExt as _;

use super::ContainerManager;

/// The most of stdout, and of stderr, that is kept.
pub const OUTPUT_CAP: usize = 64 * 1024;
/// The label every one-shot container carries, so a leftover can be found.
pub const LABEL: &str = "agentmux.drone-run";
/// An unprivileged user and group that own nothing.
const USER: &str = "65534:65534";
const MEMORY_BYTES: i64 = 512 * 1024 * 1024;
const PIDS: i64 = 256;
/// One CPU.
const NANO_CPUS: i64 = 1_000_000_000;

/// What to run.
#[derive(Debug, Clone, PartialEq)]
pub struct OneShot {
    pub image: String,
    /// The program and its arguments; the first is never empty.
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
    /// The working directory inside the container; the image's own when none.
    pub cwd: Option<String>,
}

/// What came of it.
#[derive(Debug, Clone, PartialEq)]
pub struct OneShotOutcome {
    pub exit_code: i64,
    pub stdout: String,
    pub stderr: String,
    /// Some output was cut at [`OUTPUT_CAP`].
    pub truncated: bool,
    pub duration_ms: u64,
}

/// The container's configuration. Pure, so the locks are testable without a runtime.
pub fn oneshot_config(spec: &OneShot) -> Config<String> {
    let (program, args) = spec.argv.split_first().expect("an argv has a program");
    Config {
        image: Some(spec.image.clone()),
        // The image's entrypoint is replaced, so the program is run as given and nothing else is.
        entrypoint: Some(vec![program.clone()]),
        cmd: Some(args.to_vec()),
        env: Some(spec.env.iter().map(|(k, v)| format!("{k}={v}")).collect()),
        working_dir: spec.cwd.clone(),
        user: Some(USER.to_string()),
        network_disabled: Some(true),
        tty: Some(false),
        open_stdin: Some(false),
        labels: Some(HashMap::from([(LABEL.to_string(), "1".to_string())])),
        host_config: Some(HostConfig {
            network_mode: Some("none".to_string()),
            readonly_rootfs: Some(true),
            cap_drop: Some(vec!["ALL".to_string()]),
            security_opt: Some(vec!["no-new-privileges".to_string()]),
            memory: Some(MEMORY_BYTES),
            memory_swap: Some(MEMORY_BYTES),
            pids_limit: Some(PIDS),
            nano_cpus: Some(NANO_CPUS),
            init: Some(true),
            // The only place it can write, and gone with the container.
            tmpfs: Some(HashMap::from([("/tmp".to_string(), "rw,noexec,nosuid,size=64m".to_string())])),
            // The output is read through an attach, not from a log: no log is written or rotated.
            log_config: Some(HostConfigLogConfig { typ: Some("none".to_string()), config: None }),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Removes the container when dropped, which is what happens to a run that is stopped or times out.
/// A run that ends on its own removes it first, so it is gone when the run returns.
struct Remove {
    docker: Docker,
    name: String,
    done: bool,
}

impl Remove {
    async fn now(mut self) {
        remove(&self.docker, &self.name).await;
        // Only once it is gone: a stop that lands while the removal is awaited still drops an armed guard.
        self.done = true;
    }
}

impl Drop for Remove {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        let (docker, name) = (self.docker.clone(), self.name.clone());
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move { remove(&docker, &name).await });
        }
    }
}

async fn remove(docker: &Docker, name: &str) {
    let _ = docker.remove_container(name, Some(RemoveContainerOptions { force: true, v: true, ..Default::default() })).await;
}

impl ContainerManager {
    /// Run `spec` once and give what it said. The container is removed whatever happens, including
    /// when this future is dropped. A command that exits non-zero is an outcome, not an error.
    pub async fn run_oneshot(&self, spec: &OneShot) -> Result<OneShotOutcome, String> {
        let docker = &self.inner.docker;
        let mut spec = spec.clone();
        if !self.has_image_locally(&spec.image).await {
            spec.image = self.pull_with_fallback(&spec.image, &|_| {}).await.map_err(|e| e.to_string())?;
        }
        let name = format!("agentmux-drone-run-{}", uuid::Uuid::new_v4().simple());
        docker
            .create_container(Some(CreateContainerOptions { name: name.as_str(), platform: None }), oneshot_config(&spec))
            .await
            .map_err(|e| format!("couldn't create the container: {e}"))?;
        let remove = Remove { docker: docker.clone(), name: name.clone(), done: false };
        // Attached before it starts, so every byte it says comes through, however much it says: a log
        // read afterwards, or followed, can lose what a command that is loud writes faster than it is read.
        let attached = docker
            .attach_container(
                &name,
                Some(AttachContainerOptions::<String> { stream: Some(true), stdout: Some(true), stderr: Some(true), logs: Some(false), ..Default::default() }),
            )
            .await
            .map_err(|e| format!("couldn't attach to the container: {e}"))?;
        let started = Instant::now();
        docker.start_container(&name, None::<StartContainerOptions<String>>).await.map_err(|e| format!("couldn't start the container: {e}"))?;
        let (ended, logs) = tokio::join!(wait(docker, &name), read_output(attached.output));
        let duration_ms = started.elapsed().as_millis() as u64;
        remove.now().await;
        let exit_code = ended?;
        let (stdout, stderr, truncated) = logs?;
        Ok(OneShotOutcome { exit_code, stdout, stderr, truncated, duration_ms })
    }
}

/// How the container ended.
async fn wait(docker: &Docker, name: &str) -> Result<i64, String> {
    let mut ended = docker.wait_container(name, Some(WaitContainerOptions { condition: "not-running" }));
    match ended.next().await {
        Some(Ok(done)) => Ok(done.status_code),
        // The client reports a non-zero exit as an error that carries the code.
        Some(Err(bollard::errors::Error::DockerContainerWaitError { code, .. })) => Ok(code),
        Some(Err(e)) => Err(format!("couldn't wait for the container: {e}")),
        None => Err("the container's end was not reported".to_string()),
    }
}

/// What the container wrote to stdout and stderr, each cut at [`OUTPUT_CAP`]; reads until it ends.
async fn read_output(mut logs: impl futures_util::Stream<Item = Result<LogOutput, bollard::errors::Error>> + Unpin) -> Result<(String, String, bool), String> {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let mut truncated = false;
    while let Some(line) = logs.next().await {
        let (into, bytes) = match line.map_err(|e| format!("couldn't read the container's output: {e}"))? {
            LogOutput::StdOut { message } => (&mut out, message),
            LogOutput::StdErr { message } => (&mut err, message),
            _ => continue,
        };
        let room = OUTPUT_CAP.saturating_sub(into.len());
        truncated |= bytes.len() > room;
        into.extend_from_slice(&bytes[..bytes.len().min(room)]);
    }
    Ok((lossy(out), lossy(err), truncated))
}

/// Text from bytes that a cut may have ended in the middle of a character.
fn lossy(bytes: Vec<u8>) -> String {
    String::from_utf8_lossy(&bytes).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(argv: &[&str]) -> OneShot {
        OneShot { image: "img:1".into(), argv: argv.iter().map(|s| s.to_string()).collect(), env: vec![("A".into(), "b".into())], cwd: None }
    }

    #[test]
    fn the_program_is_run_as_given_and_nothing_else_is() {
        let c = oneshot_config(&spec(&["echo", "hi", "there"]));
        assert_eq!(c.entrypoint, Some(vec!["echo".to_string()]), "the image's own entrypoint is replaced");
        assert_eq!(c.cmd, Some(vec!["hi".to_string(), "there".to_string()]));
        assert_eq!(c.env, Some(vec!["A=b".to_string()]));
    }

    #[test]
    fn the_container_has_no_network_no_host_files_and_no_privilege() {
        let c = oneshot_config(&spec(&["true"]));
        let h = c.host_config.unwrap();
        assert_eq!(c.network_disabled, Some(true));
        assert_eq!(h.network_mode.as_deref(), Some("none"));
        assert_eq!(h.readonly_rootfs, Some(true));
        assert_eq!(h.cap_drop, Some(vec!["ALL".to_string()]));
        assert_eq!(h.security_opt, Some(vec!["no-new-privileges".to_string()]));
        assert!(h.mounts.is_none() && h.binds.is_none(), "nothing from the host is mounted");
        assert_eq!(h.privileged, None);
        assert_ne!(c.user.as_deref(), Some("root"));
        assert_eq!(h.pids_limit, Some(PIDS));
        assert_eq!(h.memory, Some(MEMORY_BYTES));
        assert_eq!(h.memory_swap, Some(MEMORY_BYTES), "no swap beyond the limit");
    }

    #[test]
    fn it_is_labelled_so_a_leftover_can_be_found() {
        assert_eq!(oneshot_config(&spec(&["true"])).labels.unwrap().get(LABEL).map(String::as_str), Some("1"));
    }

    /// Needs a container runtime and an image: `cargo test ... -- --ignored one_shot`.
    #[tokio::test]
    #[ignore]
    async fn one_shot_runs_a_command_in_a_container_and_gives_its_output() {
        let cm = ContainerManager::connect_detected().await.expect("a container runtime");
        let image = super::super::image::resolve_container_image(super::super::image::DEFAULT_AGENT_IMAGE);
        let run = |argv: &[&str]| {
            let cm = cm.clone();
            let s = OneShot { image: image.clone(), argv: argv.iter().map(|s| s.to_string()).collect(), env: vec![("GREETING".into(), "hello".into())], cwd: None };
            async move { cm.run_oneshot(&s).await.unwrap() }
        };
        let echo = run(&["printenv", "GREETING"]).await;
        assert_eq!((echo.exit_code, echo.stdout.trim()), (0, "hello"));
        let fails = run(&["sh", "-c", "echo oops >&2; exit 3"]).await;
        assert_eq!((fails.exit_code, fails.stderr.trim()), (3, "oops"));
        // No network: a connection is refused at once.
        let net = run(&["sh", "-c", "wget -q -T 3 -O - http://1.1.1.1/ || curl -sS -m 3 http://1.1.1.1/"]).await;
        assert_ne!(net.exit_code, 0, "{net:?}");
        // The root is read-only, /tmp is not.
        assert_ne!(run(&["touch", "/etc/x"]).await.exit_code, 0);
        assert_eq!(run(&["touch", "/tmp/x"]).await.exit_code, 0);
        // Cut at the cap.
        let big = run(&["sh", "-c", "head -c 200000 /dev/zero | tr '\\0' a"]).await;
        assert_eq!(big.stdout.len(), OUTPUT_CAP);
        assert!(big.truncated);
        // More than the container's own log keeps: the first line, said before it, is not lost.
        let loud = run(&["sh", "-c", "echo first >&2; head -c 3000000 /dev/zero | tr '\\0' a"]).await;
        assert_eq!(loud.stderr.trim(), "first");
        assert_eq!(loud.stdout.len(), OUTPUT_CAP);
        assert!(loud.truncated);
    }
}
