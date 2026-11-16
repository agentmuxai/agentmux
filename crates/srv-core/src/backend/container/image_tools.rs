// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The check that a container's image has what an AgentMux agent needs to talk to AgentMux (#47725).

use super::{ContainerError, ContainerManager};

/// What an agent's container needs from its image to be an AgentMux agent at all.
const IMAGE_TOOLS: [&str; 2] = ["agentmux-bashwrap", "agentmux-mcp"];

/// What the check prints: `missing:<tool>` per absent tool, then `checked` (so an exec that never ran the
/// script is not read as "all present").
const IMAGE_TOOLS_CHECK: &str = r#"for t in "$@"; do command -v "$t" >/dev/null 2>&1 || echo "missing:$t"; done; echo checked"#;

/// The tools [`IMAGE_TOOLS_CHECK`] reported absent. `None` when the script's own `checked` line is missing: the
/// probe did not run, which says nothing about the image.
fn missing_image_tools(output: &str) -> Option<Vec<String>> {
    if !output.lines().any(|l| l.trim() == "checked") {
        return None;
    }
    Some(output.lines().filter_map(|l| l.trim().strip_prefix("missing:")).map(str::to_string).collect())
}

/// Whether the existing container was made from the retired image while `requested` (what this start asks for)
/// is not it. Such a container is recreated, not reused: it has none of the AgentMux tools, and reusing it would
/// refuse every start with an image the definition no longer names (#47725). A container on any other image,
/// an older base image included, is left alone: moving it is `update_container`'s, so tools installed in it stay.
pub(super) fn made_from_retired_image(details: &bollard::models::ContainerInspectResponse, requested: &str) -> bool {
    let actual = details.config.as_ref().and_then(|c| c.image.as_deref()).unwrap_or("");
    super::image::is_legacy_agent_image(actual) && !super::image::is_legacy_agent_image(requested)
}

impl ContainerManager {
    /// Whether the running container `container_name`, made from `image`, has what an AgentMux agent needs
    /// ([`IMAGE_TOOLS`]). An image that lacks them makes a container that starts and cannot talk (no tools, no
    /// startup memory, no messages), so this is asked before the agent's first turn rather than answered by a
    /// run of failed hooks. A probe that can't run says nothing about the image and lets the start go on.
    /// Checked once per container: a removed container forgets it (`forget_cli_ready`).
    pub async fn check_image_tools(&self, container_name: &str, image: &str) -> Result<(), ContainerError> {
        let key = format!("{container_name}\u{0}tools\u{0}{image}");
        if self.inner.cli_ready.lock().await.contains(&key) {
            return Ok(());
        }
        let mut argv: Vec<String> = vec!["sh".into(), "-c".into(), IMAGE_TOOLS_CHECK.into(), "agentmux-image-check".into()];
        argv.extend(IMAGE_TOOLS.iter().map(|t| t.to_string()));
        let outcome = match self.exec_collect(container_name, &argv, std::time::Duration::from_secs(20)).await {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!(container = container_name, %image, error = %e, "could not check the image's AgentMux tools; starting anyway");
                return Ok(());
            }
        };
        match missing_image_tools(&outcome.output) {
            Some(missing) if !missing.is_empty() => {
                tracing::warn!(container = container_name, %image, ?missing, "the container image lacks the AgentMux tools; stopping it");
                // Left running it would sit there idle and unusable; the next start asks again.
                let _ = self.stop(container_name, 10).await;
                Err(ContainerError::ImageIncomplete { image: image.to_string(), missing })
            }
            Some(_) => {
                self.inner.cli_ready.lock().await.insert(key);
                Ok(())
            }
            None => {
                tracing::warn!(container = container_name, %image, "the image check printed nothing usable; starting anyway");
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_image_check_reads_what_the_script_printed_and_nothing_else() {
        assert_eq!(missing_image_tools("checked\n"), Some(vec![]), "both present");
        assert_eq!(
            missing_image_tools("missing:agentmux-bashwrap\nmissing:agentmux-mcp\nchecked\n"),
            Some(vec!["agentmux-bashwrap".to_string(), "agentmux-mcp".to_string()])
        );
        assert_eq!(missing_image_tools("missing:agentmux-mcp\r\nchecked\r\n"), Some(vec!["agentmux-mcp".to_string()]));
        // The script never ran to its end (the exec failed, or `sh` is missing): that says nothing about the image.
        assert_eq!(missing_image_tools(""), None);
        assert_eq!(missing_image_tools("sh: not found\n"), None);
        assert_eq!(missing_image_tools("missing:agentmux-mcp\n"), None);
    }

    #[test]
    fn a_container_made_from_the_retired_image_is_recreated_and_no_other_is() {
        let made_from = |image: &str| bollard::models::ContainerInspectResponse {
            config: Some(bollard::models::ContainerConfig { image: Some(image.to_string()), ..Default::default() }),
            ..Default::default()
        };
        let base = "ghcr.io/agentmuxai/agent-base:0.60.29";
        assert!(made_from_retired_image(&made_from("ghcr.io/agentmuxai/agent-claude:latest"), base));
        assert!(made_from_retired_image(&made_from("ghcr.io/agentmuxai/agent-claude@sha256:abc"), base));
        assert!(!made_from_retired_image(&made_from("ghcr.io/agentmuxai/agent-base:0.60.1"), base), "an older base image is update_container's");
        assert!(!made_from_retired_image(&made_from("my/own:1"), base));
        assert!(!made_from_retired_image(&made_from("ghcr.io/agentmuxai/agent-claude:latest"), "ghcr.io/agentmuxai/agent-claude:latest"), "unless the retired image is what is asked for");
        assert!(!made_from_retired_image(&bollard::models::ContainerInspectResponse::default(), base), "no inspect data: leave it");
    }

    #[test]
    fn an_incomplete_image_says_which_image_and_what_to_do() {
        let msg = ContainerError::ImageIncomplete {
            image: "ghcr.io/agentmuxai/agent-claude:latest".into(),
            missing: vec!["agentmux-bashwrap".into(), "agentmux-mcp".into()],
        }
        .user_message();
        assert!(msg.contains("ghcr.io/agentmuxai/agent-claude:latest"), "{msg}");
        assert!(msg.contains("agentmux-bashwrap or agentmux-mcp"), "{msg}");
        assert!(msg.contains("update_container") && msg.contains("agent-base"), "{msg}");
    }
}
