//! A container made from an older agent image than this AgentMux: noticing it, and updating it.
//!
//! The release workflow tags `agent-base` with the AgentMux version (also on the image's
//! `org.opencontainers.image.version` label), so the `agentmux-mcp` and `agentmux-bashwrap`
//! inside match one srv. A container keeps the image it was created from, so after an upgrade
//! it can run an older one; it is not recreated on its own, because tools installed inside it
//! would be lost. [`ContainerManager::update_container`] does that, on request.

use super::{ContainerError, ContainerManager};
use crate::backend::container_image::is_agent_base_image;

/// Held shared by every turn that runs in a container, and exclusively while the container is
/// being updated: a turn cannot start inside an update, and an update cannot start under a turn.
pub(crate) fn update_gate(container_name: &str) -> std::sync::Arc<tokio::sync::RwLock<()>> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    static GATES: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::RwLock<()>>>>> = OnceLock::new();
    GATES.get_or_init(Default::default).lock().unwrap().entry(container_name.to_string()).or_default().clone()
}

/// The label the release workflow's metadata step puts the version in.
const VERSION_LABEL: &str = "org.opencontainers.image.version";

/// The version an image reference or its labels name: the label, else a numeric tag.
fn version_of(label: Option<&str>, image_ref: &str) -> Option<String> {
    let label = label.map(str::trim).filter(|v| !v.is_empty());
    if let Some(v) = label {
        return Some(v.to_string());
    }
    let tag = image_ref.rsplit_once(':')?.1;
    // The developer variant's tag is `<version>-developer`: the version is what the pane compares.
    let tag = tag.strip_suffix("-developer").unwrap_or(tag);
    tag.starts_with(|c: char| c.is_ascii_digit()).then(|| tag.to_string())
}

impl ContainerManager {
    /// The AgentMux version of the base image `container_name` was created from, or `None`
    /// when it isn't the base image (a custom image is the user's), or can't be read.
    pub(crate) async fn base_image_version(&self, container_name: &str) -> Option<String> {
        let details = self.inner.docker.inspect_container(container_name, None).await.ok()?;
        let image_ref = details.config.as_ref()?.image.clone()?;
        if !is_agent_base_image(&image_ref) {
            return None;
        }
        let labels = match details.image.as_deref() {
            Some(id) => self.inner.docker.inspect_image(id).await.ok().and_then(|i| i.config?.labels),
            None => None,
        };
        version_of(labels.as_ref().and_then(|l| l.get(VERSION_LABEL)).map(String::as_str), &image_ref)
    }

    /// Removes the container so the next turn creates it again from the current image. The
    /// per-agent volumes (CLI, Claude home) and the bind-mounted workspace and memory stay;
    /// whatever else was installed inside the container is gone.
    pub(crate) async fn update_container(&self, container_name: &str) -> Result<(), ContainerError> {
        let _ = self.stop(container_name, 10).await;
        self.remove(container_name, true).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_label_wins_and_a_numeric_tag_is_the_fallback() {
        assert_eq!(version_of(Some("0.60.2"), "ghcr.io/agentmuxai/agent-base:latest").as_deref(), Some("0.60.2"));
        assert_eq!(version_of(Some("  "), "ghcr.io/agentmuxai/agent-base:0.59.41").as_deref(), Some("0.59.41"));
        assert_eq!(version_of(None, "ghcr.io/agentmuxai/agent-base:0.60.7-developer").as_deref(), Some("0.60.7"), "the developer variant's tag carries the version");
        assert_eq!(version_of(None, "ghcr.io/agentmuxai/agent-base:latest"), None);
        assert_eq!(version_of(None, "ghcr.io/agentmuxai/agent-base:developer"), None);
        assert_eq!(version_of(None, "ghcr.io/agentmuxai/agent-base"), None);
    }
}

#[cfg(test)]
mod itest;
