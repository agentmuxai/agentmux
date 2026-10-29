// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Running a container agent's dev server for it: started detached, and asked whether it is listening.

use bollard::exec::{CreateExecOptions, StartExecOptions};

use super::ContainerManager;
use crate::backend::container::ContainerError;

impl ContainerManager {
    /// The dev-proxy routing table, once wired in.
    pub(crate) fn dev_proxy_registry(&self) -> Option<&crate::backend::dev_proxy::DevProxyRegistry> {
        self.inner.dev_proxy.get()
    }

    /// Whether something in `container` is listening on `port`.
    pub(crate) async fn port_listening(&self, container: &str, port: u16) -> bool {
        let probe = format!("exec 3<>/dev/tcp/127.0.0.1/{port}");
        let argv = ["bash".to_string(), "-c".to_string(), probe];
        matches!(
            self.exec_collect(container, &argv, std::time::Duration::from_secs(5)).await,
            Ok(out) if !out.timed_out && out.exit == Some(0)
        )
    }

    /// Starts `command` in `dir` inside `container`, detached (like `docker exec -d`), as the container's own user,
    /// with `env` added and its output in `log`. It keeps running after this returns, until the container stops.
    pub(crate) async fn start_dev_server(
        &self,
        container: &str,
        command: &str,
        dir: &str,
        env: &[(String, String)],
        log: &str,
    ) -> Result<(), ContainerError> {
        let env: Vec<String> = env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        // The command is the shell's `$0`, the log its `$1`: never spliced into a script.
        let script = r#"exec setsid nohup sh -c "$0" >"$1" 2>&1 </dev/null"#;
        let exec = self
            .inner
            .docker
            .create_exec(
                container,
                CreateExecOptions {
                    cmd: Some(vec!["sh", "-c", script, command, log]),
                    working_dir: Some(dir),
                    env: if env.is_empty() { None } else { Some(env.iter().map(String::as_str).collect()) },
                    attach_stdout: Some(false),
                    attach_stderr: Some(false),
                    ..Default::default()
                },
            )
            .await?;
        self.inner.docker.start_exec(&exec.id, Some(StartExecOptions { detach: true, ..Default::default() })).await?;
        Ok(())
    }
}
