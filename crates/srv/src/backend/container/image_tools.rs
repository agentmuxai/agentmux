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
                tracing::warn!(container = container_name, %image, ?missing, "the container image lacks the AgentMux tools");
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
