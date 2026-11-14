// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The real [`Source`]: the container runtime this srv found (Docker or Podman), through the same
//! self-healing handle every container start uses.

use std::sync::Arc;

use futures_util::stream::BoxStream;

use super::{DockerEvent, Listed, Probe, Source};
use crate::backend::container::ContainerRuntimeHandle;

pub struct DockerSource {
    handle: Arc<ContainerRuntimeHandle>,
}

impl DockerSource {
    pub fn new(handle: Arc<ContainerRuntimeHandle>) -> Self {
        Self { handle }
    }
}

impl Source for DockerSource {
    async fn ping(&self) -> Probe {
        probe_of(&self.handle).await
    }

    async fn list(&self) -> Result<Vec<Listed>, String> {
        let manager = self.handle.get().await.ok_or("no container runtime")?;
        manager.agent_containers().await.map_err(|e| e.to_string())
    }

    async fn events(&self) -> Result<BoxStream<'static, Result<DockerEvent, String>>, String> {
        let manager = self.handle.get().await.ok_or("no container runtime")?;
        Ok(manager.agent_container_events())
    }
}

/// A live look at the runtime: the fields `containerruntimeavailable` answers.
pub async fn probe_of(handle: &ContainerRuntimeHandle) -> Probe {
    let available = handle.is_available().await;
    Probe {
        available,
        runtime: if available { handle.runtime_label().await } else { None },
        detail: if available { None } else { handle.missing_message() },
    }
}
