//! Live check against a real Docker daemon and ghcr.io:
//!   cargo test -p agentmux-srv --bins -- --ignored --nocapture itest_old_image
//!
//! A container made from an older agent image is seen as older, `update_container` removes it,
//! and the next start is from this AgentMux's image. Pulls two base images.

use super::*;
use crate::backend::container::ContainerMountSpec;
use crate::backend::container_image::versioned_agent_image;

#[tokio::test]
#[ignore]
async fn itest_old_image_is_noticed_and_update_container_replaces_it() {
    let cm = ContainerManager::connect().expect("connect to docker (set DOCKER_HOST)");
    cm.check_available().await.expect("docker daemon must be reachable");
    let name = "agentmux-itest-drift";
    let _ = cm.remove(name, true).await;
    let old = "ghcr.io/agentmuxai/agent-base:0.59.41";
    cm.ensure_running_observed(name, old, &[], &[], &ContainerMountSpec::default(), &|_| {}).await.expect("start on the old image");
    assert_eq!(cm.base_image_version(name).await.as_deref(), Some("0.59.41"));

    cm.update_container(name).await.expect("update_container");
    assert!(cm.base_image_version(name).await.is_none(), "the container is gone");

    let current = versioned_agent_image();
    cm.ensure_running_observed(name, &current, &[], &[], &ContainerMountSpec::default(), &|_| {}).await.expect("start on the current image");
    assert_eq!(cm.base_image_version(name).await.as_deref(), Some(env!("CARGO_PKG_VERSION")));
    let _ = cm.remove(name, true).await;
    let _ = cm.inner.docker.remove_volume(&format!("agentmux-claude-{name}"), None).await;

    // A container made from an image that isn't the base image is the user's: never reported.
    cm.ensure_running_observed(name, "nginx:alpine", &[], &[], &ContainerMountSpec::default(), &|_| {}).await.expect("start nginx");
    assert!(cm.base_image_version(name).await.is_none());
    let _ = cm.remove(name, true).await;
    let _ = cm.inner.docker.remove_volume(&format!("agentmux-claude-{name}"), None).await;
}
