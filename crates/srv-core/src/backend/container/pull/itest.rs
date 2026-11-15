//! Live check of the version-pinned base image against a real Docker daemon and ghcr.io:
//!   cargo test -p agentmux-srv --bins -- --ignored --nocapture itest_version_tag
//!
//! Pulls the base image, so it needs network access and some disk.

use super::*;

#[tokio::test]
#[ignore]
async fn itest_version_tag_is_pulled_and_a_missing_one_falls_back_to_latest() {
    let cm = ContainerManager::connect().expect("connect to docker (set DOCKER_HOST)");
    cm.check_available().await.expect("docker daemon must be reachable");

    // A tag the release workflow published: pulled as it is.
    let published = "ghcr.io/agentmuxai/agent-base:0.59.41";
    let chosen = cm.pull_with_fallback(published, &|_| {}).await.expect("the published version tag pulls anonymously");
    assert_eq!(chosen, published);

    // A version that was never released: `:latest`.
    let chosen = cm.pull_with_fallback("ghcr.io/agentmuxai/agent-base:9.99.99", &|_| {}).await.expect("falls back");
    assert_eq!(chosen, DEFAULT_AGENT_IMAGE);

    // An image that is not a version-pinned base image gets no fallback.
    let err = cm.pull_with_fallback("ghcr.io/agentmuxai/agent-base-not-pinned:9.99.99", &|_| {}).await.unwrap_err();
    assert!(matches!(err, ContainerError::ImagePull { .. }), "{err:?}");
}
