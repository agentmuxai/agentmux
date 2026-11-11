// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! How many images an agent's conversation holds, read from its transcript. A model that cannot read images
//! (`catalog::Model::images`) must not be picked for a conversation that still holds one, and the pane's own count
//! only sees the part of the feed it has loaded, so srv counts in the transcript store, which keeps all of it.
//!
//! An image reaches the harness as the `<attached_files>` list srv appends to the user's message (`attachments::prompt`):
//! a found one as `N. name — <path>.send.<image extension>`, a missing one as `- name.png — (no longer available)`.
//! Only user lines are read, so an agent's own text that quotes such a list counts for nothing.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde_json::Value;

use crate::backend::storage::filestore::FileStore;

const FILE: &str = "output";
/// How much of the transcript is read at a time; a line longer than this widens the read.
const CHUNK: i64 = 4 * 1024 * 1024;
const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff"];

/// What was counted in a zone, and where the count stopped: a transcript only grows, so the next count reads on from there.
/// `created` is when the file was made: an archive deletes the file and a new conversation makes it again, and its count
/// starts at the top.
#[derive(Clone, Copy, Default)]
struct Counted {
    created: i64,
    pos: i64,
    images: u32,
}

fn cache() -> &'static Mutex<HashMap<String, Counted>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Counted>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Drops what was counted in `zone`, for a transcript rewritten in place (history repair): the file keeps its creation
/// time and may not be shorter, so nothing else tells the next count that its bytes are not the ones it read.
pub fn forget(zone: &str) {
    cache().lock().unwrap_or_else(|e| e.into_inner()).remove(zone);
}

/// The images in the user messages of `zone`'s transcript; 0 when there is none.
pub fn count_in_zone(gfs: &FileStore, zone: &str) -> Result<u32, String> {
    let Some(stat) = gfs.stat(zone, FILE).map_err(|e| format!("stat transcript: {e}"))? else {
        cache().lock().unwrap_or_else(|e| e.into_inner()).remove(zone);
        return Ok(0);
    };
    let mut done = cache().lock().unwrap_or_else(|e| e.into_inner()).get(zone).copied().unwrap_or_default();
    // Made again since the last count, or shorter than where it stopped (trimmed): count it again from the start.
    if stat.createdts != done.created || stat.size < done.pos {
        done = Counted { created: stat.createdts, ..Counted::default() };
    }
    let mut window = CHUNK;
    while done.pos < stat.size {
        let (at, data) = gfs.read_at(zone, FILE, done.pos, window).map_err(|e| format!("read transcript: {e}"))?;
        if data.is_empty() {
            break;
        }
        // Whole lines only; the last one is cut by the window unless the file ends there.
        let end = if at + data.len() as i64 >= stat.size { data.len() } else { data.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1) };
        if end == 0 {
            window = window.saturating_mul(2);
            continue;
        }
        window = CHUNK;
        done.images += data[..end].split(|b| *b == b'\n').map(images_in_line).sum::<u32>();
        done.pos = at + end as i64;
    }
    cache().lock().unwrap_or_else(|e| e.into_inner()).insert(zone.to_string(), done);
    Ok(done.images)
}

/// The images one transcript line lists, if it is a user message.
pub fn images_in_line(line: &[u8]) -> u32 {
    // Most lines name no attachment: skip parsing them.
    if !contains(line, b"<attached_") {
        return 0;
    }
    let Ok(v) = serde_json::from_slice::<Value>(line) else { return 0 };
    if v.get("type").and_then(Value::as_str) != Some("user") {
        return 0;
    }
    let mut texts = Vec::new();
    match v.pointer("/message/content") {
        Some(Value::String(s)) => texts.push(s.as_str()),
        Some(Value::Array(items)) => texts.extend(items.iter().filter_map(|i| i.get("text").and_then(Value::as_str))),
        _ => {}
    }
    texts.into_iter().map(images_in_text).sum()
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn has_image_extension(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, ext)| IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()))
}

/// The images in the `<attached_files>` (or `<attached_images>`) block at the end of a message.
fn images_in_text(text: &str) -> u32 {
    let mut in_block = false;
    let mut n = 0;
    for line in text.lines() {
        let line = line.trim();
        if line == "<attached_files>" || line == "<attached_images>" {
            in_block = true;
        } else if line == "</attached_files>" || line == "</attached_images>" {
            in_block = false;
        } else if in_block {
            if let Some(name) = line.strip_prefix("- ").and_then(|l| l.strip_suffix(" — (no longer available)")) {
                n += u32::from(has_image_extension(name));
            } else if let Some((_, path)) = line.rsplit_once(" — ") {
                // A found image is sent as its derived file: `<sha256>.<version>.send.<extension>`.
                n += u32::from(path.contains(".send.") && has_image_extension(path.trim()));
            }
        }
    }
    n
}

#[cfg(test)]
#[path = "images_tests.rs"]
mod tests;
