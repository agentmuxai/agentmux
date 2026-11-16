// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What `backend::container_status` reads from the runtime: the list of agent containers and the stream
//! of their events. Names, states and images only: nothing from inside a container.

use std::collections::HashMap;

use bollard::container::ListContainersOptions;
use bollard::system::EventsOptions;
use futures_util::stream::BoxStream;
use futures_util::StreamExt as _;

use super::{ContainerError, ContainerManager};
use crate::backend::container_status::{docker_event, DockerEvent, Listed, AGENT_LABEL, NAME_PREFIX};

/// The label a new agent container gets: the agent it belongs to, so a reader of Docker's own list can tell it
/// from any other container. `None` for a name that is not an agent container's.
pub(super) fn agent_labels(container_name: &str) -> Option<HashMap<String, String>> {
    let id = container_name.strip_prefix(NAME_PREFIX)?;
    Some(HashMap::from([(AGENT_LABEL.to_string(), id.to_string())]))
}

impl ContainerManager {
    /// Every agent container (a name under [`NAME_PREFIX`]), running or not.
    pub(crate) async fn agent_containers(&self) -> Result<Vec<Listed>, ContainerError> {
        let mut filters = HashMap::new();
        filters.insert("name", vec![NAME_PREFIX]);
        let list = self.inner.docker.list_containers(Some(ListContainersOptions { all: true, filters, ..Default::default() })).await?;
        Ok(list.into_iter().filter_map(docker_event::listed_from).collect())
    }

    /// The events that move an agent container between states, from now. Ends when the stream fails.
    pub(crate) fn agent_container_events(&self) -> BoxStream<'static, Result<DockerEvent, String>> {
        let mut filters: HashMap<String, Vec<String>> = HashMap::new();
        filters.insert("type".into(), vec!["container".into()]);
        filters.insert("event".into(), ["create", "start", "stop", "die", "destroy"].map(String::from).to_vec());
        let options = EventsOptions::<String> { since: None, until: None, filters };
        self.inner
            .docker
            .events(Some(options))
            .filter_map(|message| async move {
                match message {
                    Ok(message) => docker_event::event_from(message).map(Ok),
                    Err(error) => Some(Err(error.to_string())),
                }
            })
            .boxed()
    }
}
