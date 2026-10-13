// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `GET /agentmux/viewer/terminals/:block_id/feed`: one terminal's output,
//! live and read-only, for a paired device
//! (`agentmux-mobile/docs/reports/REPORT_PANE_LIST_TERMINALS_AND_FROZEN_FEED_2026_10_12.md`
//! §2.4, §2.5 phase B). Served beside the other viewer routes, on the viewer
//! listener only, for the viewer token only.
//!
//! Off unless the user turns on "Share terminals with paired devices"
//! (`viewer:shareterminals`): a terminal's scrollback routinely holds
//! secrets, and terminals have no per-pane hide flag. The shell drawer of an
//! agent hidden from paired devices is hidden with it. Nothing is ever sent
//! to the terminal: no input, no resize. The device renders at the size the
//! desktop gave the PTY (`size` events).
//!
//! Wire (SSE): `snapshot { offset, next_offset, data64, cols?, rows? }`, the
//! last 256 KB of the terminal's output from a line start; `append { offset,
//! data64 }` per write; `size { cols, rows }` when the PTY is resized;
//! `reset { reason }` before a fresh snapshot, or before the stream ends
//! (`behind`, `hidden`, `not_shared`); `: hb` every 15 s. Ids are the byte
//! offset after the event; a reconnect whose `Last-Event-ID` is still within
//! 1 MB of the end gets the bytes it missed as one `append`.

use std::convert::Infallible;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::Path;
use base64::Engine as _;
use futures_util::Stream;

use super::http_viewer::{agent_hidden, ViewerCaller, FEED_HEARTBEAT, HEARTBEAT, RETRY_PREAMBLE, STATUS_POLL};
use super::*;
use crate::backend::viewer::feed::{self, HubEvent, TERM_FILE};
use crate::backend::viewer::terminal::{self, TermStep};
use crate::backend::viewer::{FeedLease, FeedRefused};

/// Settings key: let paired devices watch this computer's terminals. Off
/// unless set.
pub(crate) const SETTING_SHARE_TERMINALS: &str = "viewer:shareterminals";

/// Whether the user lets paired devices watch terminals.
pub(crate) fn terminals_shared(state: &AppState) -> bool {
    state
        .config_watcher
        .get_settings()
        .extra
        .get(SETTING_SHARE_TERMINALS)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Why a block's terminal can't be watched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unwatchable {
    /// Gone, or not a terminal.
    NotATerminal,
    /// The shell drawer of an agent hidden from paired devices.
    Hidden,
}

/// Whether `block_id` is a terminal a paired device may watch (sharing aside).
pub(crate) fn terminal_watchable(state: &AppState, block_id: &str) -> Result<(), Unwatchable> {
    use crate::backend::obj::{meta_get_string, Block};
    let Ok(Some(block)) = state.mstore.get::<Block>(block_id) else {
        return Err(Unwatchable::NotATerminal);
    };
    if meta_get_string(&block.meta, "view", "") != "term" {
        return Err(Unwatchable::NotATerminal);
    }
    // An agent CLI can run in a terminal pane: that pane is the agent's, and
    // hidden with it.
    if agent_hidden(state, block_id) {
        return Err(Unwatchable::Hidden);
    }
    let key = crate::backend::blockcontroller::META_KEY_SHELL_SUBBLOCK_ID;
    let parent = state
        .mstore
        .get_all::<Block>()
        .unwrap_or_default()
        .into_iter()
        .find(|b| meta_get_string(&b.meta, key, "") == block_id);
    match parent {
        Some(agent) if agent_hidden(state, &agent.oid) => Err(Unwatchable::Hidden),
        _ => Ok(()),
    }
}

fn refused(status: StatusCode, error: &str) -> Response {
    (status, Json(json!({ "error": error }))).into_response()
}

pub(super) async fn handle_viewer_terminal_feed(
    State(state): State<AppState>,
    Extension(caller): Extension<ViewerCaller>,
    Path(block_id): Path<String>,
    headers: axum::http::HeaderMap,
) -> Response {
    if !terminals_shared(&state) {
        return refused(StatusCode::FORBIDDEN, "this computer doesn't share its terminals");
    }
    let st = state.clone();
    let id = block_id.clone();
    let watchable = tokio::task::spawn_blocking(move || terminal_watchable(&st, &id)).await;
    if !matches!(watchable, Ok(Ok(()))) {
        return refused(StatusCode::NOT_FOUND, "no such terminal");
    }
    let lease = match state.viewer.open_feed(&caller.device_id) {
        Ok(lease) => lease,
        Err(FeedRefused::PerDevice) => {
            return refused(StatusCode::SERVICE_UNAVAILABLE, "too many open feeds for this device")
        }
        Err(FeedRefused::Total) => return refused(StatusCode::SERVICE_UNAVAILABLE, "too many open feeds"),
    };
    let last_event_id = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<u64>().ok());
    tracing::info!(device = %caller.device_id, block = %block_id, resume = last_event_id.is_some(), "viewer terminal feed: opened");
    let stream = terminal_stream(
        TermSource::of(&state, &block_id),
        last_event_id,
        lease,
        TermTiming::default(),
        TermProbe::of(&state, &block_id),
    );
    (
        [(header::CONTENT_TYPE, "text/event-stream"), (header::CACHE_CONTROL, "no-cache")],
        Body::from_stream(stream),
    )
        .into_response()
}

/// A terminal feed's clocks.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TermTiming {
    pub heartbeat: Duration,
    pub size_poll: Duration,
}

impl Default for TermTiming {
    fn default() -> Self {
        Self { heartbeat: FEED_HEARTBEAT, size_poll: STATUS_POLL }
    }
}

/// What a terminal feed asks while it runs: the PTY's size, and whether it
/// may still be watched (`Some(reason)` ends the stream).
pub(crate) struct TermProbe {
    pub size: Box<dyn Fn() -> Option<(u16, u16)> + Send>,
    pub closed: Box<dyn Fn() -> Option<&'static str> + Send>,
}

impl TermProbe {
    pub(crate) fn of(state: &AppState, block_id: &str) -> Self {
        let size_block = block_id.to_string();
        let (st, block) = (state.clone(), block_id.to_string());
        Self {
            size: Box::new(move || crate::backend::blockcontroller::term_size::get(&size_block)),
            closed: Box::new(move || {
                if !terminals_shared(&st) {
                    return Some("not_shared");
                }
                match terminal_watchable(&st, &block) {
                    Ok(()) => None,
                    Err(Unwatchable::Hidden) => Some("hidden"),
                    Err(Unwatchable::NotATerminal) => Some("gone"),
                }
            }),
        }
    }
}

/// Where a terminal feed reads the terminal's output from: its `term` file.
pub(crate) struct TermSource {
    filestore: Arc<crate::backend::storage::filestore::FileStore>,
    hub: Arc<feed::FeedHub>,
    block_id: String,
}

/// A snapshot as read: its bytes start at `offset`; the file ends at `next`.
struct TermSnapshot {
    offset: u64,
    next: u64,
    data: Vec<u8>,
}

impl TermSource {
    pub(crate) fn of(state: &AppState, block_id: &str) -> Self {
        Self { filestore: state.filestore.clone(), hub: state.viewer.hub.clone(), block_id: block_id.to_string() }
    }

    /// The file's size and its bytes from `from` (`None`: from
    /// [`terminal::SNAPSHOT_TAIL_BYTES`] before the end), on the blocking pool.
    async fn read_from(&self, from: Option<u64>) -> Option<(u64, u64, Vec<u8>)> {
        let (fs, block) = (self.filestore.clone(), self.block_id.clone());
        tokio::task::spawn_blocking(move || {
            let size = match fs.stat(&block, TERM_FILE) {
                Ok(Some(file)) => file.size.max(0) as u64,
                Ok(None) => return Some((0, 0, Vec::new())),
                Err(_) => return None,
            };
            let start = from.unwrap_or_else(|| size.saturating_sub(terminal::SNAPSHOT_TAIL_BYTES));
            if start >= size {
                return Some((start, size, Vec::new()));
            }
            let (at, data) = fs.read_at(&block, TERM_FILE, start as i64, (size - start) as i64).ok()?;
            Some((at.max(0) as u64, size, data))
        })
        .await
        .ok()
        .flatten()
    }

    /// The tail, from a line start.
    async fn snapshot(&self) -> Option<TermSnapshot> {
        let (offset, next, mut data) = self.read_from(None).await?;
        let start = terminal::tail_start(offset, &data);
        data.drain(..start);
        Some(TermSnapshot { offset: offset + start as u64, next, data })
    }

    /// The bytes from `from` on, for a reconnect: `None` when `from` is past
    /// the end (the file was truncated) or more than a stream may fall behind
    /// by was written since (a snapshot then).
    async fn resume(&self, from: u64) -> Option<(u64, Vec<u8>)> {
        let (at, size, data) = self.read_from(Some(from)).await?;
        if from > size || at != from || size - from > feed::MAX_BEHIND_BYTES as u64 {
            return None;
        }
        Some((at, data))
    }
}

fn b64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn snapshot_event(s: &TermSnapshot, size: Option<(u16, u16)>) -> Bytes {
    let mut data = json!({ "offset": s.offset, "next_offset": s.next, "data64": b64(&s.data) });
    if let Some((rows, cols)) = size {
        data["rows"] = json!(rows);
        data["cols"] = json!(cols);
    }
    feed::sse_event("snapshot", Some(&s.next.to_string()), &data)
}

fn append_event(offset: u64, data: &[u8]) -> Bytes {
    let end = offset + data.len() as u64;
    feed::sse_event("append", Some(&end.to_string()), &json!({ "offset": offset, "data64": b64(data) }))
}

fn size_event((rows, cols): (u16, u16)) -> Bytes {
    feed::sse_event("size", None, &json!({ "rows": rows, "cols": cols }))
}

fn reset_event(reason: &str) -> Bytes {
    feed::sse_event("reset", None, &json!({ "reason": reason }))
}

/// One terminal feed: `retry`, a `snapshot` (or, for a reconnect that still
/// holds, the bytes it missed as one `append`), then an `append` per write,
/// `size` on a resize, `reset` + `snapshot` when the file is truncated or the
/// feed loses its place, and a heartbeat on a quiet stream. Ends with `reset`
/// when the device falls more than 1 MB behind or the terminal may no longer
/// be watched, and silently when the device is revoked or goes away.
pub(crate) fn terminal_stream(
    source: TermSource,
    last_event_id: Option<u64>,
    lease: FeedLease,
    timing: TermTiming,
    probe: TermProbe,
) -> impl Stream<Item = Result<Bytes, Infallible>> + Send + 'static {
    async_stream::stream! {
        let lease = lease;
        // Subscribed before anything is read, so nothing written between the
        // read and the first event is missed.
        let mut sub = source.hub.subscribe_file(&source.block_id, TERM_FILE);
        yield Ok(Bytes::from_static(RETRY_PREAMBLE));
        let mut shown_size = (probe.size)();

        let resumed = match last_event_id {
            Some(from) => source.resume(from).await,
            None => None,
        };
        let mut next = match resumed {
            Some((at, data)) => {
                if !data.is_empty() {
                    yield Ok(append_event(at, &data));
                }
                if let Some(size) = shown_size {
                    yield Ok(size_event(size));
                }
                Some(at + data.len() as u64)
            }
            None => match source.snapshot().await {
                Some(snapshot) => {
                    yield Ok(snapshot_event(&snapshot, shown_size));
                    Some(snapshot.next)
                }
                None => None,
            },
        };
        if next.is_none() {
            yield Ok(reset_event("unavailable"));
        }

        let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + timing.heartbeat, timing.heartbeat);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut size_tick = tokio::time::interval_at(tokio::time::Instant::now() + timing.size_poll, timing.size_poll);
        size_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        while let Some(cursor) = next.as_mut() {
            let mut resync: Option<String> = None;
            tokio::select! {
                biased;
                _ = lease.token.cancelled() => break,
                event = sub.recv() => match event {
                    None => break,
                    Some(HubEvent::Behind) => {
                        yield Ok(reset_event("behind"));
                        break;
                    }
                    Some(HubEvent::Rewritten { fileop }) => resync = Some(fileop),
                    Some(HubEvent::Append { offset, records, .. }) => match terminal::apply_term_append(cursor, offset, &records) {
                        TermStep::Send { offset, data } => yield Ok(append_event(offset, &data)),
                        TermStep::Skip => {}
                        TermStep::Resync(reason) => resync = Some(reason.to_string()),
                    },
                },
                _ = size_tick.tick() => {
                    let size = (probe.size)();
                    if size.is_some() && size != shown_size {
                        shown_size = size;
                        if let Some(size) = size {
                            yield Ok(size_event(size));
                        }
                    }
                }
                _ = tick.tick() => {
                    if let Some(reason) = (probe.closed)() {
                        yield Ok(reset_event(reason));
                        break;
                    }
                    yield Ok(Bytes::from_static(HEARTBEAT));
                }
            }
            if let Some(reason) = resync {
                tracing::info!(block = %source.block_id, %reason, "viewer terminal feed: reset; sending a fresh snapshot");
                yield Ok(reset_event(&reason));
                match source.snapshot().await {
                    Some(snapshot) => {
                        next = Some(snapshot.next);
                        yield Ok(snapshot_event(&snapshot, shown_size));
                    }
                    None => {
                        yield Ok(reset_event("unavailable"));
                        break;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/viewer/viewer_terminal_tests.rs"]
mod tests;
