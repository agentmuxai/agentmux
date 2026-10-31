//! Cache volumes: directories an image asks to keep across container recreation, named in its
//! `ai.agentmux.cache-dirs` label (the developer image lists its cargo home and build output).
//! An image update removes the container and keeps volumes, so a toolchain's registry and build
//! output there survive it. Spec: docs/specs/SPEC_CONTAINER_AGENT_DEVELOPER_IMAGE_2026_10_29.md section 4.

use std::collections::HashMap;

use bollard::models::{Mount, MountTypeEnum, MountVolumeOptions};
use bollard::volume::{ListVolumesOptions, RemoveVolumeOptions};

use super::{ContainerManager, CONTAINER_CLAUDE_DIR, CONTAINER_WORKSPACE_DIR};
use crate::backend::rpc_types::ContainerCacheVolume;

/// The image label that lists directories (absolute paths, separated by `:`) to keep in per-agent volumes.
pub const CACHE_DIRS_LABEL: &str = "ai.agentmux.cache-dirs";

/// Label on every volume made for an agent's container: which container, so a sweep can find them.
pub const VOLUME_AGENT_LABEL: &str = "ai.agentmux.agent";

/// Label on a volume that holds a cache (as opposed to a CLI or Claude home), so one can be cleared on its own.
pub const VOLUME_KIND_LABEL: &str = "ai.agentmux.kind";

/// The most directories one image may ask to keep.
const MAX_CACHE_DIRS: usize = 4;

/// Whether `a` and `b` are the same directory, or one is inside the other.
fn overlaps(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim_end_matches('/'), b.trim_end_matches('/'));
    a == b || a.strip_prefix(b).is_some_and(|r| r.starts_with('/')) || b.strip_prefix(a).is_some_and(|r| r.starts_with('/'))
}

/// The directories a label asks for that can be kept: absolute, plain (no `.` or `..` parts), not inside or around
/// the workspace or Claude home and not one of the mounts already `taken`, at most [`MAX_CACHE_DIRS`], in order,
/// each once. A label comes from an image that may be the user's own, so what it asks for is checked, not trusted.
pub(super) fn parse_cache_dirs(label: &str, taken: &[String]) -> Vec<String> {
    let mut dirs: Vec<String> = Vec::new();
    for raw in label.split(':') {
        let dir = raw.trim().trim_end_matches('/');
        let plain = dir.starts_with('/') && dir.len() > 1 && dir.split('/').skip(1).all(|p| !p.is_empty() && p != "." && p != "..");
        if !plain {
            continue;
        }
        let reserved = [CONTAINER_CLAUDE_DIR, CONTAINER_WORKSPACE_DIR];
        if reserved.iter().any(|r| overlaps(dir, r)) || taken.iter().any(|t| overlaps(dir, t)) || dirs.iter().any(|d| overlaps(dir, d)) {
            continue;
        }
        dirs.push(dir.to_string());
        if dirs.len() == MAX_CACHE_DIRS {
            break;
        }
    }
    dirs
}

/// The volume that keeps `dir` for `container_name`: `agentmux-cache-<container>-<dir as words>-<hash>`. The words
/// are for a person reading `docker volume ls`; the hash of the exact container and directory is what keeps two
/// different ones (`/a/.cargo` and `/a/cargo`, `/Cache` and `/cache`, agent `a` with `/b/c` and agent `a-b` with `/c`)
/// from sharing a volume, and a label that lists its directories in another order still finds each one's data.
pub(super) fn cache_volume_name(container_name: &str, dir: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut words = String::new();
    for c in dir.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            words.push(c);
        } else if !words.ends_with('-') {
            words.push('-');
        }
    }
    let hash: String = Sha256::digest(format!("{container_name}\0{dir}")).iter().take(6).map(|b| format!("{b:02x}")).collect();
    format!("agentmux-cache-{container_name}-{}-{hash}", words.trim_matches('-'))
}

fn cache_mount(container_name: &str, dir: &str) -> Mount {
    Mount {
        target: Some(dir.to_string()),
        source: Some(cache_volume_name(container_name, dir)),
        typ: Some(MountTypeEnum::VOLUME),
        read_only: Some(false),
        volume_options: Some(MountVolumeOptions {
            labels: Some(HashMap::from([
                (VOLUME_AGENT_LABEL.to_string(), container_name.to_string()),
                (VOLUME_KIND_LABEL.to_string(), "cache".to_string()),
            ])),
            ..Default::default()
        }),
        ..Default::default()
    }
}

impl ContainerManager {
    /// The cache volumes a container about to be made from `image` gets: one per directory its label lists
    /// (see [`parse_cache_dirs`]), none for an image without the label or one that is not here to read.
    /// `taken` are the mounts it already has.
    pub(in crate::backend::container) async fn cache_mounts_for_new(&self, container_name: &str, image: &str, taken: &[Mount]) -> Vec<Mount> {
        let labels = self.docker().inspect_image(image).await.ok().and_then(|i| i.config).and_then(|c| c.labels);
        let Some(label) = labels.as_ref().and_then(|l| l.get(CACHE_DIRS_LABEL)) else { return Vec::new() };
        let taken: Vec<String> = taken.iter().filter_map(|m| m.target.clone()).collect();
        parse_cache_dirs(label, &taken).iter().map(|dir| cache_mount(container_name, dir)).collect()
    }

    /// The cache volumes of `container_name` and what each holds, largest first (Docker's own disk-usage report,
    /// which does not scan the volumes; a volume whose driver can't say is counted as 0).
    pub(crate) async fn cache_volumes(&self, container_name: &str) -> Result<Vec<ContainerCacheVolume>, String> {
        let usage = self.docker().df().await.map_err(|e| format!("Docker would not report volume sizes: {e}"))?;
        let mut out: Vec<ContainerCacheVolume> = usage
            .volumes
            .unwrap_or_default()
            .into_iter()
            .filter(|v| v.labels.get(VOLUME_AGENT_LABEL).map(String::as_str) == Some(container_name) && v.labels.get(VOLUME_KIND_LABEL).map(String::as_str) == Some("cache"))
            .map(|v| ContainerCacheVolume { bytes: v.usage_data.map_or(0, |u| u.size.max(0) as u64), name: v.name })
            .collect();
        out.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name)));
        Ok(out)
    }

    /// Removes the container and its cache volumes (the CLI and Claude home volumes stay): the next message starts a
    /// new container, which fills them again. Returns what was removed. The caller makes sure no turn is using the
    /// container.
    pub(crate) async fn clear_caches(&self, container_name: &str) -> Result<Vec<ContainerCacheVolume>, String> {
        let volumes = self.cache_volumes(container_name).await?;
        let _ = self.stop(container_name, 10).await;
        let _ = self.remove(container_name, true).await;
        let mut removed = Vec::new();
        for v in volumes {
            match self.docker().remove_volume(&v.name, Some(RemoveVolumeOptions { force: true })).await {
                Ok(()) | Err(bollard::errors::Error::DockerResponseServerError { status_code: 404, .. }) => removed.push(v),
                Err(e) => return Err(format!("couldn't remove the volume {}: {e}", v.name)),
            }
        }
        Ok(removed)
    }

    /// Removes the container and every volume made for an agent, once the agent is deleted: its cache volumes
    /// (by label), and the CLI and Claude home volumes older versions made without one. Best effort: what is
    /// not there, or is still in use, is skipped and logged.
    pub(crate) async fn remove_agent_storage(&self, container_name: &str) {
        let _ = self.stop(container_name, 10).await;
        let _ = self.remove(container_name, true).await;
        let mut names = vec![format!("agentmux-cli-{container_name}"), format!("agentmux-claude-{container_name}")];
        let filter = HashMap::from([("label".to_string(), vec![format!("{VOLUME_AGENT_LABEL}={container_name}")])]);
        match self.docker().list_volumes(Some(ListVolumesOptions { filters: filter })).await {
            Ok(list) => names.extend(list.volumes.unwrap_or_default().into_iter().map(|v| v.name)),
            Err(e) => tracing::warn!(container = container_name, error = %e, "container storage: can't list the agent's volumes"),
        }
        for name in names {
            match self.docker().remove_volume(&name, Some(RemoveVolumeOptions { force: true })).await {
                Ok(()) => tracing::info!(container = container_name, volume = %name, "removed an agent's volume"),
                Err(bollard::errors::Error::DockerResponseServerError { status_code: 404, .. }) => {}
                Err(e) => tracing::warn!(container = container_name, volume = %name, error = %e, "container storage: couldn't remove a volume"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_label_lists_plain_absolute_directories_in_order() {
        assert_eq!(parse_cache_dirs("/home/agent/.cargo:/home/agent/target", &[]), ["/home/agent/.cargo", "/home/agent/target"]);
        assert_eq!(parse_cache_dirs(" /a/b/ : /c ", &[]), ["/a/b", "/c"], "spaces and a trailing slash are dropped");
        assert!(parse_cache_dirs("", &[]).is_empty());
    }

    #[test]
    fn what_an_image_may_not_ask_for_is_skipped() {
        let taken = ["/home/agent/.agentmux/cli".to_string()];
        let label = "relative/dir:/:/a/../etc:/a/./b:/workspace:/workspace/target:/home/agent/.claude/projects:/home/agent:/home/agent/.agentmux/cli:/ok";
        assert_eq!(parse_cache_dirs(label, &taken), ["/ok"], "no relative or odd paths, nothing in or around the workspace, Claude home or a mount it has");
    }

    #[test]
    fn a_directory_is_asked_for_once_and_not_inside_another_it_asked_for() {
        assert_eq!(parse_cache_dirs("/x/cargo:/x/cargo:/x/cargo/registry:/x/other", &[]), ["/x/cargo", "/x/other"]);
        assert_eq!(parse_cache_dirs("/a:/b:/c:/d:/e:/f", &[]), ["/a", "/b", "/c", "/d"], "at most four");
    }

    #[test]
    fn different_agents_and_directories_never_share_a_volume() {
        let name = |c: &str, d: &str| cache_volume_name(c, d);
        assert_ne!(name("agentmux-a", "/home/agent/.cargo"), name("agentmux-a", "/home/agent/cargo"), "a dot is not nothing");
        assert_ne!(name("agentmux-a", "/Cache"), name("agentmux-a", "/cache"), "nor is a capital");
        assert_ne!(name("agentmux-a", "/b/c"), name("agentmux-a-b", "/c"), "nor which side of the dash a word is on");
        assert_ne!(name("agentmux-a", "/x"), name("agentmux-b", "/x"));
    }

    #[test]
    fn a_volume_is_named_by_its_agent_and_directory() {
        let cargo = cache_volume_name("agentmux-demo", "/home/agent/.cargo");
        assert!(cargo.starts_with("agentmux-cache-agentmux-demo-home-agent-cargo-"), "readable: {cargo}");
        assert_eq!(cargo, cache_volume_name("agentmux-demo", "/home/agent/.cargo"), "the same input, the same volume");
        assert!(cache_volume_name("agentmux-demo", "/home/agent/target").starts_with("agentmux-cache-agentmux-demo-home-agent-target-"));
        let m = cache_mount("agentmux-demo", "/home/agent/target");
        assert_eq!(m.typ, Some(MountTypeEnum::VOLUME));
        let labels = m.volume_options.unwrap().labels.unwrap();
        assert_eq!(labels.get(VOLUME_AGENT_LABEL).map(String::as_str), Some("agentmux-demo"));
        assert_eq!(labels.get(VOLUME_KIND_LABEL).map(String::as_str), Some("cache"));
    }
}

/// Live check against a real Docker daemon:
///   cargo test -p agentmux-srv --bins -- --ignored --nocapture itest_cache_dirs
/// An image that names cache directories gets a labelled volume for each; the data survives the container being
/// recreated (what an image update does); deleting the agent removes the container and the volumes.
#[cfg(test)]
mod itest {
    use bollard::container::{Config, CreateContainerOptions};
    use bollard::image::{CommitContainerOptions, RemoveImageOptions};
    use futures_util::StreamExt as _;

    use super::*;
    use crate::backend::container::ContainerMountSpec;

    async fn run(cm: &ContainerManager, name: &str, script: &str) -> String {
        let session = cm.exec(name, &["sh".into(), "-c".into(), script.into()], None, &[], false).await.expect("exec");
        drop(session.input);
        let mut out = String::new();
        let mut stream = session.output;
        while let Some(Ok(chunk)) = stream.next().await {
            out.push_str(&String::from_utf8_lossy(&chunk.into_bytes()));
        }
        out
    }

    /// An image that is `debian:13` with the cache-dirs label, made by committing a container (no docker CLI).
    async fn labelled_image(cm: &ContainerManager, image: &str) {
        let maker = "agentmux-itest-cache-maker";
        let _ = cm.docker().remove_container(maker, Some(bollard::container::RemoveContainerOptions { force: true, ..Default::default() })).await;
        cm.docker()
            .create_container(Some(CreateContainerOptions { name: maker, platform: None }), Config { image: Some("debian:13".to_string()), ..Default::default() })
            .await
            .expect("create the container to commit (debian:13 must be pulled)");
        let labels = HashMap::from([(CACHE_DIRS_LABEL.to_string(), "/home/agent/.cargo:/home/agent/target".to_string())]);
        let (repo, tag) = image.split_once(':').unwrap();
        cm.docker()
            .commit_container(
                CommitContainerOptions { container: maker.to_string(), repo: repo.to_string(), tag: tag.to_string(), ..Default::default() },
                Config::<String> { labels: Some(labels), cmd: Some(vec!["sleep".into(), "infinity".into()]), ..Default::default() },
            )
            .await
            .expect("commit");
        let _ = cm.docker().remove_container(maker, Some(bollard::container::RemoveContainerOptions { force: true, ..Default::default() })).await;
    }

    #[tokio::test]
    #[ignore]
    async fn itest_cache_dirs_survive_a_recreate_and_go_with_the_agent() {
        let cm = ContainerManager::connect().expect("connect to docker (set DOCKER_HOST)");
        cm.check_available().await.expect("docker daemon must be reachable");
        let image = "agentmux-cache-itest:1";
        labelled_image(&cm, image).await;

        let name = "agentmux-itest-cache";
        cm.remove_agent_storage(name).await;
        let spec = ContainerMountSpec::default();
        cm.ensure_running_observed(name, image, &[], &[], &spec, &|_| {}).await.expect("ensure_running");
        let first = run(&cm, name, "echo built > /home/agent/target/artifact && echo registry > /home/agent/.cargo/crate && echo ok").await;
        // What an image update does: the container goes, the volumes stay.
        cm.update_container(name).await.expect("remove the container");
        cm.ensure_running_observed(name, image, &[], &[], &spec, &|_| {}).await.expect("ensure_running again");
        let kept = run(&cm, name, "cat /home/agent/target/artifact /home/agent/.cargo/crate").await;

        let filter = HashMap::from([("label".to_string(), vec![format!("{VOLUME_AGENT_LABEL}={name}")])]);
        let labelled: Vec<String> = cm
            .docker()
            .list_volumes(Some(ListVolumesOptions { filters: filter.clone() }))
            .await
            .unwrap()
            .volumes
            .unwrap_or_default()
            .into_iter()
            .map(|v| v.name)
            .collect();

        // Their sizes as Docker reports them, and clearing them (the container goes with them).
        let sized = cm.cache_volumes(name).await.expect("volume sizes");
        let freed = cm.clear_caches(name).await.expect("clear");
        let after_clear = cm.cache_volumes(name).await.expect("volume sizes after clearing");
        let container_gone_after_clear = cm.docker().inspect_container(name, None).await.is_err();

        cm.remove_agent_storage(name).await;
        let left = cm.docker().list_volumes(Some(ListVolumesOptions { filters: filter })).await.unwrap().volumes.unwrap_or_default().len();
        let container_gone = cm.docker().inspect_container(name, None).await.is_err();
        let _ = cm.docker().remove_image(image, Some(RemoveImageOptions { force: true, ..Default::default() }), None).await;

        assert!(first.contains("ok"), "the agent can write into both cache directories: {first}");
        assert_eq!(kept.trim(), "built\nregistry", "the data is there after the container was recreated: {kept:?}");
        let mut labelled = labelled;
        labelled.sort();
        let mut want = [cache_volume_name(name, "/home/agent/.cargo"), cache_volume_name(name, "/home/agent/target")];
        want.sort();
        assert_eq!(labelled, want, "one labelled volume per directory");
        assert_eq!(sized.len(), 2, "both caches are reported: {sized:?}");
        assert!(sized.iter().all(|v| v.bytes > 0), "each holds what was written (Docker's own count): {sized:?}");
        assert_eq!(freed.len(), 2, "clearing removed both");
        assert!(after_clear.is_empty() && container_gone_after_clear, "and left nothing behind, the container included: {after_clear:?}");
        assert_eq!(left, 0, "the agent's volumes are removed with it");
        assert!(container_gone, "and its container");
    }
}
