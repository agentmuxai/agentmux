// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Docker side of the container liveness sweep (`background_task_container_sweep`): ask a container,
//! through the exec API, whether a process is still there and when it started.

use std::sync::Arc;
use std::time::Duration;

use futures_util::future::BoxFuture;
use futures_util::StreamExt;

use super::background_task_container_sweep::{verdict_of, ContainerProbe, Verdict};
use super::container::runtime::ContainerRuntimeHandle;
use super::container::{ContainerError, ContainerManager};

/// How long one probe may take, start to answer. A container that does not answer is not one to end rows for.
const PROBE_LIMIT: Duration = Duration::from_secs(15);

/// Prints `gone`, or `alive <start of the process, unix seconds>` (`alive` alone when the start can't be read).
/// A zombie is gone. The start comes from `/proc/<pid>/stat` (field 22, in clock ticks since boot) and the
/// kernel's boot time, which is how `pid_alive_since` reads a host pid's. The pid is an argument, never part of
/// the script's text.
const SCRIPT: &str = r#"p="$1"
[ -r "/proc/$p/stat" ] || { echo gone; exit 0; }
rest=$(sed 's/^.*) //' "/proc/$p/stat" 2>/dev/null)
state=${rest%% *}
[ "$state" = "Z" ] || [ "$state" = "X" ] && { echo gone; exit 0; }
ticks=$(echo "$rest" | cut -d' ' -f20)
boot=$(awk '/^btime/ {print $2}' /proc/stat 2>/dev/null)
hz=$(getconf CLK_TCK 2>/dev/null || echo 100)
if [ -n "$ticks" ] && [ -n "$boot" ] && [ "$hz" -gt 0 ] 2>/dev/null; then echo "alive $((boot + ticks / hz))"; else echo alive; fi"#;

/// Whether a refused exec means the container is not running: Docker answers 404 for one that does not exist
/// and 409 for one that is stopped, paused or restarting.
fn container_is_down(e: &ContainerError) -> bool {
    matches!(e, ContainerError::Docker(bollard::errors::Error::DockerResponseServerError { status_code: 404 | 409, .. }))
}

/// Run the probe for `pid` in `container`.
pub async fn probe(cm: &ContainerManager, container: &str, pid: u32, task_started_ms: i64) -> Verdict {
    let argv: Vec<String> = ["sh", "-c", SCRIPT, "sh"].iter().map(|s| s.to_string()).chain(std::iter::once(pid.to_string())).collect();
    let ask = async {
        let session = match cm.exec(container, &argv, None, &[], false).await {
            Ok(session) => session,
            Err(e) if container_is_down(&e) => return Verdict::ContainerDown,
            Err(_) => return Verdict::Unknown,
        };
        let mut stream = session.output;
        let mut printed = String::new();
        while let Some(Ok(chunk)) = stream.next().await {
            printed.push_str(&String::from_utf8_lossy(&chunk.into_bytes()));
            if printed.len() > 4096 {
                break;
            }
        }
        // The stream ending is not the exit: a shell that failed to run the script printed nothing readable.
        verdict_of(&printed, task_started_ms)
    };
    tokio::time::timeout(PROBE_LIMIT, ask).await.unwrap_or(Verdict::Unknown)
}

/// The sweep's probe, through whichever container runtime srv is connected to now.
pub struct DockerProbe {
    pub runtime: Arc<ContainerRuntimeHandle>,
}

impl ContainerProbe for DockerProbe {
    fn probe<'a>(&'a self, container: &'a str, pid: u32, task_started_ms: i64) -> BoxFuture<'a, Verdict> {
        Box::pin(async move {
            match self.runtime.get().await {
                Some(cm) => probe(&cm, container, pid, task_started_ms).await,
                None => Verdict::Unknown,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    fn run(args: &[&str]) -> String {
        let out = std::process::Command::new("sh").arg("-c").arg(SCRIPT).arg("sh").args(args).output().expect("sh runs");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    #[test]
    fn a_refused_exec_means_down_only_for_a_missing_or_stopped_container() {
        let refused = |status_code| ContainerError::Docker(bollard::errors::Error::DockerResponseServerError { status_code, message: String::new() });
        assert!(container_is_down(&refused(404)) && container_is_down(&refused(409)));
        assert!(!container_is_down(&refused(500)), "a daemon error says nothing about the container");
        assert!(!container_is_down(&ContainerError::NotAvailable("no docker".into())));
    }

    /// The script, run where /proc is Linux's: a process of this test is alive with a start time that is not
    /// after now; a pid no process has is gone.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_script_reads_a_live_process_and_a_missing_one() {
        let now = agentmux_common::time::now_ms() / 1000;
        let mine = run(&[&std::process::id().to_string()]);
        let started: i64 = mine.strip_prefix("alive ").expect(&mine).parse().unwrap();
        assert!(started <= now + 2 && started > now - 7 * 24 * 3600, "{mine}");
        assert_eq!(run(&["4194999"]), "gone");
        assert_eq!(verdict_of(&mine, now * 1000), Verdict::Alive);
    }
}
