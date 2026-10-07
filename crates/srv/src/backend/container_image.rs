// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which container image an agent runs, and what happens when it can't be had.
//!
//! - The default and the legacy image names, and the fallback between them.
//! - Classifying a failed pull into something a person can act on, instead of
//!   passing bollard's text through.
//! - An anonymous registry check ("could the daemon pull this?") for the
//!   create flow, which has to decide before anything is pulled.
//!
//! Spec: docs/specs/SPEC_CONTAINER_AGENTS_WORK_FOR_EVERYONE_2026_10_07.md.

use std::time::Duration;

use crate::backend::container::ContainerManager;
use crate::backend::rpc_types::ContainerImageAccess;

/// The public base image: everything a container agent needs except the
/// provider CLI, which is installed on first start.
pub const DEFAULT_AGENT_IMAGE: &str = "ghcr.io/agentmuxai/agent-base:latest";

/// Repository of the earlier image that bundled Claude Code. It is private, so
/// only machines that already hold it can use it.
const LEGACY_AGENT_IMAGE_REPO: &str = "ghcr.io/agentmuxai/agent-claude";

/// Where the provider CLI is installed inside the container.
pub const CONTAINER_CLI_DIR: &str = "/home/agent/.agentmux/cli";

/// The image a stored `agent:container_image` value means: empty is the default.
pub fn resolve_container_image(stored: &str) -> String {
    let stored = stored.trim();
    if stored.is_empty() {
        DEFAULT_AGENT_IMAGE.to_string()
    } else {
        stored.to_string()
    }
}

/// Whether `image` is the private legacy image, at any tag.
pub fn is_legacy_agent_image(image: &str) -> bool {
    let image = image.trim();
    match image.strip_prefix(LEGACY_AGENT_IMAGE_REPO) {
        Some(rest) => rest.is_empty() || rest.starts_with(':') || rest.starts_with('@'),
        None => false,
    }
}

/// Why a pull failed, as far as the daemon's text lets us tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullFailure {
    /// The registry refused access: private image, or no such repository for
    /// an anonymous caller.
    Denied,
    /// The registry says the image or tag does not exist.
    NotFound,
    /// The registry could not be reached.
    Network,
    Other,
}

impl PullFailure {
    /// Whether falling back to the base image can help: the legacy image is
    /// refused or gone, not merely unreachable right now.
    pub fn is_unavailable_image(self) -> bool {
        matches!(self, Self::Denied | Self::NotFound)
    }
}

/// Classify a pull error from the daemon's HTTP status (when it had one) and text.
pub fn classify_pull_error(status: Option<u16>, message: &str) -> PullFailure {
    let m = message.to_ascii_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| m.contains(n));

    if matches!(status, Some(401 | 403))
        || has(&[
            "denied",
            "unauthorized",
            "authentication required",
            "forbidden",
            "insufficient_scope",
        ])
    {
        return PullFailure::Denied;
    }
    if status == Some(404) || has(&["manifest unknown", "name unknown", "not found", "no such image"]) {
        return PullFailure::NotFound;
    }
    if has(&[
        "no such host",
        "lookup ",
        "timeout",
        "timed out",
        "deadline exceeded",
        "connection refused",
        "connection reset",
        "tls handshake",
        "no route to host",
        "network is unreachable",
        "dial tcp",
        "proxyconnect",
        "unexpected eof",
    ]) {
        return PullFailure::Network;
    }
    PullFailure::Other
}

/// The sentence for a failed pull. Names the image, says what happened, and
/// says what to do next. Never carries the daemon's raw text except for
/// `Other`, where there is nothing better to say.
pub fn pull_failure_message(image: &str, kind: PullFailure, detail: &str) -> String {
    match kind {
        PullFailure::Denied => format!(
            "Couldn't start the container for this agent: the registry refused access to {image}. \
             AgentMux downloads images without signing in, so the image has to be public. \
             Create the agent to run on this computer (host) instead, or use a public image."
        ),
        PullFailure::NotFound => format!(
            "Couldn't start the container for this agent: {image} was not found in its registry. \
             Check the image name in the agent's settings, or create the agent to run on this computer (host)."
        ),
        PullFailure::Network => format!(
            "Couldn't start the container for this agent: couldn't reach the registry to download {image}. \
             Check your internet connection and any proxy set in Docker, then send your message again, \
             or create the agent to run on this computer (host)."
        ),
        PullFailure::Other => format!(
            "Couldn't start the container for this agent: couldn't download {image}: {}. \
             Send your message again, or create the agent to run on this computer (host).",
            truncate(detail.trim(), 300)
        ),
    }
}

/// The sentence for a provider CLI that couldn't be installed in the container.
pub fn cli_install_failure_message(command: &str, detail: &str) -> String {
    format!(
        "Couldn't start the container for this agent: couldn't install {command} in it: {}. \
         Check your internet connection, then send your message again, \
         or create the agent to run on this computer (host).",
        truncate(detail.trim(), 400)
    )
}

/// The sentence for a Docker daemon that isn't answering.
pub fn docker_unreachable_message() -> String {
    "Couldn't start the container for this agent: Docker isn't responding. \
     Start Docker Desktop (or your container runtime) and send your message again, \
     or create the agent to run on this computer (host)."
        .to_string()
}

/// The sentence for any other Docker failure while starting the container.
pub fn docker_error_message(detail: &str) -> String {
    format!(
        "Couldn't start the container for this agent: Docker reported an error: {}",
        truncate(detail.trim(), 300)
    )
}

/// Whether a Docker error is the daemon being unreachable, not a request it refused.
pub fn looks_like_daemon_unreachable(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    [
        "error trying to connect",
        "cannot connect to the docker daemon",
        "is the docker daemon running",
        "the system cannot find the file specified",
        "the system cannot find the path specified",
        "docker.sock",
        "docker_engine",
    ]
    .iter()
    .any(|n| m.contains(n))
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{cut}...")
}

/// A parsed image reference: where to ask, which repository, which tag or digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRef {
    pub registry: String,
    pub repo: String,
    pub reference: String,
}

/// Split `image` the way Docker does. `None` for a reference we won't probe
/// (a local or port-qualified registry, which may well speak plain HTTP).
pub fn parse_image_ref(image: &str) -> Option<ImageRef> {
    let image = image.trim();
    if image.is_empty() {
        return None;
    }
    let (name, reference) = if let Some((name, digest)) = image.split_once('@') {
        (name, digest.to_string())
    } else {
        let last_slash = image.rfind('/').map_or(0, |i| i + 1);
        match image[last_slash..].rfind(':') {
            Some(i) => (&image[..last_slash + i], image[last_slash + i + 1..].to_string()),
            None => (image, "latest".to_string()),
        }
    };
    if reference.is_empty() {
        return None;
    }

    let (registry, repo) = match name.split_once('/') {
        Some((first, rest)) if first.contains('.') || first.contains(':') || first == "localhost" => {
            (first.to_string(), rest.to_string())
        }
        _ => ("docker.io".to_string(), name.to_string()),
    };
    if registry == "localhost" || registry.starts_with("localhost:") || registry.contains(':') {
        return None;
    }
    let (registry, repo) = if registry == "docker.io" || registry == "index.docker.io" {
        let repo = if repo.contains('/') { repo } else { format!("library/{repo}") };
        ("registry-1.docker.io".to_string(), repo)
    } else {
        (registry, repo)
    };
    if repo.is_empty() || repo.contains("..") {
        return None;
    }
    Some(ImageRef { registry, repo, reference })
}

/// The pieces of a `WWW-Authenticate: Bearer ...` challenge.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BearerChallenge {
    pub realm: String,
    pub service: Option<String>,
    pub scope: Option<String>,
}

/// Parse a Bearer challenge; `None` for any other scheme or a missing realm.
pub fn parse_bearer_challenge(header: &str) -> Option<BearerChallenge> {
    let rest = header.trim().strip_prefix("Bearer ").or_else(|| header.trim().strip_prefix("bearer "))?;
    let mut out = BearerChallenge::default();
    for part in split_challenge_params(rest) {
        let Some((key, value)) = part.split_once('=') else { continue };
        let value = value.trim().trim_matches('"').to_string();
        match key.trim().to_ascii_lowercase().as_str() {
            "realm" => out.realm = value,
            "service" => out.service = Some(value),
            "scope" => out.scope = Some(value),
            _ => {}
        }
    }
    (!out.realm.is_empty()).then_some(out)
}

/// Split on commas that are outside double quotes (a scope can hold `,`).
fn split_challenge_params(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut start, mut in_quotes) = (0, false);
    for (i, c) in s.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                parts.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(s[start..].trim());
    parts.into_iter().filter(|p| !p.is_empty()).collect()
}

const MANIFEST_ACCEPT: &str = "application/vnd.oci.image.index.v1+json, \
     application/vnd.oci.image.manifest.v1+json, \
     application/vnd.docker.distribution.manifest.list.v2+json, \
     application/vnd.docker.distribution.manifest.v2+json";

/// Ask the image's registry, without credentials, whether its manifest can be
/// read: the same question the daemon's pull answers. `Unknown` for anything
/// that isn't a definite answer (offline, a proxy, an unfamiliar registry).
pub async fn probe_anonymous(image: &str) -> ContainerImageAccess {
    let Some(image_ref) = parse_image_ref(image) else {
        return ContainerImageAccess::Unknown;
    };
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(6))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
    else {
        return ContainerImageAccess::Unknown;
    };
    let url = format!(
        "https://{}/v2/{}/manifests/{}",
        image_ref.registry, image_ref.repo, image_ref.reference
    );

    let first = match client.get(&url).header("Accept", MANIFEST_ACCEPT).send().await {
        Ok(r) => r,
        Err(_) => return ContainerImageAccess::Unknown,
    };
    match first.status().as_u16() {
        200 => return ContainerImageAccess::Public,
        404 => return ContainerImageAccess::NotFound,
        401 => {}
        403 => return ContainerImageAccess::Denied,
        _ => return ContainerImageAccess::Unknown,
    }

    let challenge = first
        .headers()
        .get("www-authenticate")
        .and_then(|v| v.to_str().ok())
        .and_then(parse_bearer_challenge);
    let Some(challenge) = challenge else {
        return ContainerImageAccess::Denied;
    };

    let mut query: Vec<(&str, String)> = Vec::new();
    if let Some(service) = challenge.service {
        query.push(("service", service));
    }
    query.push(("scope", challenge.scope.unwrap_or_else(|| format!("repository:{}:pull", image_ref.repo))));
    let token = match client.get(&challenge.realm).query(&query).send().await {
        Ok(r) if r.status().is_success() => r
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| {
                v.get("token")
                    .or_else(|| v.get("access_token"))
                    .and_then(|t| t.as_str())
                    .map(str::to_string)
            }),
        Ok(r) if matches!(r.status().as_u16(), 401 | 403) => return ContainerImageAccess::Denied,
        _ => None,
    };
    let Some(token) = token else {
        return ContainerImageAccess::Unknown;
    };

    match client
        .get(&url)
        .header("Accept", MANIFEST_ACCEPT)
        .bearer_auth(token)
        .send()
        .await
    {
        Ok(r) => match r.status().as_u16() {
            200 => ContainerImageAccess::Public,
            401 | 403 => ContainerImageAccess::Denied,
            404 => ContainerImageAccess::NotFound,
            _ => ContainerImageAccess::Unknown,
        },
        Err(_) => ContainerImageAccess::Unknown,
    }
}

/// Whether `image` can be had here. A stored legacy image is also checked
/// against the base image it falls back to, so the answer matches what a
/// start would actually do.
pub async fn check_image_access(manager: Option<&ContainerManager>, image: &str) -> ContainerImageAccess {
    let image = resolve_container_image(image);
    let mut candidates = vec![image.clone()];
    if is_legacy_agent_image(&image) {
        candidates.push(DEFAULT_AGENT_IMAGE.to_string());
    }
    let mut best: Option<ContainerImageAccess> = None;
    for candidate in &candidates {
        let local = match manager {
            Some(m) => m.has_image_locally(candidate).await,
            None => false,
        };
        let access = if local { ContainerImageAccess::Local } else { probe_anonymous(candidate).await };
        best = Some(match best {
            Some(prev) => better_access(prev, access),
            None => access,
        });
    }
    best.unwrap_or(ContainerImageAccess::Unknown)
}

/// The better of two answers for "can this be had": something usable beats
/// something unknown, which beats a refusal.
pub fn better_access(a: ContainerImageAccess, b: ContainerImageAccess) -> ContainerImageAccess {
    fn rank(x: ContainerImageAccess) -> u8 {
        match x {
            ContainerImageAccess::Local => 4,
            ContainerImageAccess::Public => 3,
            ContainerImageAccess::Unknown => 2,
            ContainerImageAccess::Denied => 1,
            ContainerImageAccess::NotFound => 0,
        }
    }
    if rank(b) > rank(a) { b } else { a }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_stored_image_means_the_public_base_image() {
        assert_eq!(resolve_container_image(""), DEFAULT_AGENT_IMAGE);
        assert_eq!(resolve_container_image("  "), DEFAULT_AGENT_IMAGE);
        assert_eq!(resolve_container_image("my/own:1"), "my/own:1");
    }

    #[test]
    fn the_legacy_image_is_recognised_at_any_tag_and_nothing_else_is() {
        assert!(is_legacy_agent_image("ghcr.io/agentmuxai/agent-claude:latest"));
        assert!(is_legacy_agent_image("ghcr.io/agentmuxai/agent-claude:2026-06-14"));
        assert!(is_legacy_agent_image("ghcr.io/agentmuxai/agent-claude"));
        assert!(is_legacy_agent_image("ghcr.io/agentmuxai/agent-claude@sha256:abc"));
        assert!(!is_legacy_agent_image("ghcr.io/agentmuxai/agent-claude-extra:latest"));
        assert!(!is_legacy_agent_image(DEFAULT_AGENT_IMAGE));
        assert!(!is_legacy_agent_image("someone/agent-claude:latest"));
    }

    #[test]
    fn real_daemon_texts_classify_as_denied() {
        for text in [
            "Head \"https://ghcr.io/v2/agentmuxai/agent-claude/manifests/latest\": denied",
            "Error response from daemon: Head \"https://ghcr.io/v2/x/manifests/latest\": unauthorized",
            "pull access denied for x, repository does not exist or may require 'docker login'",
            "unauthorized: authentication required",
            "denied: requested access to the resource is denied",
        ] {
            assert_eq!(classify_pull_error(Some(500), text), PullFailure::Denied, "{text}");
        }
        assert_eq!(classify_pull_error(Some(401), ""), PullFailure::Denied);
        assert_eq!(classify_pull_error(Some(403), "whatever"), PullFailure::Denied);
    }

    #[test]
    fn missing_images_classify_as_not_found() {
        assert_eq!(classify_pull_error(Some(404), ""), PullFailure::NotFound);
        assert_eq!(
            classify_pull_error(Some(500), "manifest for ghcr.io/x/y:nope not found: manifest unknown: manifest unknown"),
            PullFailure::NotFound
        );
        assert_eq!(classify_pull_error(None, "No such image: foo"), PullFailure::NotFound);
    }

    #[test]
    fn unreachable_registries_classify_as_network() {
        for text in [
            "Get \"https://ghcr.io/v2/\": dial tcp: lookup ghcr.io on 127.0.0.11:53: no such host",
            "Get \"https://ghcr.io/v2/\": net/http: TLS handshake timeout",
            "Get \"https://ghcr.io/v2/\": context deadline exceeded",
            "dial tcp 140.82.112.34:443: connect: connection refused",
            "proxyconnect tcp: dial tcp 10.0.0.1:3128: i/o timeout",
        ] {
            assert_eq!(classify_pull_error(Some(500), text), PullFailure::Network, "{text}");
        }
    }

    #[test]
    fn denied_wins_over_not_found_when_both_words_appear() {
        // Docker's own text for a private or missing repo mentions both.
        assert_eq!(
            classify_pull_error(Some(500), "pull access denied for x, repository does not exist or not found"),
            PullFailure::Denied
        );
    }

    #[test]
    fn anything_else_is_other() {
        assert_eq!(classify_pull_error(Some(500), "toomanyrequests: rate limit"), PullFailure::Other);
        assert_eq!(classify_pull_error(None, ""), PullFailure::Other);
    }

    #[test]
    fn only_a_refused_or_missing_image_is_worth_a_fallback() {
        assert!(PullFailure::Denied.is_unavailable_image());
        assert!(PullFailure::NotFound.is_unavailable_image());
        assert!(!PullFailure::Network.is_unavailable_image());
        assert!(!PullFailure::Other.is_unavailable_image());
    }

    #[test]
    fn every_message_names_the_image_and_a_next_step_and_never_a_raw_docker_error() {
        for kind in [PullFailure::Denied, PullFailure::NotFound, PullFailure::Network, PullFailure::Other] {
            let msg = pull_failure_message("ghcr.io/x/y:1", kind, "some detail");
            assert!(msg.starts_with("Couldn't start the container for this agent"), "{msg}");
            assert!(msg.contains("ghcr.io/x/y:1"), "{msg}");
            assert!(msg.contains("(host)"), "offers the host fallback: {msg}");
            assert!(!msg.contains("Docker API error"), "{msg}");
        }
    }

    #[test]
    fn only_the_catch_all_message_carries_the_daemons_text() {
        let denied = pull_failure_message("img", PullFailure::Denied, "Head https://secret-host/v2: denied");
        assert!(!denied.contains("secret-host"));
        let other = pull_failure_message("img", PullFailure::Other, "odd failure");
        assert!(other.contains("odd failure"));
    }

    #[test]
    fn a_long_detail_is_cut() {
        let msg = pull_failure_message("img", PullFailure::Other, &"x".repeat(2000));
        assert!(msg.len() < 700, "{}", msg.len());
    }

    #[test]
    fn daemon_down_texts_are_recognised() {
        assert!(looks_like_daemon_unreachable("error trying to connect: The system cannot find the file specified. (os error 2)"));
        assert!(looks_like_daemon_unreachable("Cannot connect to the Docker daemon at unix:///var/run/docker.sock"));
        assert!(!looks_like_daemon_unreachable("Docker responded with status code 409: name conflict"));
    }

    #[test]
    fn image_refs_split_like_docker_does() {
        let r = parse_image_ref("ghcr.io/agentmuxai/agent-base:latest").unwrap();
        assert_eq!((r.registry.as_str(), r.repo.as_str(), r.reference.as_str()), ("ghcr.io", "agentmuxai/agent-base", "latest"));

        let r = parse_image_ref("ghcr.io/agentmuxai/agent-base").unwrap();
        assert_eq!(r.reference, "latest");

        let r = parse_image_ref("alpine").unwrap();
        assert_eq!((r.registry.as_str(), r.repo.as_str(), r.reference.as_str()), ("registry-1.docker.io", "library/alpine", "latest"));

        let r = parse_image_ref("someone/tool:1.2").unwrap();
        assert_eq!((r.registry.as_str(), r.repo.as_str(), r.reference.as_str()), ("registry-1.docker.io", "someone/tool", "1.2"));

        let r = parse_image_ref("docker.io/library/node:24-slim").unwrap();
        assert_eq!((r.registry.as_str(), r.repo.as_str()), ("registry-1.docker.io", "library/node"));

        let r = parse_image_ref("ghcr.io/o/r@sha256:0123").unwrap();
        assert_eq!((r.repo.as_str(), r.reference.as_str()), ("o/r", "sha256:0123"));
    }

    #[test]
    fn local_and_port_registries_are_not_probed() {
        assert_eq!(parse_image_ref("localhost/foo:1"), None);
        assert_eq!(parse_image_ref("localhost:5000/foo:1"), None);
        assert_eq!(parse_image_ref("registry.local:5000/foo:1"), None);
        assert_eq!(parse_image_ref(""), None);
        assert_eq!(parse_image_ref("foo:"), None);
    }

    #[test]
    fn a_bearer_challenge_is_parsed_including_a_scope_with_a_comma() {
        let c = parse_bearer_challenge(
            r#"Bearer realm="https://ghcr.io/token",service="ghcr.io",scope="repository:agentmuxai/agent-base:pull""#,
        )
        .unwrap();
        assert_eq!(c.realm, "https://ghcr.io/token");
        assert_eq!(c.service.as_deref(), Some("ghcr.io"));
        assert_eq!(c.scope.as_deref(), Some("repository:agentmuxai/agent-base:pull"));

        let c = parse_bearer_challenge(r#"Bearer realm="https://auth.example/t",scope="repository:a/b:pull,push""#).unwrap();
        assert_eq!(c.scope.as_deref(), Some("repository:a/b:pull,push"));
        assert_eq!(c.service, None);
    }

    #[test]
    fn a_challenge_that_is_not_bearer_or_has_no_realm_is_none() {
        assert_eq!(parse_bearer_challenge("Basic realm=\"x\""), None);
        assert_eq!(parse_bearer_challenge("Bearer service=\"x\""), None);
        assert_eq!(parse_bearer_challenge(""), None);
    }

    #[test]
    fn a_usable_answer_beats_unknown_which_beats_a_refusal() {
        use ContainerImageAccess::*;
        assert_eq!(better_access(Denied, Public), Public);
        assert_eq!(better_access(Public, Denied), Public);
        assert_eq!(better_access(Denied, Unknown), Unknown);
        assert_eq!(better_access(NotFound, Denied), Denied);
        assert_eq!(better_access(Local, Public), Local);
    }

    /// Hits the real registry. Run by hand:
    /// `cargo test -p agentmux-srv --bin agentmux-srv -- --ignored probe_real`
    #[tokio::test]
    #[ignore]
    async fn probe_real_registries() {
        let public = probe_anonymous("docker.io/library/alpine:latest").await;
        let private = probe_anonymous("ghcr.io/agentmuxai/agent-claude:latest").await;
        let missing = probe_anonymous("docker.io/library/agentmux-no-such-image-xyz:latest").await;
        println!("alpine={public:?} agent-claude={private:?} missing={missing:?}");
        assert_eq!(public, ContainerImageAccess::Public);
        assert_eq!(private, ContainerImageAccess::Denied);
        assert!(matches!(missing, ContainerImageAccess::Denied | ContainerImageAccess::NotFound));
    }
}
