// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The container image a new agent gets when it is created from a template (the new-agent form's image choice).
//! Spec: docs/specs/SPEC_CONTAINER_AGENT_DEVELOPER_IMAGE_2026_10_29.md section 4.

/// The image a new container agent runs in: the picker's choice between the two images AgentMux ships
/// (`requested`), else the template's own. Any other image is refused (an agent's own image is set by editing its
/// definition), and a native agent has none to choose.
pub(super) fn chosen_container_image(requested: Option<&str>, agent_type: &str, template_image: &str) -> Result<String, String> {
    use crate::backend::container_image::{DEFAULT_AGENT_IMAGE, DEFAULT_DEVELOPER_IMAGE};
    match requested.map(str::trim) {
        Some(image) if agent_type == "container" => {
            if image == DEFAULT_AGENT_IMAGE || image == DEFAULT_DEVELOPER_IMAGE {
                Ok(image.to_string())
            } else {
                Err(format!("container_image must be {DEFAULT_AGENT_IMAGE} or {DEFAULT_DEVELOPER_IMAGE}, not {image:?}"))
            }
        }
        _ => Ok(template_image.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::chosen_container_image;
    use crate::backend::container_image::{DEFAULT_AGENT_IMAGE, DEFAULT_DEVELOPER_IMAGE};

    #[test]
    fn a_container_agent_gets_the_image_the_picker_chose_and_nothing_else_is_accepted() {
        assert_eq!(chosen_container_image(Some(DEFAULT_DEVELOPER_IMAGE), "container", "").unwrap(), DEFAULT_DEVELOPER_IMAGE);
        assert_eq!(chosen_container_image(Some(&format!(" {DEFAULT_AGENT_IMAGE} ")), "container", "x/y:1").unwrap(), DEFAULT_AGENT_IMAGE);
        assert!(chosen_container_image(Some("my/own:1"), "container", "").unwrap_err().contains("container_image must be"));
        assert!(chosen_container_image(Some("ghcr.io/agentmuxai/agent-base:0.59.1"), "container", "").is_err(), "a pinned version is set on the definition, not here");
    }

    #[test]
    fn without_a_choice_or_without_a_container_the_template_keeps_its_image() {
        assert_eq!(chosen_container_image(None, "container", "tpl/image:2").unwrap(), "tpl/image:2");
        assert_eq!(chosen_container_image(Some(DEFAULT_DEVELOPER_IMAGE), "host", "tpl/image:2").unwrap(), "tpl/image:2", "a native agent has no image to choose");
        assert_eq!(chosen_container_image(Some("my/own:1"), "host", "").unwrap(), "", "and a native agent's request is not validated");
    }
}
