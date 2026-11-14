// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `GET /api/v1/containers`: this machine's agent containers and the container runtime's state, as names and
//! states only (`backend::container_status`). What is there now: a removed container is only ever an event.
//! Read-only, and it reaches nothing inside a container.
//!
//! The runtime is looked at live, as `containerruntimeavailable` does, so a reader finds a runtime that started
//! after srv did and wakes the watcher. Where there is no container runtime this answers 200 with an empty
//! list and `runtime.available: false`, never an error.

use axum::extract::State;
use axum::response::{IntoResponse, Json};

use crate::backend::container_status::{self, ContainersList, DockerSource, Source as _};
use crate::server::AppState;

pub async fn handle_containers(State(state): State<AppState>) -> impl IntoResponse {
    let Some(hub) = container_status::hub() else {
        return Json(ContainersList { runtime: container_status::ContainerRuntimeStatus::absent(), containers: Vec::new() });
    };
    let probe = DockerSource::new(state.container_manager.clone()).ping().await;
    let available = probe.available;
    hub.on_runtime(probe);
    if available {
        hub.wake();
    }
    Json(hub.list())
}
