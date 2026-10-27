// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! Pulling a container agent's image: the progress the pane shows, and the
//! limits that end a pull which has stopped (spec
//! docs/specs/SPEC_CONTAINER_AGENTS_AT_PARITY_2026_10_27.md, P1 and P2).

use bollard::image::CreateImageOptions;
use futures_util::StreamExt as _;

use super::{ContainerError, ContainerManager};
use crate::backend::container_image::{classify_pull_error, is_legacy_agent_image, PullFailure, DEFAULT_AGENT_IMAGE};

impl ContainerManager {
    /// Pull `image`; a legacy image that the registry refuses or no longer has
    /// is replaced by the public base image. Returns the image to run.
    pub(super) async fn pull_with_fallback(&self, image: &str, observe: StartObserver<'_>) -> Result<String, ContainerError> {
        match self.pull_image(image, observe).await {
            Ok(()) => Ok(image.to_string()),
            Err(ContainerError::ImagePull { kind, .. }) if kind.is_unavailable_image() && is_legacy_agent_image(image) => {
                tracing::warn!(
                    image = image,
                    fallback = DEFAULT_AGENT_IMAGE,
                    "legacy agent image can't be pulled; using the public base image"
                );
                self.pull_image(DEFAULT_AGENT_IMAGE, observe).await?;
                Ok(DEFAULT_AGENT_IMAGE.to_string())
            }
            Err(e) => Err(e),
        }
    }

    /// Pull `image` via the Docker socket (create_image API).
    ///
    /// Streams the pull response to completion before returning. If the image is
    /// already present locally, Docker returns an empty stream immediately — this
    /// function treats that as success (no pull needed).
    ///
    /// Errors only on genuine pull failures (network, auth, no such image).
    async fn pull_image(&self, image: &str, observe: StartObserver<'_>) -> Result<(), ContainerError> {
        // Skip the pull entirely when the image is already present locally.
        // `create_image` always contacts the registry, so on an offline host the
        // pull stream errors out even when the image is cached — aborting
        // container creation even though `docker run` would start it from the
        // local cache. An `inspect_image` short-circuit makes the cached-offline
        // path work (and avoids a redundant registry round-trip when online).
        if self.inner.docker.inspect_image(image).await.is_ok() {
            tracing::info!(image = image, "image already present locally; skipping pull");
            return Ok(());
        }

        tracing::info!(image = image, "pulling image (not cached locally)");
        observe(StartEvent::Pulling { image, progress: None });
        let mut stream = self.inner.docker.create_image(
            Some(CreateImageOptions {
                from_image: image,
                ..Default::default()
            }),
            None,
            None,
        );
        drain_pull(image, &mut stream, PullLimits::default(), observe).await?;
        tracing::info!(image = image, "image pull complete (or already cached)");
        Ok(())
    }
}

/// How far an image download has got, from the byte counts the daemon reports
/// for each layer it is downloading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PullProgress {
    pub downloaded: u64,
    pub total: u64,
}

impl PullProgress {
    /// Whole percent, 0 to 100.
    pub fn percent(&self) -> u8 {
        if self.total == 0 {
            return 0;
        }
        (self.downloaded.min(self.total) * 100 / self.total) as u8
    }
}

/// A step of getting a container ready that takes long enough for a person to
/// wonder what is happening.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartEvent<'a> {
    /// The image is being downloaded; `progress` is `None` until the daemon
    /// has reported any byte counts.
    Pulling { image: &'a str, progress: Option<PullProgress> },
    /// The image is here and the container is being created and started.
    Starting,
}

/// Told about each [`StartEvent`]; called from the task that is starting the
/// container, so it must not block.
pub type StartObserver<'a> = &'a (dyn Fn(StartEvent<'_>) + Send + Sync);

/// How long a pull may go quiet, and how long it may take in all.
#[derive(Debug, Clone, Copy)]
pub struct PullLimits {
    pub stall: std::time::Duration,
    pub total: std::time::Duration,
}

impl Default for PullLimits {
    fn default() -> Self {
        Self {
            stall: std::time::Duration::from_secs(crate::backend::container_image::PULL_STALL_SECS),
            total: std::time::Duration::from_secs(crate::backend::container_image::PULL_TOTAL_SECS),
        }
    }
}

/// Turns the daemon's per-layer byte counts into [`PullProgress`] updates,
/// few enough to show: the first one, then one every two seconds when the
/// percentage moved, or sooner when it jumped by ten points.
#[derive(Default)]
struct PullTracker {
    layers: std::collections::HashMap<String, (u64, u64)>,
    last: Option<(std::time::Instant, u8)>,
}

impl PullTracker {
    fn observe(&mut self, info: &bollard::models::CreateImageInfo, now: std::time::Instant) -> Option<PullProgress> {
        let id = info.id.as_deref()?;
        let status = info.status.as_deref().unwrap_or_default();
        if let Some(total) = info.progress_detail.as_ref().and_then(|d| d.total).filter(|t| *t > 0) {
            let current = info.progress_detail.as_ref().and_then(|d| d.current).unwrap_or(0).clamp(0, total);
            self.layers.insert(id.to_string(), (current as u64, total as u64));
        } else if status.starts_with("Download complete") || status.starts_with("Pull complete") {
            if let Some(layer) = self.layers.get_mut(id) {
                layer.0 = layer.1;
            }
        } else {
            return None;
        }
        let progress = PullProgress {
            downloaded: self.layers.values().map(|l| l.0).sum(),
            total: self.layers.values().map(|l| l.1).sum(),
        };
        let percent = progress.percent();
        let due = match self.last {
            None => true,
            Some((at, last)) => {
                percent >= last.saturating_add(10) || (percent != last && now.duration_since(at) >= std::time::Duration::from_secs(2))
            }
        };
        if !due {
            return None;
        }
        self.last = Some((now, percent));
        Some(progress)
    }
}

/// Read a pull's stream to its end, reporting progress, and give up when it
/// goes quiet for `limits.stall` or runs past `limits.total`. Dropping the
/// stream cancels the download at the daemon, so nothing is left running.
async fn drain_pull<S>(
    image: &str,
    stream: &mut S,
    limits: PullLimits,
    observe: StartObserver<'_>,
) -> Result<(), ContainerError>
where
    S: futures_util::Stream<Item = Result<bollard::models::CreateImageInfo, bollard::errors::Error>> + Unpin,
{
    let started = std::time::Instant::now();
    let mut tracker = PullTracker::default();
    loop {
        let left = limits.total.saturating_sub(started.elapsed());
        let total_is_next = left <= limits.stall;
        let wait = limits.stall.min(left);
        let item = match tokio::time::timeout(wait, stream.next()).await {
            Ok(item) => item,
            Err(_) => {
                let kind = if total_is_next { PullFailure::TooSlow } else { PullFailure::Stalled };
                tracing::warn!(image = image, ?kind, elapsed_s = started.elapsed().as_secs(), "image pull given up");
                return Err(ContainerError::ImagePull {
                    image: image.to_string(),
                    kind,
                    detail: format!("no end after {} s", started.elapsed().as_secs()),
                });
            }
        };
        match item {
            None => return Ok(()),
            Some(Ok(info)) => {
                if let Some(status) = &info.status {
                    tracing::debug!(image = image, status = %status, "pull progress");
                }
                if let Some(progress) = tracker.observe(&info, std::time::Instant::now()) {
                    observe(StartEvent::Pulling { image, progress: Some(progress) });
                }
            }
            Some(Err(e)) => {
                tracing::warn!(image = image, error = %e, "image pull error");
                return Err(pull_error(image, &e));
            }
        }
    }
}

/// Map a bollard pull error to [`ContainerError::ImagePull`].
pub(super) fn pull_error(image: &str, e: &bollard::errors::Error) -> ContainerError {
    let status = match e {
        bollard::errors::Error::DockerResponseServerError { status_code, .. } => Some(*status_code),
        _ => None,
    };
    let detail = e.to_string();
    ContainerError::ImagePull {
        image: image.to_string(),
        kind: classify_pull_error(status, &detail),
        detail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::models::{CreateImageInfo, ProgressDetail};
    use futures_util::stream;
    use std::sync::Mutex as StdMutex;
    use std::time::Duration;

    fn layer(id: &str, status: &str, current: i64, total: i64) -> CreateImageInfo {
        CreateImageInfo {
            id: Some(id.to_string()),
            status: Some(status.to_string()),
            progress_detail: Some(ProgressDetail { current: Some(current), total: Some(total) }),
            ..Default::default()
        }
    }

    type Seen = StdMutex<Vec<Option<u8>>>;

    fn recorder(seen: &Seen) -> impl Fn(StartEvent<'_>) + Send + Sync + '_ {
        move |event| {
            if let StartEvent::Pulling { progress, .. } = event {
                seen.lock().unwrap().push(progress.map(|p| p.percent()));
            }
        }
    }

    fn quick() -> PullLimits {
        PullLimits { stall: Duration::from_millis(60), total: Duration::from_millis(400) }
    }

    #[tokio::test]
    async fn a_pull_that_goes_quiet_is_given_up_with_the_stalled_kind() {
        let seen = Seen::default();
        let mut quiet = stream::pending::<Result<CreateImageInfo, bollard::errors::Error>>();
        let started = std::time::Instant::now();
        let err = drain_pull("ghcr.io/x/y:z", &mut quiet, quick(), &recorder(&seen)).await.unwrap_err();
        assert!(matches!(&err, ContainerError::ImagePull { kind: PullFailure::Stalled, .. }), "{err:?}");
        assert!(started.elapsed() < Duration::from_millis(350), "gave up at the stall limit, not the total");
        assert!(err.user_message().contains("no data for"), "{}", err.user_message());
    }

    #[tokio::test]
    async fn a_pull_that_keeps_trickling_is_given_up_at_the_total_limit() {
        let seen = Seen::default();
        // One layer item every 30 ms, so the stall limit (60 ms) never trips.
        let mut trickle = Box::pin(stream::unfold(0i64, |n| async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            Some((Ok::<_, bollard::errors::Error>(layer("a", "Downloading", n, 1_000_000)), n + 1))
        }));
        let err = drain_pull("ghcr.io/x/y:z", &mut trickle, quick(), &recorder(&seen)).await.unwrap_err();
        assert!(matches!(&err, ContainerError::ImagePull { kind: PullFailure::TooSlow, .. }), "{err:?}");
    }

    #[tokio::test]
    async fn a_pull_that_finishes_reports_progress_and_succeeds() {
        let seen = Seen::default();
        let items: Vec<Result<CreateImageInfo, bollard::errors::Error>> = vec![
            Ok(CreateImageInfo { status: Some("Pulling from x/y".into()), ..Default::default() }),
            Ok(layer("a", "Downloading", 10, 100)),
            Ok(layer("a", "Downloading", 100, 100)),
        ];
        let mut s = stream::iter(items);
        drain_pull("ghcr.io/x/y:z", &mut s, quick(), &recorder(&seen)).await.expect("pull ends cleanly");
        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen.first(), Some(&Some(10)), "the first count is shown at once: {seen:?}");
        assert_eq!(seen.last(), Some(&Some(100)), "a jump of ten points or more is shown at once: {seen:?}");
    }

    #[test]
    fn progress_is_summed_over_layers_and_shown_sparingly() {
        let now = std::time::Instant::now();
        let mut t = PullTracker::default();
        // First count: shown (10 of 100 bytes = 10 %).
        assert_eq!(t.observe(&layer("a", "Downloading", 10, 100), now).map(|p| p.percent()), Some(10));
        assert_eq!(t.observe(&layer("b", "Downloading", 10, 100), now), None, "same percent window, under 2 s, under 10 points");
        // A change of under 10 points, 2 s later: shown.
        let later = now + Duration::from_secs(2);
        assert_eq!(t.observe(&layer("b", "Downloading", 20, 100), later).map(|p| p.percent()), Some(15));
        // A layer that says it is complete counts as fully downloaded, and a 10-point jump is shown at once.
        let done = CreateImageInfo { id: Some("a".into()), status: Some("Download complete".into()), ..Default::default() };
        assert_eq!(t.observe(&done, later).map(|p| p.percent()), Some(60));
        // Items without a layer id or byte counts say nothing.
        assert_eq!(t.observe(&CreateImageInfo { status: Some("Pulling fs layer".into()), ..Default::default() }, later), None);
    }
}
