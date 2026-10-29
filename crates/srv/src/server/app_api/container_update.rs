//! A container agent whose container runs an older agent image than this AgentMux: telling its
//! pane once, and updating the container when asked (a button in the pane, or `Agents
//! update_container`). See `backend::container::drift` for what an update does.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock};

use crate::backend::cli_notice;
use crate::backend::container::ContainerManager;
use crate::backend::container_image::{is_agent_base_image, resolve_container_image, versioned_agent_image};
use crate::server::AppState;

/// The version this AgentMux ships the agent image for.
const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// Containers (by name and running version) whose pane has been told, so one srv run says it
/// once per container.
fn told() -> std::sync::MutexGuard<'static, HashSet<String>> {
    static TOLD: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    TOLD.get_or_init(Default::default).lock().unwrap()
}

fn already_told(container_name: &str, running: &str) -> bool {
    told().contains(&format!("{container_name}@{running}"))
}

/// Whether the container named `container_name` runs another image version than `current`, and has
/// not been reported yet. Marks it reported.
fn first_report(container_name: &str, running: &str, current: &str) -> bool {
    running != current && told().insert(format!("{container_name}@{running}"))
}

/// Whether this AgentMux's own agent image can be had: on this machine, or from the registry. When it
/// can't (a build ahead of its release), a container is on `:latest` because nothing newer exists.
async fn current_image_available(cm: &ContainerManager) -> bool {
    let image = versioned_agent_image();
    cm.has_image_locally(&image).await
        || crate::backend::registry_probe::probe_anonymous(&image).await == crate::backend::rpc_types::ContainerImageAccess::Public
}

/// [`current_image_available`] for the check made on every turn: a registry answer is kept for a
/// while, so a slow or unreachable registry does not delay each message.
async fn current_image_available_cached(cm: &ContainerManager) -> bool {
    static SEEN: OnceLock<Mutex<Option<(std::time::Instant, bool)>>> = OnceLock::new();
    const KEEP: std::time::Duration = std::time::Duration::from_secs(600);
    let seen = SEEN.get_or_init(Default::default);
    if let Some((at, available)) = *seen.lock().unwrap() {
        if at.elapsed() < KEEP {
            return available;
        }
    }
    let available = current_image_available(cm).await;
    *seen.lock().unwrap() = Some((std::time::Instant::now(), available));
    available
}

/// Writes one pane row, and one log line, when an existing container runs another agent image
/// version than this AgentMux.
pub(crate) async fn note_outdated(
    cm: &ContainerManager,
    container_name: &str,
    provider: &str,
    block_id: &str,
    broker: &Arc<crate::backend::mps::Broker>,
    filestore: &Arc<crate::backend::storage::filestore::FileStore>,
    mstore: &Arc<crate::backend::storage::store::Store>,
) {
    let Some(running) = cm.base_image_version(container_name).await else { return };
    if running == CURRENT || already_told(container_name, &running) || !current_image_available_cached(cm).await {
        return;
    }
    if !first_report(container_name, &running, CURRENT) {
        return;
    }
    tracing::info!(container = container_name, running = %running, current = CURRENT, "container runs an older agent image than this AgentMux");
    let frame = cli_notice::container_image_frame(provider, block_id, "outdated", &running, CURRENT);
    cli_notice::append_to_pane(broker, filestore, mstore, block_id, &frame);
}

/// Why an agent configured with `configured` (its stored `agent:container_image`, empty for the default) can't be
/// moved to this AgentMux's agent image: it runs its own image, or is pinned to another version.
fn image_blocks_update(configured: &str) -> Option<String> {
    let image = resolve_container_image(configured);
    if !is_agent_base_image(&image) {
        return Some(format!("this agent runs its own image ({image}); updating the container would start it from the same image"));
    }
    // `resolve_container_image` turns the default and `:latest` into this version's tag, so any
    // other reference (another version, a digest, another tag) is a pin the update would keep.
    if image != versioned_agent_image() {
        return Some(format!("this agent is pinned to {image}; change its container image to update it"));
    }
    None
}

/// Moves an idle agent's container to this AgentMux's agent image: when it runs an older one, the agent follows
/// the default image (no pin, no image of its own), the current image can be had, and no turn is using the
/// container. The container is removed (workspace, memory and the CLI volume stay; tools installed inside it are
/// installed again) and the pane is told; the start that follows creates it from the current image. Returns whether
/// it did. Anything else leaves the "outdated" row and its button as they are.
///
/// Run where nothing is mid-turn (the pre-start): a turn holds the update gate shared for its whole length, so
/// there this finds the gate taken and does nothing.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn update_if_idle(
    cm: &ContainerManager,
    container_name: &str,
    configured_image: &str,
    provider: &str,
    block_id: &str,
    broker: &Arc<crate::backend::mps::Broker>,
    filestore: &Arc<crate::backend::storage::filestore::FileStore>,
    mstore: &Arc<crate::backend::storage::store::Store>,
) -> bool {
    if image_blocks_update(configured_image).is_some() {
        return false;
    }
    let Some(running) = cm.base_image_version(container_name).await else { return false };
    if running == CURRENT || !current_image_available_cached(cm).await {
        return false;
    }
    let gate = crate::backend::container::drift::update_gate(container_name);
    let Ok(_updating) = gate.try_write_owned() else { return false };
    if let Err(e) = cm.update_container(container_name).await {
        tracing::warn!(container = container_name, error = %e.user_message(), "could not move an idle container to the current agent image");
        return false;
    }
    told().retain(|k| !k.starts_with(&format!("{container_name}@")));
    let frame = cli_notice::container_image_frame(provider, block_id, "auto_updated", &running, CURRENT);
    cli_notice::append_to_pane(broker, filestore, mstore, block_id, &frame);
    tracing::info!(block_id, container = container_name, from = %running, to = CURRENT, "idle container moved to the current agent image");
    true
}

/// Removes `block_id`'s container so its next message starts it from this AgentMux's agent
/// image. Only between turns. Returns the image version the next start uses.
pub(crate) async fn update_for_block(state: &AppState, block_id: &str) -> Result<String, String> {
    use crate::backend::obj::{meta_get_string, Block};
    let block = state.mstore.get::<Block>(block_id).ok().flatten().ok_or("no such pane")?;
    if meta_get_string(&block.meta, "agentMode", "host") != "container" {
        return Err("this agent does not run in a container".into());
    }
    if let Some(why) = image_blocks_update(&meta_get_string(&block.meta, "agent:container_image", "")) {
        return Err(why);
    }
    let agent_id = meta_get_string(&block.meta, "agentId", "");
    let container_name = crate::backend::container::container_name_for_slug(&agent_id);
    let cm = state.container_manager.get().await.ok_or("Docker is not available on this machine")?;
    if !current_image_available(&cm).await {
        return Err(format!("the agent image for AgentMux {CURRENT} is not available yet, so a new container would start from the same image"));
    }
    // Every turn of this agent holds the gate shared, and one that starts meanwhile is turned away.
    let gate = crate::backend::container::drift::update_gate(&container_name);
    let _updating = gate.try_write_owned().map_err(|_| "a turn is running; update the container when it has finished")?;
    cm.update_container(&container_name).await.map_err(|e| e.user_message())?;
    told().retain(|k| !k.starts_with(&format!("{container_name}@")));
    let provider = meta_get_string(&block.meta, "agentProvider", "claude");
    let frame = cli_notice::container_image_frame(&provider, block_id, "updated", "", CURRENT);
    cli_notice::append_to_pane(&state.broker, &state.filestore, &state.mstore, block_id, &frame);
    tracing::info!(block_id, container = %container_name, "container removed; the next message starts it from the current agent image");
    Ok(CURRENT.to_string())
}

/// `POST /api/v1/agent/container/update`: `Agents update_container`, for the agent named in
/// `agent`.
#[derive(serde::Deserialize)]
pub(crate) struct UpdateContainerRequest {
    agent: String,
}

pub(crate) async fn handle_update_container(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::Json(req): axum::Json<UpdateContainerRequest>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let target = req.agent.trim();
    let registration = crate::backend::reactive::handler::get_global_handler().get_agent(target);
    let Some(registration) = registration else {
        let body = serde_json::json!({ "error": format!("no agent named {target} is open here") });
        return (axum::http::StatusCode::NOT_FOUND, axum::Json(body)).into_response();
    };
    match update_for_block(&state, &registration.block_id).await {
        Ok(version) => axum::Json(serde_json::json!({ "status": "updated", "next_start_uses": version })).into_response(),
        Err(why) => (axum::http::StatusCode::CONFLICT, axum::Json(serde_json::json!({ "error": why }))).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded_block(state: &AppState) -> String {
        let tab = state.mstore.get_all::<crate::backend::obj::Tab>().unwrap().into_iter().next().expect("seeded tab");
        tab.blockids.first().expect("seeded agent block").clone()
    }

    fn set_meta(state: &AppState, block_id: &str, pairs: &[(&str, &str)]) {
        let mut meta = crate::backend::obj::MetaMapType::new();
        for (k, v) in pairs {
            meta.insert((*k).to_string(), serde_json::json!(v));
        }
        crate::server::service::object_helpers::update_object_meta(&state.mstore, &format!("block:{block_id}"), &meta).unwrap();
    }

    #[tokio::test]
    async fn only_a_container_agent_on_the_current_image_is_updated() {
        let state = crate::server::tests::test_state();
        let block_id = seeded_block(&state);
        set_meta(&state, &block_id, &[("agentMode", "host")]);
        let err = update_for_block(&state, &block_id).await.unwrap_err();
        assert!(err.contains("does not run in a container"), "{err}");

        // An agent on its own image is told: updating would start it from the same image.
        set_meta(&state, &block_id, &[("agentMode", "container"), ("agent:container_image", "my/own:1")]);
        let err = update_for_block(&state, &block_id).await.unwrap_err();
        assert!(err.contains("runs its own image (my/own:1)"), "{err}");

        // An agent pinned to another version of the image is told, not silently left or moved.
        set_meta(&state, &block_id, &[("agentMode", "container"), ("agent:container_image", "ghcr.io/agentmuxai/agent-base:0.59.41")]);
        let err = update_for_block(&state, &block_id).await.unwrap_err();
        assert!(err.contains("pinned to ghcr.io/agentmuxai/agent-base:0.59.41"), "{err}");

        // So is one pinned by digest: the next start would use the same image.
        set_meta(&state, &block_id, &[("agentMode", "container"), ("agent:container_image", "ghcr.io/agentmuxai/agent-base@sha256:abc")]);
        let err = update_for_block(&state, &block_id).await.unwrap_err();
        assert!(err.contains("pinned to ghcr.io/agentmuxai/agent-base@sha256:abc"), "{err}");

        assert!(update_for_block(&state, "no-such-block").await.unwrap_err().contains("no such pane"));
    }

    #[test]
    fn only_an_agent_on_the_default_image_follows_it() {
        assert!(image_blocks_update("").is_none(), "the default image");
        assert!(image_blocks_update("ghcr.io/agentmuxai/agent-base:latest").is_none(), ":latest follows the version");
        assert!(image_blocks_update("my/own:1").unwrap().contains("runs its own image (my/own:1)"));
        assert!(image_blocks_update("ghcr.io/agentmuxai/agent-base:0.59.41").unwrap().contains("pinned to"));
        assert!(image_blocks_update("ghcr.io/agentmuxai/agent-base@sha256:abc").unwrap().contains("pinned to"));
    }

    #[test]
    fn a_container_is_reported_once_per_running_version_and_never_when_current() {
        assert!(!first_report("agentmux-t1", "0.60.2", "0.60.2"));
        assert!(first_report("agentmux-t1", "0.59.41", "0.60.2"));
        assert!(!first_report("agentmux-t1", "0.59.41", "0.60.2"), "once");
        assert!(first_report("agentmux-t1", "0.59.40", "0.60.2"), "another running version is news");
        assert!(first_report("agentmux-t2", "0.59.41", "0.60.2"), "another container");
    }
}
