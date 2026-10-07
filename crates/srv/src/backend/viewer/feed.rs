// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The server-side half of the live feed: a [`FeedHub`] that watches every
//! published `blockfile` event in-process (a broker observer, so no
//! WebSocket route is involved) and hands each open feed the transcript
//! changes of the one block it follows; plus the pure pieces of the feed's
//! wire (cursor, frame truncation, SSE framing) the route assembles.
//!
//! Nothing is buffered without bound for a slow device: a subscription counts
//! the bytes queued for it, and once more than [`MAX_BEHIND_BYTES`] wait it
//! stops queueing and receives one [`HubEvent::Behind`], on which the feed
//! sends `reset` and closes (the device reconnects and gets a snapshot).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use axum::body::Bytes;
use base64::Engine as _;
use serde::Serialize;
use tokio::sync::mpsc;

use crate::backend::mps::{self, MuxEvent, StreamPos};

/// A stream more than this far behind is reset and closed.
pub const MAX_BEHIND_BYTES: usize = 1024 * 1024;
/// A frame longer than this is replaced by an `amx_truncated` frame.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;
/// How much of a truncated frame is kept, in characters.
pub const TRUNCATED_HEAD_CHARS: usize = 2048;

/// About how many bytes `records` cost on the wire: each line as the device
/// gets it, so at most [`MAX_FRAME_BYTES`] however large it is written. One
/// 5 MB tool result is 64 KB behind, not 5 MB.
pub fn wire_size(records: &[u8]) -> usize {
    records.split(|b| *b == b'\n').map(|line| line.len().min(MAX_FRAME_BYTES) + 1).sum()
}

/// What a subscription receives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HubEvent {
    /// Records appended to the block's transcript, as published: where they
    /// landed per stream, and the records themselves.
    Append { pos: Vec<StreamPos>, records: Vec<u8> },
    /// The transcript was replaced, deleted or otherwise rewritten as a
    /// whole (`fileop`).
    Rewritten { fileop: String },
    /// Too much was waiting; nothing more will be queued.
    Behind,
}

struct Subscriber {
    id: u64,
    tx: mpsc::UnboundedSender<HubEvent>,
    /// Bytes queued and not yet received. With `behind`, what bounds `tx`.
    pending: Arc<AtomicUsize>,
    behind: Arc<AtomicBool>,
}

/// Every open feed's subscription, by the block it follows.
pub struct FeedHub {
    subs: parking_lot::RwLock<HashMap<String, Vec<Subscriber>>>,
    next_id: AtomicU64,
    max_behind: usize,
}

impl Default for FeedHub {
    fn default() -> Self {
        Self::with_max_behind(MAX_BEHIND_BYTES)
    }
}

impl FeedHub {
    pub fn with_max_behind(max_behind: usize) -> Self {
        Self {
            subs: parking_lot::RwLock::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            max_behind,
        }
    }

    /// Watch `broker` for transcript events. Called once per broker.
    pub fn install(self: &Arc<Self>, broker: &mps::Broker) {
        let hub = Arc::clone(self);
        broker.add_observer(Arc::new(move |event: &MuxEvent| hub.observe(event)));
    }

    /// Start receiving `block_id`'s transcript changes. Subscribe before
    /// reading the snapshot, so nothing published in between is missed; the
    /// feed drops what the snapshot already holds by line number.
    pub fn subscribe(self: &Arc<Self>, block_id: &str) -> FeedSubscription {
        let (tx, rx) = mpsc::unbounded_channel();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let pending = Arc::new(AtomicUsize::new(0));
        let behind = Arc::new(AtomicBool::new(false));
        self.subs.write().entry(block_id.to_string()).or_default().push(Subscriber {
            id,
            tx,
            pending: Arc::clone(&pending),
            behind,
        });
        FeedSubscription { hub: Arc::clone(self), block_id: block_id.to_string(), id, rx, pending }
    }

    #[cfg(test)]
    pub fn subscribers(&self, block_id: &str) -> usize {
        self.subs.read().get(block_id).map_or(0, Vec::len)
    }

    fn unsubscribe(&self, block_id: &str, id: u64) {
        let mut subs = self.subs.write();
        if let Some(list) = subs.get_mut(block_id) {
            list.retain(|s| s.id != id);
            if list.is_empty() {
                subs.remove(block_id);
            }
        }
    }

    /// The broker observer. Runs on every publish, so it returns at once for
    /// anything but a transcript event of a followed block, and never blocks.
    pub fn observe(&self, event: &MuxEvent) {
        if event.event != mps::EVENT_BLOCK_FILE {
            return;
        }
        let Some(data) = event.data.as_ref() else { return };
        if data.get("filename").and_then(|v| v.as_str()) != Some(crate::backend::agent_session::OUTPUT_FILE) {
            return;
        }
        let Some(block_id) = data.get("zoneid").and_then(|v| v.as_str()) else { return };
        let subs = self.subs.read();
        let Some(list) = subs.get(block_id) else { return };
        let Ok(file) = serde_json::from_value::<mps::WSFileEventData>(data.clone()) else { return };
        let event = if file.fileop == mps::FILE_OP_APPEND {
            let Ok(records) = base64::engine::general_purpose::STANDARD.decode(file.data64.as_bytes()) else {
                return;
            };
            HubEvent::Append { pos: file.pos, records }
        } else {
            HubEvent::Rewritten { fileop: file.fileop }
        };
        let size = match &event {
            HubEvent::Append { records, .. } => wire_size(records),
            _ => 0,
        };
        for sub in list {
            if sub.behind.load(Ordering::Relaxed) {
                continue;
            }
            if sub.pending.load(Ordering::Relaxed) + size > self.max_behind {
                sub.behind.store(true, Ordering::Relaxed);
                let _ = sub.tx.send(HubEvent::Behind);
                continue;
            }
            sub.pending.fetch_add(size, Ordering::Relaxed);
            let _ = sub.tx.send(event.clone());
        }
    }
}

/// One feed's subscription; unsubscribes when dropped.
pub struct FeedSubscription {
    hub: Arc<FeedHub>,
    block_id: String,
    id: u64,
    rx: mpsc::UnboundedReceiver<HubEvent>,
    pending: Arc<AtomicUsize>,
}

impl FeedSubscription {
    pub async fn recv(&mut self) -> Option<HubEvent> {
        let event = self.rx.recv().await?;
        if let HubEvent::Append { records, .. } = &event {
            self.pending.fetch_sub(wire_size(records), Ordering::Relaxed);
        }
        Some(event)
    }
}

impl Drop for FeedSubscription {
    fn drop(&mut self) {
        self.hub.unsubscribe(&self.block_id, self.id);
    }
}

/// Where a feed stands in its stream: the generation it is reading and the
/// next line it has not sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cursor {
    /// `b:<blockId>`, or `g:<zone>` when the transcript is read from the
    /// agent's global zone.
    pub stream: String,
    /// '' while the transcript does not exist yet.
    pub gen: String,
    pub next_line: u64,
}

impl Cursor {
    /// `<gen>:<next line>`, the SSE `id` a reconnect sends back.
    pub fn event_id(&self) -> String {
        format!("{}:{}", self.gen, self.next_line)
    }
}

/// `Last-Event-ID` read back: `(gen, next line)`.
pub fn parse_event_id(id: &str) -> Option<(String, u64)> {
    let (gen, line) = id.trim().rsplit_once(':')?;
    if gen.is_empty() {
        return None;
    }
    Some((gen.to_string(), line.parse().ok()?))
}

/// What to do with one published append.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppendStep {
    /// Send these lines, starting at `line`; the cursor has moved past them.
    Send { line: u64, lines: Vec<String> },
    /// Nothing new for this feed (another stream's event, or lines already sent).
    Skip,
    /// The feed can't continue from here (another generation, a gap, a count
    /// that doesn't match): reset and send a fresh snapshot.
    Resync(&'static str),
}

/// The lines in published `records`, split the way `blockfile:read_range`
/// splits the file, so the counts agree.
pub fn split_records(records: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(records)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(str::to_string)
        .collect()
}

/// Apply one published append to `cursor`.
pub fn apply_append(cursor: &mut Cursor, pos: &[StreamPos], records: &[u8]) -> AppendStep {
    let Some(pos) = pos.iter().find(|p| p.stream == cursor.stream) else {
        return AppendStep::Skip;
    };
    if pos.gen != cursor.gen {
        // A transcript that didn't exist at the snapshot: its first
        // generation simply continues from line 0.
        if cursor.gen.is_empty() && cursor.next_line == 0 {
            cursor.gen = pos.gen.clone();
        } else {
            return AppendStep::Resync("gen");
        }
    }
    if pos.lines <= cursor.next_line {
        return AppendStep::Skip;
    }
    if pos.line > cursor.next_line {
        return AppendStep::Resync("gap");
    }
    let mut lines = split_records(records);
    if lines.len() as u64 != pos.lines - pos.line {
        return AppendStep::Resync("count");
    }
    let skip = (cursor.next_line - pos.line) as usize;
    lines.drain(..skip);
    let line = cursor.next_line;
    cursor.next_line = pos.lines;
    AppendStep::Send { line, lines }
}

/// A frame as the device gets it: as written, or, over
/// [`MAX_FRAME_BYTES`], an `amx_truncated` frame with its size and head.
pub fn device_frame(line: String) -> String {
    if line.len() <= MAX_FRAME_BYTES {
        return line;
    }
    let head: String = line.chars().take(TRUNCATED_HEAD_CHARS).collect();
    serde_json::json!({ "type": "amx_truncated", "bytes": line.len(), "head": head }).to_string()
}

/// [`device_frame`] over a batch.
pub fn device_frames(lines: Vec<String>) -> Vec<String> {
    lines.into_iter().map(device_frame).collect()
}

/// One SSE event. `data` is serde output on one line, so no raw newline can
/// end the `data:` field early.
pub fn sse_event(name: &str, id: Option<&str>, data: &impl Serialize) -> Bytes {
    let json = serde_json::to_string(data).unwrap_or_else(|_| "{}".to_string());
    match id {
        Some(id) => Bytes::from(format!("event: {name}\nid: {id}\ndata: {json}\n\n")),
        None => Bytes::from(format!("event: {name}\ndata: {json}\n\n")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(stream: &str, gen: &str, line: u64, lines: u64) -> Vec<StreamPos> {
        vec![StreamPos { stream: stream.into(), gen: gen.into(), line, lines }]
    }

    fn cursor(gen: &str, next: u64) -> Cursor {
        Cursor { stream: "b:blk".into(), gen: gen.into(), next_line: next }
    }

    #[test]
    fn appends_continue_the_cursor_and_drop_what_was_sent() {
        let mut c = cursor("g1", 3);
        let step = apply_append(&mut c, &pos("b:blk", "g1", 3, 5), b"{\"a\":1}\n{\"a\":2}\n");
        assert_eq!(step, AppendStep::Send { line: 3, lines: vec!["{\"a\":1}".into(), "{\"a\":2}".into()] });
        assert_eq!(c.next_line, 5);

        // Already sent (the snapshot held it).
        assert_eq!(apply_append(&mut c, &pos("b:blk", "g1", 4, 5), b"x\n"), AppendStep::Skip);
        // Half sent: only the new part goes out.
        let step = apply_append(&mut c, &pos("b:blk", "g1", 4, 7), b"x\ny\nz\n");
        assert_eq!(step, AppendStep::Send { line: 5, lines: vec!["y".into(), "z".into()] });
        assert_eq!(c.event_id(), "g1:7");
    }

    #[test]
    fn another_generation_a_gap_or_a_bad_count_resyncs() {
        let mut c = cursor("g1", 3);
        assert_eq!(apply_append(&mut c, &pos("b:blk", "g2", 0, 1), b"x\n"), AppendStep::Resync("gen"));
        assert_eq!(apply_append(&mut c, &pos("b:blk", "g1", 4, 5), b"x\n"), AppendStep::Resync("gap"));
        assert_eq!(apply_append(&mut c, &pos("b:blk", "g1", 3, 5), b"x\n"), AppendStep::Resync("count"));
        assert_eq!(c.next_line, 3, "nothing moved");
    }

    #[test]
    fn other_streams_are_ignored_and_a_new_transcript_is_adopted() {
        let mut c = cursor("g1", 3);
        assert_eq!(apply_append(&mut c, &pos("g:agent:x:current", "zz", 0, 9), b"x\n"), AppendStep::Skip);
        let mut fresh = cursor("", 0);
        let step = apply_append(&mut fresh, &pos("b:blk", "g9", 0, 1), b"first\n");
        assert_eq!(step, AppendStep::Send { line: 0, lines: vec!["first".into()] });
        assert_eq!(fresh.gen, "g9");
    }

    #[test]
    fn event_ids_round_trip() {
        assert_eq!(parse_event_id("ab12:40"), Some(("ab12".into(), 40)));
        assert_eq!(parse_event_id(" ab12:0 "), Some(("ab12".into(), 0)));
        for bad in ["", ":4", "ab12", "ab12:x", "ab12:-1"] {
            assert_eq!(parse_event_id(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_frame_over_64_kb_is_replaced_by_its_head() {
        let small = "x".repeat(MAX_FRAME_BYTES);
        assert_eq!(device_frame(small.clone()), small);
        let big = format!("{{\"type\":\"user\",\"text\":\"{}\"}}", "é".repeat(5 * 1024 * 1024 / 2));
        let frame = device_frame(big.clone());
        assert!(frame.len() < MAX_FRAME_BYTES);
        let v: serde_json::Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(v["type"], "amx_truncated");
        assert_eq!(v["bytes"], big.len());
        let head = v["head"].as_str().unwrap();
        assert_eq!(head.chars().count(), TRUNCATED_HEAD_CHARS);
        assert!(big.starts_with(head));
    }

    #[test]
    fn a_huge_line_counts_as_the_frame_the_device_gets() {
        let huge = format!("{}\n", "x".repeat(5 * 1024 * 1024));
        assert!(wire_size(huge.as_bytes()) <= MAX_FRAME_BYTES + 2);
        assert_eq!(wire_size(b"ab\ncd\n"), 3 + 3 + 1);
    }

    #[test]
    fn sse_events_are_one_data_line() {
        let ev = sse_event("append", Some("g:3"), &serde_json::json!({"lines": ["a\nb"]}));
        let text = String::from_utf8(ev.to_vec()).unwrap();
        assert_eq!(text, "event: append\nid: g:3\ndata: {\"lines\":[\"a\\nb\"]}\n\n");
        let reset = sse_event("reset", None, &serde_json::json!({"reason": "gen"}));
        assert_eq!(String::from_utf8(reset.to_vec()).unwrap(), "event: reset\ndata: {\"reason\":\"gen\"}\n\n");
    }

    fn blockfile(block: &str, fileop: &str, records: &[u8], pos: Vec<StreamPos>) -> MuxEvent {
        let data = mps::WSFileEventData {
            zoneid: block.into(),
            filename: "output".into(),
            fileop: fileop.into(),
            data64: base64::engine::general_purpose::STANDARD.encode(records),
            offset: None,
            pos,
            echo: None,
        };
        MuxEvent {
            event: mps::EVENT_BLOCK_FILE.into(),
            scopes: vec![format!("block:{block}")],
            sender: String::new(),
            persist: 0,
            data: serde_json::to_value(&data).ok(),
        }
    }

    #[tokio::test]
    async fn the_hub_delivers_only_the_followed_blocks_transcript() {
        let broker = mps::Broker::new();
        let hub = Arc::new(FeedHub::default());
        hub.install(&broker);
        let mut sub = hub.subscribe("b1");
        broker.publish(blockfile("b2", "append", b"other\n", pos("b:b2", "g", 0, 1)));
        let mut term = blockfile("b1", "append", b"tty", vec![]);
        if let Some(d) = term.data.as_mut() {
            d["filename"] = "term".into();
        }
        broker.publish(term);
        broker.publish(blockfile("b1", "append", b"mine\n", pos("b:b1", "g", 0, 1)));
        broker.publish(blockfile("b1", "replace", b"", vec![]));
        assert_eq!(
            sub.recv().await,
            Some(HubEvent::Append { pos: pos("b:b1", "g", 0, 1), records: b"mine\n".to_vec() })
        );
        assert_eq!(sub.recv().await, Some(HubEvent::Rewritten { fileop: "replace".into() }));
        assert_eq!(hub.subscribers("b1"), 1);
        drop(sub);
        assert_eq!(hub.subscribers("b1"), 0, "dropping the subscription unsubscribes");
    }

    #[tokio::test]
    async fn a_subscription_that_falls_behind_gets_behind_once_and_nothing_more() {
        let broker = mps::Broker::new();
        let hub = Arc::new(FeedHub::with_max_behind(10));
        hub.install(&broker);
        let mut sub = hub.subscribe("b1");
        for i in 0..5u64 {
            broker.publish(blockfile("b1", "append", b"12345\n", pos("b:b1", "g", i, i + 1)));
        }
        assert!(matches!(sub.recv().await, Some(HubEvent::Append { .. })));
        assert_eq!(sub.recv().await, Some(HubEvent::Behind), "the second append would pass 10 bytes");
        broker.publish(blockfile("b1", "append", b"x\n", pos("b:b1", "g", 5, 6)));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), sub.recv()).await.is_err(),
            "nothing is queued after Behind"
        );
    }
}
