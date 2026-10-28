//! The default agent image, run by the digest the release's signed manifest names
//! (`backend::agent_image_digest`).

use bollard::image::TagImageOptions;

use super::{ContainerError, ContainerManager, StartObserver};

/// `repo` and `tag` of an image reference `repo:tag`; a registry's `:port` is not a tag.
fn split_tag(image: &str) -> Option<(&str, &str)> {
    let (repo, tag) = image.rsplit_once(':')?;
    (!tag.contains('/') && !repo.is_empty() && !tag.is_empty()).then_some((repo, tag))
}

impl ContainerManager {
    /// Make the local tag `image` (`repo:version`) the image at `reference` (`repo@sha256:...`),
    /// pulling it if it isn't here. Whatever the tag held before, a pushed or stale image, is
    /// replaced, so the container is created from the signed one. There is no fall back to a
    /// tag: the point is that no tag is trusted.
    pub(super) async fn pull_pinned(&self, image: &str, reference: &str, observe: StartObserver<'_>) -> Result<(), ContainerError> {
        let holds = |digests: Option<Vec<String>>| digests.is_some_and(|d| d.iter().any(|r| r == reference));
        if let Ok(local) = self.inner.docker.inspect_image(image).await {
            if holds(local.repo_digests) {
                tracing::info!(image, reference, "the local agent image is the signed one");
                return Ok(());
            }
        }
        self.pull_image(reference, observe).await?;
        let (repo, tag) = split_tag(image).ok_or_else(|| ContainerError::ImagePull {
            image: image.to_string(),
            kind: crate::backend::container_image::PullFailure::Other,
            detail: "not a repository and a tag".into(),
        })?;
        self.inner.docker.tag_image(reference, Some(TagImageOptions { repo, tag })).await?;
        tracing::info!(image, reference, "the agent image is the signed one, tagged locally");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tag_is_split_from_its_repository_and_a_port_is_not_a_tag() {
        assert_eq!(split_tag("ghcr.io/agentmuxai/agent-base:0.60.3"), Some(("ghcr.io/agentmuxai/agent-base", "0.60.3")));
        assert_eq!(split_tag("localhost:5000/base:1"), Some(("localhost:5000/base", "1")));
        assert_eq!(split_tag("localhost:5000/base"), None);
        assert_eq!(split_tag("base"), None);
    }

    /// Against a real daemon, with an image it already has (no registry): the tag is moved onto the
    /// image the reference names.
    ///   cargo test -p agentmux-srv --bins -- --ignored --nocapture itest_pinned
    #[tokio::test]
    #[ignore]
    async fn itest_pinned_tag_is_moved_onto_the_image_the_reference_names() {
        let cm = ContainerManager::connect().expect("connect to docker (set DOCKER_HOST)");
        cm.check_available().await.expect("docker daemon must be reachable");
        let docker = &cm.inner.docker;
        // Any small local image with a repo digest; the base image's own, which every host here has.
        let source = "ghcr.io/agentmuxai/agent-base:latest";
        let local = docker.inspect_image(source).await.expect("the base image is pulled on this machine");
        let reference = local.repo_digests.and_then(|d| d.into_iter().next()).expect("a pulled image has a repo digest");
        let tag = "agentmux-itest/pinned:1";
        let _ = docker.remove_image(tag, None::<bollard::image::RemoveImageOptions>, None).await;

        cm.pull_pinned(tag, &reference, &|_| {}).await.expect("a local reference needs no registry");
        let tagged = docker.inspect_image(tag).await.expect("the tag exists");
        assert_eq!(tagged.id, local.id, "the tag names the image the digest does");

        // Asked again, it is already the signed image.
        cm.pull_pinned(tag, &reference, &|_| {}).await.expect("idempotent");
        let _ = docker.remove_image(tag, None::<bollard::image::RemoveImageOptions>, None).await;
    }
}
