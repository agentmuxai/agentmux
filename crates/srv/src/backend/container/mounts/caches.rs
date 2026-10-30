//! Cache volumes: directories an image asks to keep across container recreation, named in its
//! `ai.agentmux.cache-dirs` label (the developer image lists its cargo home and build output).
//! An image update removes the container and keeps volumes, so a toolchain's registry and build
//! output there survive it. Spec: docs/specs/SPEC_CONTAINER_AGENT_DEVELOPER_IMAGE_2026_10_29.md section 4.

use std::collections::HashMap;

use bollard::models::{Mount, MountTypeEnum, MountVolumeOptions};
use bollard::volume::{ListVolumesOptions, RemoveVolumeOptions};

use super::{ContainerManager, CONTAINER_CLAUDE_DIR, CONTAINER_WORKSPACE_DIR};

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

/// The volume that keeps `dir` for `container_name`: `agentmux-cache-<container>-<dir as words>`, so a label that
/// lists its directories in another order still finds each one's data.
pub(super) fn cache_volume_name(container_name: &str, dir: &str) -> String {
    let mut words = String::new();
    for c in dir.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            words.push(c);
        } else if !words.ends_with('-') {
            words.push('-');
        }
    }
    format!("agentmux-cache-{container_name}-{}", words.trim_matches('-'))
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
    fn a_volume_is_named_by_its_agent_and_directory() {
        assert_eq!(cache_volume_name("agentmux-demo", "/home/agent/.cargo"), "agentmux-cache-agentmux-demo-home-agent-cargo");
        assert_eq!(cache_volume_name("agentmux-demo", "/home/agent/target"), "agentmux-cache-agentmux-demo-home-agent-target");
        let m = cache_mount("agentmux-demo", "/home/agent/target");
        assert_eq!(m.typ, Some(MountTypeEnum::VOLUME));
        let labels = m.volume_options.unwrap().labels.unwrap();
        assert_eq!(labels.get(VOLUME_AGENT_LABEL).map(String::as_str), Some("agentmux-demo"));
        assert_eq!(labels.get(VOLUME_KIND_LABEL).map(String::as_str), Some("cache"));
    }
}
