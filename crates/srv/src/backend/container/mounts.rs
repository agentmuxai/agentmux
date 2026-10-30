//! The mounts of a container agent's home: the per-agent volume, the bound account's
//! credentials, and the agent's Personal Memory folder.

mod credential_mode;
mod dev_exec;

use bollard::exec::{CreateExecOptions, StartExecOptions, StartExecResults};
use bollard::models::{Mount, MountTypeEnum};
use futures_util::StreamExt as _;

use super::{
    normalize_host_path, ContainerManager, ContainerMountSpec, CLAUDE_CREDENTIALS_FILE, CONTAINER_CLAUDE_DIR,
    CONTAINER_WORKSPACE_DIR,
};

/// Where Claude in the container reads and writes the agent's Personal Memory: the folder
/// it names for its working directory, [`CONTAINER_WORKSPACE_DIR`], under its config dir.
pub(super) fn container_memory_dir() -> String {
    let project = crate::backend::claude_layout::project_dir_name(CONTAINER_WORKSPACE_DIR);
    format!("{CONTAINER_CLAUDE_DIR}/projects/{project}/memory")
}

/// The folder a native session of this agent keeps its Personal Memory in, for
/// [`ContainerMountSpec::memory_host_dir`]: created if it is not there yet (Docker refuses a
/// bind whose source is missing) and resolved through symlinks on Linux and macOS, so the
/// daemon is given the real directory. `None` when `working_dir` is empty.
pub(crate) fn personal_memory_host_dir(claude_config_dir: Option<&str>, working_dir: &str) -> Option<String> {
    if working_dir.is_empty() {
        return None;
    }
    let dir = crate::server::native_memory_handlers::memory_dir_for_cwd(claude_config_dir.unwrap_or(""), working_dir);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(dir = %dir.display(), error = %e, "container agent: can't create the Personal Memory folder; not mounting it");
        return None;
    }
    let dir = if cfg!(windows) { dir } else { dir.canonicalize().unwrap_or(dir) };
    Some(dir.to_string_lossy().into_owned())
}

impl ContainerMountSpec {
    /// What an agent started with `env_vars` in `working_dir` gets mounted: its bound
    /// account's credentials, its workspace, and, for an agent with a Claude config dir,
    /// its Personal Memory. A `~/...` working dir (what `agent.open` records for an agent with
    /// none of its own) is expanded here, against the AgentMux home an isolated instance
    /// overrides: Docker takes only an absolute bind source.
    pub(crate) fn for_agent(env_vars: &std::collections::HashMap<String, String>, working_dir: &str) -> Self {
        let working_dir = crate::backend::base::expand_home_dir_safe(working_dir).to_string_lossy().into_owned();
        let working_dir = working_dir.as_str();
        let config_dir = env_vars.get("CLAUDE_CONFIG_DIR").map(String::as_str).filter(|d| !d.is_empty());
        Self {
            claude_config_host_dir: config_dir.and_then(super::credentials_dir_if_file_backed),
            workspace_host_dir: Some(working_dir.to_string()).filter(|d| !d.is_empty()),
            memory_host_dir: config_dir.and_then(|dir| personal_memory_host_dir(Some(dir), working_dir)),
            github_hooks_host_dir: crate::identity::github_tokens::container_git_hooks().map(|d| d.to_string_lossy().into_owned()),
        }
    }
}

impl ContainerManager {
    /// The daemon connection, for services that sit beside the container manager (the dev relay).
    pub(crate) fn docker(&self) -> &bollard::Docker {
        &self.inner.docker
    }

    /// Hands the folders above the memory mount to the user the container runs turns as.
    /// Docker creates them as root inside the per-agent volume, and Claude could not then
    /// write its session files into `projects/<workspace>/`. Best-effort: a failure, or a
    /// non-zero exit, is logged and the container carries on.
    pub(super) async fn own_memory_parents(&self, container_name: &str, spec: &ContainerMountSpec) {
        if spec.memory_host_dir.as_deref().is_none_or(str::is_empty) {
            return;
        }
        let memory = container_memory_dir();
        let Some(project) = memory.strip_suffix("/memory") else { return };
        let Some(projects) = project.rsplit_once('/').map(|(p, _)| p) else { return };
        let Some((uid, gid)) = self.exec_identity(container_name).await else {
            tracing::warn!(container = container_name, "container agent: can't tell which user runs turns; the memory folder's parents stay root-owned");
            return;
        };
        let script = format!("chown {uid}:{gid} '{projects}' '{project}'");
        let outcome = async {
            let exec = self
                .inner
                .docker
                .create_exec(
                    container_name,
                    CreateExecOptions {
                        user: Some("root"),
                        cmd: Some(vec!["sh", "-c", &script]),
                        attach_stdout: Some(true),
                        attach_stderr: Some(true),
                        ..Default::default()
                    },
                )
                .await?;
            if let StartExecResults::Attached { mut output, .. } = self
                .inner
                .docker
                .start_exec(&exec.id, Some(StartExecOptions { detach: false, ..Default::default() }))
                .await?
            {
                while output.next().await.is_some() {}
            }
            self.inspect_exec(&exec.id).await
        }
        .await;
        match outcome {
            Ok(Some(0)) => {}
            Ok(code) => tracing::warn!(container = container_name, ?code, "container agent: handing the memory folder's parents to the turn user failed"),
            Err(e) => tracing::warn!(container = container_name, error = %e, "container agent: couldn't hand the memory folder's parents to the turn user"),
        }
    }
}

/// The mounts that make up the agent's home dir inside the container.
///
/// The per-agent named volume stays mounted at [`CONTAINER_CLAUDE_DIR`] exactly
/// as before, and a bound account contributes ONE extra mount: its
/// `.credentials.json`, bind-mounted as a single file on top of that volume.
///
/// Binding the account's whole config dir over `…/.claude` would be the more
/// obvious shape, and it does NOT work — both failures verified live on
/// 2026-09-02 against `ghcr.io/agentmuxai/agent-claude:latest`:
///
///  1. That dir's `projects` entry is a symlink into the shared identities tree,
///     which does not resolve in-container (`ls` → "No such file or directory").
///  2. Shadowing it with a nested volume at `…/.claude/projects` SILENTLY does
///     not mount — Docker will not mount over a path that is a symlink inside a
///     bind mount. `mount` shows only the parent 9p bind, and the CLI is left
///     writing session state through the broken link. Chowning does not help;
///     there is nothing there to chown.
///
/// Keeping the volume and binding just the credential file avoids both, and
/// preserves the existing ownership story: the volume is initialised from the
/// image, so `projects`/`sessions` stay writable by uid 1000. Verified end to
/// end — a container agent using this shape completed a real turn against the
/// bound account (`"result":"AUTH_OK"`, `is_error: false`).
///
/// Known limitation — HOST UID PARITY. A bind preserves the host file's owner
/// and mode (`.credentials.json` is typically 0600), while the image always
/// execs as `agent`, uid 1000. On a host whose AgentMux user is not uid 1000,
/// the container cannot read the token and authentication stays broken there.
///
/// That is deliberately NOT worked around here. It is the container design's
/// pre-existing, documented assumption — "`agent` non-root user (UID 1000) …
/// Host filesystem UID parity" (SPEC_CONTAINER_PANE_SUPPORT_2026_06_11.md §
/// "What claw's containers provide"; the Dockerfile renames node:22-slim's
/// uid-1000 user for exactly this) — and the `/workspace` bind below has the
/// identical problem: at uid != 1000 the agent cannot write its own workspace
/// either. Special-casing credentials would fix one symptom of that assumption
/// and leave the other, while adding a second provisioning path to maintain.
/// The real fix is to exec as the host uid/gid, which is an architectural change
/// affecting every mount, and belongs in its own PR.
///
/// Known limitation: a single-file bind follows the inode. If the CLI ever
/// replaces `.credentials.json` by atomic rename rather than writing in place, a
/// refreshed token would land on the host but not be seen by the running
/// container until it is recreated. Reads — the case that was totally broken —
/// are unaffected.
pub(super) fn agent_home_mounts(container_name: &str, spec: &ContainerMountSpec) -> Vec<Mount> {
    let mut mounts = vec![Mount {
        target: Some(CONTAINER_CLAUDE_DIR.to_string()),
        source: Some(format!("agentmux-claude-{container_name}")),
        typ: Some(MountTypeEnum::VOLUME),
        read_only: Some(false),
        ..Default::default()
    }];

    if let Some(host_dir) = spec.claude_config_host_dir.as_deref().filter(|d| !d.is_empty()) {
        let host_dir = normalize_host_path(host_dir);
        mounts.push(Mount {
            target: Some(format!("{CONTAINER_CLAUDE_DIR}/{CLAUDE_CREDENTIALS_FILE}")),
            source: Some(format!("{}/{CLAUDE_CREDENTIALS_FILE}", host_dir.trim_end_matches('/'))),
            typ: Some(MountTypeEnum::BIND),
            // Writable: an in-place token refresh should reach the host copy
            // rather than being silently confined to the container.
            read_only: Some(false),
            ..Default::default()
        });
    }
    if let Some(host_dir) = spec.memory_host_dir.as_deref().filter(|d| !d.is_empty()) {
        mounts.push(Mount {
            target: Some(container_memory_dir()),
            source: Some(normalize_host_path(host_dir)),
            typ: Some(MountTypeEnum::BIND),
            read_only: Some(false),
            ..Default::default()
        });
    }
    if let Some(host_dir) = spec.github_hooks_host_dir.as_deref().filter(|d| !d.is_empty()) {
        // Read-only: the push scan is not the agent's to edit.
        mounts.push(Mount {
            target: Some(crate::identity::github_tokens::CONTAINER_HOOKS_DIR.to_string()),
            source: Some(normalize_host_path(host_dir)),
            typ: Some(MountTypeEnum::BIND),
            read_only: Some(true),
            ..Default::default()
        });
    }
    mounts
}

#[cfg(test)]
mod itest;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn bind_at<'a>(mounts: &'a [Mount], target: &str) -> Option<&'a Mount> {
        mounts.iter().find(|m| m.target.as_deref() == Some(target))
    }

    #[test]
    fn claude_reads_memory_from_the_folder_it_names_for_the_workspace() {
        assert_eq!(container_memory_dir(), "/home/agent/.claude/projects/-workspace/memory");
    }

    #[test]
    fn the_git_hooks_are_a_read_only_bind_and_only_when_there_are_some() {
        let with = ContainerMountSpec { github_hooks_host_dir: Some("/host/github-hooks".into()), ..Default::default() };
        let mounts = agent_home_mounts("agentmux-abc", &with);
        let hooks = bind_at(&mounts, crate::identity::github_tokens::CONTAINER_HOOKS_DIR).expect("the hooks are mounted");
        assert_eq!(hooks.source.as_deref(), Some("/host/github-hooks"));
        assert_eq!(hooks.read_only, Some(true), "the scan is not the agent's to edit");
        for none in [None, Some(String::new())] {
            let without = ContainerMountSpec { github_hooks_host_dir: none, ..Default::default() };
            assert!(bind_at(&agent_home_mounts("agentmux-abc", &without), crate::identity::github_tokens::CONTAINER_HOOKS_DIR).is_none());
        }
    }

    #[test]
    fn the_memory_folder_is_one_bind_and_only_when_there_is_one() {
        let with = ContainerMountSpec { memory_host_dir: Some("/host/mem".into()), ..Default::default() };
        let mounts = agent_home_mounts("agentmux-abc", &with);
        let memory = bind_at(&mounts, &container_memory_dir()).expect("memory is mounted");
        assert_eq!(memory.source.as_deref(), Some("/host/mem"));
        assert_eq!(memory.typ, Some(MountTypeEnum::BIND));
        assert_eq!(memory.read_only, Some(false));
        // Nothing wider than that one folder: no identity dir, no projects dir.
        assert!(bind_at(&mounts, &format!("{CONTAINER_CLAUDE_DIR}/projects")).is_none());
        assert_eq!(mounts.len(), 2, "the volume and the memory folder");

        for none in [None, Some(String::new())] {
            let without = ContainerMountSpec { memory_host_dir: none, ..Default::default() };
            assert!(bind_at(&agent_home_mounts("agentmux-abc", &without), &container_memory_dir()).is_none());
        }
    }

    #[test]
    fn the_host_folder_is_where_a_native_session_keeps_memory_and_is_created() {
        let root = std::env::temp_dir().join(format!("agentmux-p5-{}", std::process::id()));
        let config = root.join("claude");
        let work = root.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let got = personal_memory_host_dir(Some(&config.to_string_lossy()), &work.to_string_lossy()).expect("a folder");
        let want = crate::server::native_memory_handlers::memory_dir_for_cwd(&config.to_string_lossy(), &work.to_string_lossy());
        assert!(want.is_dir(), "created, so Docker has a source to bind");
        // Windows passes the folder through as it is (a canonical path there has the verbatim prefix, which
        // Docker doesn't take); elsewhere symlinks are resolved.
        let expected = if cfg!(windows) { want.clone() } else { want.canonicalize().unwrap() };
        assert_eq!(std::path::Path::new(&got), expected);
        assert_eq!(personal_memory_host_dir(Some(&config.to_string_lossy()), ""), None);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_tilde_working_dir_is_mounted_as_the_absolute_folder_it_names() {
        let _guard = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let fake = tempfile::tempdir().unwrap();
        let prior = std::env::var_os("AGENTMUX_HOME_OVERRIDE");
        std::env::set_var("AGENTMUX_HOME_OVERRIDE", fake.path());
        let isolated = ContainerMountSpec::for_agent(&HashMap::new(), "~/.agentmux/agents/aria");
        std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
        let normal = ContainerMountSpec::for_agent(&HashMap::new(), "~/.agentmux/agents/aria");
        if let Some(p) = prior {
            std::env::set_var("AGENTMUX_HOME_OVERRIDE", p);
        }
        assert_eq!(isolated.workspace_host_dir.as_deref().map(std::path::Path::new), Some(fake.path().join("agents").join("aria").as_path()));
        let normal = normal.workspace_host_dir.unwrap();
        assert!(std::path::Path::new(&normal).is_absolute() && !normal.contains('~'), "{normal}");
    }

    #[test]
    fn an_agent_without_a_claude_config_dir_gets_no_memory_mount() {
        let spec = ContainerMountSpec::for_agent(&HashMap::new(), "/some/work");
        assert_eq!(spec.memory_host_dir, None);
        assert_eq!(spec.workspace_host_dir.as_deref(), Some("/some/work"));
    }
}
