// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Reading from inside a container agent's container: one command run there, as the user the agent's turns
//! run as, with its output collected. Never starts a container or a runtime: one that is not running has no
//! answer.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use bollard::container::LogOutput;
use futures_util::StreamExt as _;

use super::ContainerManager;
use crate::backend::container::ContainerRuntimeHandle;
use crate::backend::container_status::ContainerState;

/// What a command run inside a container printed and how it ended.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReadOutput {
    /// `None` when the runtime could not say.
    pub exit: Option<i64>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// The exit code of a command that was stopped for running past its limit.
pub const TIMED_OUT_EXIT: i64 = 124;

/// The answer for a container that is not running.
pub const NOT_RUNNING: &str = "The agent's container isn't running.";

/// How much of the command's error text is kept.
const KEEP_STDERR: usize = 16 * 1024;

/// How long past its limit a command is waited for before the read gives up on it.
const GRACE: Duration = Duration::from_secs(3);

/// Runs the command (`$2`...) for at most `$1` seconds where the image has `timeout`: a command started in a
/// container can't be stopped from outside it, so it is stopped from inside.
const BOUNDED: &str = r#"t=$1; shift; if command -v timeout >/dev/null 2>&1; then exec timeout -k 1 "$t" "$@"; fi; exec "$@""#;

/// `argv` as it is run: under [`BOUNDED`], with `limit` in whole seconds.
fn bounded(argv: &[String], limit: Duration) -> Vec<String> {
    let mut cmd = vec!["sh".to_string(), "-c".to_string(), BOUNDED.to_string(), "sh".to_string(), limit.as_secs().max(1).to_string()];
    cmd.extend(argv.iter().cloned());
    cmd
}

impl ContainerManager {
    /// Runs `argv` in `container` with `env` added, for at most `limit`, and collects what it printed: at most
    /// `max_stdout` bytes of output (more is an error) and the start of its error text.
    pub(crate) async fn exec_read(
        &self,
        container: &str,
        argv: &[String],
        env: &[(String, String)],
        limit: Duration,
        max_stdout: usize,
    ) -> Result<ReadOutput, String> {
        let session = self.exec(container, &bounded(argv, limit), None, env, false).await.map_err(|e| e.to_string())?;
        let exec_id = session.exec_id.clone();
        drop(session.input);
        let mut stream = session.output;
        let mut out = ReadOutput::default();
        let drain = async {
            while let Some(item) = stream.next().await {
                match item.map_err(|e| e.to_string())? {
                    LogOutput::StdOut { message } => {
                        if out.stdout.len() + message.len() > max_stdout {
                            return Err("It printed more than can be read.".to_string());
                        }
                        out.stdout.extend_from_slice(&message);
                    }
                    LogOutput::StdErr { message } if out.stderr.len() < KEEP_STDERR => out.stderr.extend_from_slice(&message),
                    _ => {}
                }
            }
            Ok(())
        };
        match tokio::time::timeout(limit + GRACE, drain).await {
            Err(_) => return Err("It took too long to answer.".to_string()),
            Ok(done) => done?,
        }
        // The stream can end a moment before the runtime has the exit code.
        for _ in 0..10 {
            out.exit = self.inspect_exec(&exec_id).await.map_err(|e| e.to_string())?;
            if out.exit.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Ok(out)
    }
}

static RUNTIME: OnceLock<Arc<ContainerRuntimeHandle>> = OnceLock::new();

/// Name the container runtime that reads are made through. Called at start-up and by what leads to a read;
/// the first call wins.
pub fn install(handle: &Arc<ContainerRuntimeHandle>) {
    let _ = RUNTIME.set(Arc::clone(handle));
}

/// Whether the containers' status says `container` is there and not running.
fn known_not_running(container: &str) -> bool {
    crate::backend::container_status::hub()
        .is_some_and(|hub| hub.list().containers.iter().any(|c| c.name == container && c.state != ContainerState::Running))
}

/// Run `argv` in `container` as its agent (see [`ContainerManager::exec_read`]). `Err` with the reason when
/// the container is not running, there is no runtime, or the command could not be run or read.
pub async fn read(
    container: &str,
    argv: &[String],
    env: &[(String, String)],
    limit: Duration,
    max_stdout: usize,
) -> Result<ReadOutput, String> {
    #[cfg(test)]
    {
        let stand_in = stand_ins().lock().get(container).cloned();
        if let Some(stand_in) = stand_in {
            return stand_in(argv, env);
        }
    }
    if container.is_empty() || known_not_running(container) {
        return Err(NOT_RUNNING.to_string());
    }
    let manager = match RUNTIME.get() {
        Some(handle) => handle.get().await,
        None => None,
    };
    let Some(manager) = manager else {
        return Err("No container runtime is running.".to_string());
    };
    manager.exec_read(container, argv, env, limit, max_stdout).await
}

#[cfg(test)]
type StandIn = Arc<dyn Fn(&[String], &[(String, String)]) -> Result<ReadOutput, String> + Send + Sync>;

#[cfg(test)]
fn stand_ins() -> &'static parking_lot::Mutex<std::collections::HashMap<String, StandIn>> {
    static STAND_INS: std::sync::LazyLock<parking_lot::Mutex<std::collections::HashMap<String, StandIn>>> =
        std::sync::LazyLock::new(Default::default);
    &STAND_INS
}

/// For the tests of this crate: what a read from `container` answers, in place of a runtime (each test names
/// a container of its own).
#[cfg(test)]
pub(crate) fn stand_in_for_test(
    container: &str,
    answer: impl Fn(&[String], &[(String, String)]) -> Result<ReadOutput, String> + Send + Sync + 'static,
) {
    stand_ins().lock().insert(container.to_string(), Arc::new(answer));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_is_run_under_a_limit_and_never_spliced_into_the_script() {
        let argv = vec!["git".to_string(), "-C".to_string(), "/workspace/a b; rm -rf /".to_string(), "status".to_string()];
        let cmd = bounded(&argv, Duration::from_millis(4500));
        assert_eq!(&cmd[..5], ["sh", "-c", BOUNDED, "sh", "4"]);
        assert_eq!(&cmd[5..], &argv[..], "each word is one argument of the script");
        assert_eq!(bounded(&argv, Duration::from_millis(10))[4], "1", "never a limit of none");
    }

    #[tokio::test]
    async fn a_container_that_is_not_there_has_no_answer_and_nothing_is_started() {
        assert_eq!(read("", &[], &[], Duration::from_secs(1), 10).await, Err(NOT_RUNNING.to_string()));
        // No runtime was named in this test process: nothing is looked for.
        let other = read("agentmux-read-exec-none", &["true".to_string()], &[], Duration::from_secs(1), 10).await;
        assert!(other.is_err(), "{other:?}");
    }

    /// Live check against a real container runtime:
    ///   cargo test -p agentmux-srv-core --lib -- --ignored --nocapture itest_exec_read
    #[tokio::test]
    #[ignore]
    async fn itest_exec_read_collects_output_and_exit_and_stops_a_slow_command() {
        let cm = ContainerManager::connect().expect("connect to docker (set DOCKER_HOST)");
        cm.check_available().await.expect("docker daemon must be reachable");
        let name = "agentmux-itest-read-exec";
        let image = "ghcr.io/agentmuxai/agent-base:0.59.41";
        let clean = || async {
            let _ = cm.remove(name, true).await;
            for volume in ["claude", "cli"] {
                let _ = cm.inner.docker.remove_volume(&format!("agentmux-{volume}-{name}"), None).await;
            }
        };
        clean().await;
        let spec = crate::backend::container::ContainerMountSpec::default();
        cm.ensure_running_observed(name, image, &[], &[], &spec, &|_| {}).await.expect("ensure_running");

        let words = |w: &[&str]| w.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let pair = |k: &str, v: &str| (k.to_string(), v.to_string());
        let limit = Duration::from_secs(4);
        let both = cm.exec_read(name, &words(&["sh", "-c", "printf 'out\\0put|'; printf err >&2; id -u"]), &[], limit, 1024).await;
        let env = [pair("GIT_ALLOW_PROTOCOL", ""), pair("GIT_NO_LAZY_FETCH", "1")];
        let with_env = cm.exec_read(name, &words(&["sh", "-c", "printf %s \"${GIT_ALLOW_PROTOCOL-unset}|$GIT_NO_LAZY_FETCH\""]), &env, limit, 1024).await;
        let failed = cm.exec_read(name, &words(&["sh", "-c", "exit 3"]), &[], limit, 1024).await;
        let git = cm.exec_read(name, &words(&["git", "-C", "/nowhere", "rev-parse", "--show-toplevel"]), &[], limit, 1024).await;
        let began = std::time::Instant::now();
        let slow = cm.exec_read(name, &words(&["sleep", "31"]), &[], Duration::from_secs(1), 1024).await;
        let took = began.elapsed();
        tokio::time::sleep(Duration::from_secs(2)).await;
        // The slow command by its whole command line: the container's own idle process is a `sleep` too, and
        // that one is counted to show the look sees a command that is still running.
        let left = cm.exec_read(name, &words(&["sh", "-c", "pgrep -fx 'sleep 31' | wc -l"]), &[], limit, 1024).await;
        let kept = cm.exec_read(name, &words(&["sh", "-c", "pgrep -fx 'sleep infinity' | wc -l"]), &[], limit, 1024).await;
        let big = cm.exec_read(name, &words(&["sh", "-c", "head -c 5000 /dev/zero"]), &[], limit, 1024).await;
        clean().await;
        println!("both={both:?}\nwith_env={with_env:?}\nfailed={failed:?}\ngit={git:?}\nslow={slow:?} took={took:?}\nleft={left:?}\nkept={kept:?}\nbig={big:?}");

        let both = both.expect("a command that prints");
        assert_eq!(both.exit, Some(0));
        assert!(both.stdout.starts_with(b"out\0put|") && both.stdout.ends_with(b"\n"), "output is kept byte for byte: {both:?}");
        assert_ne!(&both.stdout[8..], b"0\n", "it runs as the agent's user, not as root");
        assert_eq!(both.stderr, b"err", "the error text is apart from the output");
        assert_eq!(with_env.expect("a command with an environment").stdout, b"|1", "an empty value is set, not left out");
        let failed = failed.expect("a command that fails");
        assert_eq!((failed.exit, failed.stderr.is_empty()), (Some(3), true));
        let git = git.expect("git in a folder that is not there");
        assert!(git.exit.is_some_and(|code| code != 0) && !git.stderr.is_empty(), "{git:?}");
        assert_eq!(slow.expect("a command that runs past its limit").exit, Some(TIMED_OUT_EXIT));
        assert!(took < limit + GRACE, "took {took:?}");
        assert_eq!(String::from_utf8_lossy(&left.expect("a look for what is left").stdout).trim(), "0", "the slow command was stopped inside the container");
        assert_eq!(String::from_utf8_lossy(&kept.expect("a look for what is kept").stdout).trim(), "1", "the look counts a command that is still running");
        assert!(big.is_err(), "more output than asked for is refused: {big:?}");
    }

    #[tokio::test]
    async fn a_stand_in_answers_for_its_container_only() {
        stand_in_for_test("agentmux-read-exec-a", |argv, _| Ok(ReadOutput { exit: Some(0), stdout: argv.join(" ").into_bytes(), stderr: Vec::new() }));
        let got = read("agentmux-read-exec-a", &["echo".to_string(), "hi".to_string()], &[], Duration::from_secs(1), 10).await.unwrap();
        assert_eq!(got.stdout, b"echo hi");
    }
}
