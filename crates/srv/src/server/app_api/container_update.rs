//! A container agent whose container runs an older agent image than this AgentMux: telling its
//! pane once, and updating the container when asked (a button in the pane, or `Agents
//! update_container`). See `backend::container::drift` for what an update does.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, OnceLock};

use crate::backend::cli_notice;
use crate::backend::container::ContainerManager;
use crate::backend::container_image::{current_image_for, is_agent_base_image, resolve_container_image};
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

/// Whether this AgentMux's own agent image of the kind `configured` (its stored `agent:container_image`) can be
/// had: on this machine, or from the registry. When it can't (a build ahead of its release), a container is on
/// the floating tag because nothing newer exists.
async fn current_image_available(cm: &ContainerManager, configured: &str) -> bool {
    let image = current_image_for(&resolve_container_image(configured));
    cm.has_image_locally(&image).await
        || crate::backend::registry_probe::probe_anonymous(&image).await == crate::backend::rpc_types::ContainerImageAccess::Public
}

/// [`current_image_available`] for the check made on every turn: a registry answer is kept for a
/// while, so a slow or unreachable registry does not delay each message.
async fn current_image_available_cached(cm: &ContainerManager, configured: &str) -> bool {
    // By the image asked about: the base and the developer variant are two registry answers.
    static SEEN: OnceLock<Mutex<std::collections::HashMap<String, (std::time::Instant, bool)>>> = OnceLock::new();
    const KEEP: std::time::Duration = std::time::Duration::from_secs(600);
    let seen = SEEN.get_or_init(Default::default);
    let key = current_image_for(&resolve_container_image(configured));
    let kept = seen.lock().unwrap().get(&key).copied();
    if let Some((at, available)) = kept {
        if at.elapsed() < KEEP {
            return available;
        }
    }
    let available = current_image_available(cm, configured).await;
    seen.lock().unwrap().insert(key, (std::time::Instant::now(), available));
    available
}

/// Writes one pane row, and one log line, when an existing container runs another agent image
/// version than this AgentMux.
pub(crate) async fn note_outdated(
    cm: &ContainerManager,
    container_name: &str,
    configured_image: &str,
    provider: &str,
    block_id: &str,
    broker: &Arc<crate::backend::mps::Broker>,
    filestore: &Arc<crate::backend::storage::filestore::FileStore>,
    mstore: &Arc<crate::backend::storage::store::Store>,
) {
    let Some(running) = cm.base_image_version(container_name).await else { return };
    if running == CURRENT || already_told(container_name, &running) || !current_image_available_cached(cm, configured_image).await {
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
    if image != current_image_for(&image) {
        return Some(format!("this agent is pinned to {image}; change its container image to update it"));
    }
    None
}

/// Whether version `running` is older than `current`, comparing their numeric dotted parts (`0.59.41` < `0.60.2`).
/// A version that does not parse is not older: it is left alone rather than replaced.
fn is_older(running: &str, current: &str) -> bool {
    fn parts(v: &str) -> Option<Vec<u64>> {
        let core = v.trim().trim_start_matches('v').split(['-', '+']).next()?;
        core.split('.').map(|p| p.parse::<u64>().ok()).collect()
    }
    match (parts(running), parts(current)) {
        (Some(r), Some(c)) => r < c,
        _ => false,
    }
}

/// The container's update gate taken exclusively, or None while a turn (or another update) holds it.
fn take_update_gate(container_name: &str) -> Option<tokio::sync::OwnedRwLockWriteGuard<()>> {
    crate::backend::container::drift::update_gate(container_name).try_write_owned().ok()
}

/// Moves an idle agent's container to this AgentMux's agent image: when it runs an older one, the agent follows
/// the default image (no pin, no image of its own), the current image is already on this machine, and no turn is
/// using the container. The container is removed (workspace, memory and the CLI volume stay; tools installed inside it are
/// installed again) and the pane is told; the start that follows creates it from the current image. Returns whether
/// it did. Anything else leaves the "outdated" row and its button as they are.
///
/// Run where nothing is mid-turn (the pre-start): a turn holds the update gate shared for its whole length, so
/// called from inside a turn this finds the gate taken and does nothing.
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
    // Taken first: a turn holds the gate shared, and then nothing below is worth asking the daemon.
    let Some(_updating) = take_update_gate(container_name) else { return false };
    let Some(running) = cm.base_image_version(container_name).await else { return false };
    // Only forward: after a downgrade the container is on a NEWER image, which a banner reports but an update would
    // replace with an older one. And only to an image already on this machine: the container is removed before the
    // start pulls anything, so a download that then failed (offline, a slow registry) would leave an agent that
    // used to run with nothing to run. When the image is not here yet, the banner and its button stay.
    if !is_older(&running, CURRENT) || !cm.has_image_locally(&current_image_for(&resolve_container_image(configured_image))).await {
        return false;
    }
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
    let configured = meta_get_string(&block.meta, "agent:container_image", "");
    if let Some(why) = image_blocks_update(&configured) {
        return Err(why);
    }
    let agent_id = meta_get_string(&block.meta, "agentId", "");
    let container_name = crate::backend::container::container_name_for_slug(&agent_id);
    let cm = state.container_manager.get().await.ok_or("Docker is not available on this machine")?;
    if !current_image_available(&cm, &configured).await {
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
    fn a_turn_holding_the_gate_stops_the_update_and_an_idle_agent_does_not() {
        let name = "agentmux-test-gate-for-update-if-idle";
        let turn = crate::backend::container::drift::update_gate(name).try_read_owned().expect("a turn takes the gate shared");
        assert!(take_update_gate(name).is_none(), "an update must not start under a turn");
        drop(turn);
        let updating = take_update_gate(name).expect("no turn: the update takes the gate");
        assert!(
            crate::backend::container::drift::update_gate(name).try_read_owned().is_err(),
            "a turn must not start inside an update"
        );
        drop(updating);
    }

    #[test]
    fn only_an_older_image_is_moved_forward() {
        assert!(is_older("0.59.41", "0.60.2"));
        assert!(is_older("0.60.5", "0.60.6"));
        assert!(is_older("0.9.0", "0.10.0"), "numeric, not text");
        assert!(!is_older("0.60.6", "0.60.6"), "the same");
        assert!(!is_older("0.60.7", "0.60.6"), "newer: after a downgrade it is left alone");
        assert!(!is_older("latest", "0.60.6"), "unparseable: left alone");
        assert!(!is_older("", "0.60.6"));
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
    fn the_developer_variant_follows_its_own_version_and_is_pinned_like_the_base() {
        assert!(image_blocks_update("ghcr.io/agentmuxai/agent-base:developer").is_none(), ":developer follows the version, like :latest");
        assert!(image_blocks_update(&crate::backend::container_image::versioned_developer_image()).is_none());
        assert!(image_blocks_update("ghcr.io/agentmuxai/agent-base:0.59.41-developer").unwrap().contains("pinned to"), "another version's developer image");
        // The base image's current tag is not the developer image's: an agent set to one is not on the other.
        assert_eq!(current_image_for("ghcr.io/agentmuxai/agent-base:developer"), crate::backend::container_image::versioned_developer_image());
        assert_eq!(current_image_for("ghcr.io/agentmuxai/agent-base:latest"), crate::backend::container_image::versioned_agent_image());
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
