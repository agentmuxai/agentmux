// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What Docker says about a container, reduced to what [`super::Registry`] reads: the five actions that move
//! a container between states, its name and image, and whether it carries the agent label.

use bollard::models::{ContainerSummary, EventMessage, EventMessageTypeEnum};

use super::{AGENT_LABEL, NAME_PREFIX};

/// An event action that changes where a container is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Create,
    Start,
    Stop,
    Die,
    Destroy,
}

impl Action {
    /// The five actions the mapping reads. Any other (pause, exec, attach, a runtime's own) is ignored: the
    /// periodic list corrects a miss.
    fn parse(action: &str) -> Option<Self> {
        match action {
            "create" => Some(Self::Create),
            "start" => Some(Self::Start),
            "stop" => Some(Self::Stop),
            "die" => Some(Self::Die),
            "destroy" => Some(Self::Destroy),
            _ => None,
        }
    }
}

/// One event about an agent container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerEvent {
    pub action: Action,
    pub name: String,
    pub image: String,
    pub labelled: bool,
}

/// One container of Docker's list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub name: String,
    pub image: String,
    pub running: bool,
    pub labelled: bool,
}

/// The event of a container whose name starts with [`NAME_PREFIX`] and whose action is one of the five.
pub fn event_from(message: EventMessage) -> Option<DockerEvent> {
    if message.typ != Some(EventMessageTypeEnum::CONTAINER) {
        return None;
    }
    let action = Action::parse(message.action.as_deref()?)?;
    let attributes = message.actor?.attributes?;
    let name = attributes.get("name")?.trim_start_matches('/').to_string();
    name.starts_with(NAME_PREFIX).then(|| DockerEvent {
        action,
        image: attributes.get("image").cloned().unwrap_or_default(),
        labelled: attributes.contains_key(AGENT_LABEL),
        name,
    })
}

/// The container of Docker's list under a name that starts with [`NAME_PREFIX`].
pub fn listed_from(summary: ContainerSummary) -> Option<Listed> {
    let name = summary.names?.into_iter().map(|n| n.trim_start_matches('/').to_string()).find(|n| n.starts_with(NAME_PREFIX))?;
    Some(Listed {
        name,
        image: summary.image.unwrap_or_default(),
        running: matches!(summary.state.as_deref(), Some("running" | "paused" | "restarting")),
        labelled: summary.labels.is_some_and(|l| l.contains_key(AGENT_LABEL)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::models::EventActor;
    use std::collections::HashMap;

    fn message(action: &str, name: &str, extra: &[(&str, &str)]) -> EventMessage {
        let mut attributes: HashMap<String, String> = extra.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        attributes.insert("name".into(), name.into());
        attributes.insert("image".into(), "agentmux-agent:1".into());
        EventMessage {
            typ: Some(EventMessageTypeEnum::CONTAINER),
            action: Some(action.into()),
            actor: Some(EventActor { id: Some("c1".into()), attributes: Some(attributes) }),
            ..Default::default()
        }
    }

    #[test]
    fn the_five_actions_map_and_the_others_are_left_to_the_list() {
        for (word, action) in [("create", Action::Create), ("start", Action::Start), ("stop", Action::Stop), ("die", Action::Die), ("destroy", Action::Destroy)] {
            let e = event_from(message(word, "agentmux-a1", &[])).unwrap();
            assert_eq!((e.action, e.name.as_str(), e.image.as_str(), e.labelled), (action, "agentmux-a1", "agentmux-agent:1", false));
        }
        for word in ["pause", "exec_start: sh", "attach", "kill", "oom"] {
            assert_eq!(event_from(message(word, "agentmux-a1", &[])), None, "{word}");
        }
    }

    #[test]
    fn only_agent_containers_are_read() {
        assert_eq!(event_from(message("start", "postgres", &[])), None);
        assert_eq!(event_from(message("start", "amx-dev-relay", &[])), None);
        let mut volume = message("start", "agentmux-a1", &[]);
        volume.typ = Some(EventMessageTypeEnum::VOLUME);
        assert_eq!(event_from(volume), None);
        assert!(event_from(message("start", "agentmux-a1", &[(AGENT_LABEL, "a1")])).unwrap().labelled);
    }

    #[test]
    fn a_listed_container_says_whether_it_runs_and_carries_the_label() {
        let summary = |name: &str, state: &str, labelled: bool| ContainerSummary {
            names: Some(vec![format!("/{name}")]),
            image: Some("agentmux-agent:1".into()),
            state: Some(state.into()),
            labels: labelled.then(|| HashMap::from([(AGENT_LABEL.to_string(), "a1".to_string())])),
            ..Default::default()
        };
        let running = listed_from(summary("agentmux-a1", "running", true)).unwrap();
        assert_eq!((running.name.as_str(), running.running, running.labelled), ("agentmux-a1", true, true));
        assert!(!listed_from(summary("agentmux-a1", "exited", false)).unwrap().running);
        assert!(!listed_from(summary("agentmux-a1", "created", false)).unwrap().running);
        assert_eq!(listed_from(summary("nginx", "running", false)), None);
    }
}
