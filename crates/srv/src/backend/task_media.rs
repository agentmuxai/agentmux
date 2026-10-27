// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The files a Kanban task carries, on disk (docs/specs/SPEC_KANBAN_TASK_MEDIA_2026_10_25.md §3.2, §3.3).
//!
//! ```text
//! <shared>/task-media/ab/<sha256>.<ext>        the file, named by its content
//! <shared>/task-media/ab/<sha256>.thumb.png    a 256 px thumbnail of an image
//! <shared>/task-media/incoming/<uuid>.part     an upload in flight
//! ```
//!
//! It is in the shared home, not a channel's data dir, because the tasks are shared by every channel.
//! It is not the attachments store: that store sweeps files by last use, and a picture the operator
//! has not looked at yet must stay. What a task carries is in `storage/work_media.rs`.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use image::ImageFormat;

use super::attachments::process::{self, Sniffed};
use super::storage::work_media::media_kind;

/// The largest image and recording.
pub const MAX_IMAGE_BYTES: u64 = 25 * 1024 * 1024;
pub const MAX_RECORDING_BYTES: u64 = 100 * 1024 * 1024;
/// The most files in one call.
pub const MAX_FILES_PER_CALL: usize = 10;
/// Long edge of a card's thumbnail.
const THUMB_EDGE: u32 = 256;
/// An upload left in `incoming/` this long is a leftover.
const INCOMING_MAX_AGE: Duration = Duration::from_secs(60 * 60);
/// A stored file no task names, and older than this, is removed (it may be an upload not yet attached).
const ORPHAN_MAX_AGE: Duration = Duration::from_secs(60 * 60);

/// What a file is, by its content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sniff {
    pub kind: &'static str,
    pub mime: &'static str,
    pub ext: &'static str,
}

/// What the first bytes of a file say it is, if it is one the Kanban accepts: PNG, JPEG, GIF, WebP, SVG,
/// MP4, MOV, WebM. Never by the name.
pub fn sniff(head: &[u8]) -> Option<Sniff> {
    let image = |mime, ext| Some(Sniff { kind: media_kind::IMAGE, mime, ext });
    let recording = |mime, ext| Some(Sniff { kind: media_kind::RECORDING, mime, ext });
    match process::sniff(head) {
        Sniffed::Image(ImageFormat::Png) => return image("image/png", "png"),
        Sniffed::Image(ImageFormat::Jpeg) => return image("image/jpeg", "jpg"),
        Sniffed::Image(ImageFormat::Gif) => return image("image/gif", "gif"),
        Sniffed::Image(ImageFormat::WebP) => return image("image/webp", "webp"),
        Sniffed::Svg => return image("image/svg+xml", "svg"),
        // Other pictures (HEIC, AVIF, BMP, TIFF) are not shown by every pane; ask for a PNG.
        Sniffed::Image(_) | Sniffed::Heic | Sniffed::Avif => return None,
        Sniffed::NotImage => {}
    }
    if head.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return recording("video/webm", "webm");
    }
    if head.len() >= 12 && &head[4..8] == b"ftyp" {
        return match &head[8..12] {
            b"qt  " => recording("video/quicktime", "mov"),
            b"heic" | b"heix" | b"hevc" | b"heim" | b"heis" | b"hevm" | b"mif1" | b"msf1" | b"avif" | b"avis" => None,
            _ => recording("video/mp4", "mp4"),
        };
    }
    None
}

/// The largest a file of this kind can be.
pub fn max_bytes(kind: &str) -> u64 {
    if kind == media_kind::RECORDING {
        MAX_RECORDING_BYTES
    } else {
        MAX_IMAGE_BYTES
    }
}

/// The extension a stored file of this MIME type has.
pub fn ext_of_mime(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "video/webm" => "webm",
        "video/quicktime" => "mov",
        _ => "mp4",
    }
}

/// The folder, in the shared home.
#[cfg(not(test))]
pub fn default_root() -> Option<PathBuf> {
    agentmux_common::DataPaths::from_env().map(|p| p.shared_dir.join("task-media"))
}

/// In tests, a folder of the test process's own, never the real shared home.
#[cfg(test)]
pub fn default_root() -> Option<PathBuf> {
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    Some(DIR.get_or_init(|| tempfile::tempdir().unwrap()).path().join("task-media"))
}

fn valid_sha(sha: &str) -> bool {
    sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Where the file with this content is (or would be). `None` for a hash that isn't one: it can never
/// carry a separator into a path.
pub fn blob_path(root: &Path, sha: &str, mime: &str) -> Option<PathBuf> {
    valid_sha(sha).then(|| root.join(&sha[..2]).join(format!("{sha}.{}", ext_of_mime(mime))))
}

/// Where an image's thumbnail is (or would be).
pub fn thumb_path(root: &Path, sha: &str) -> Option<PathBuf> {
    valid_sha(sha).then(|| root.join(&sha[..2]).join(format!("{sha}.thumb.png")))
}

/// A new path for an upload in flight.
pub fn new_incoming_path(root: &Path) -> PathBuf {
    root.join("incoming").join(format!("{}.part", uuid::Uuid::new_v4()))
}

/// Read the first bytes of a file, enough to sniff it.
pub fn read_head(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut f = std::fs::File::open(path)?;
    let mut buf = vec![0u8; 1024];
    let mut n = 0;
    while n < buf.len() {
        let got = f.read(&mut buf[n..])?;
        if got == 0 {
            break;
        }
        n += got;
    }
    buf.truncate(n);
    Ok(buf)
}

/// Move an upload to the place its content names, unless the same content is already there. An image also
/// gets its thumbnail (a failure to make one is not the upload's: the card falls back to the file).
pub fn commit(root: &Path, tmp: &Path, sha: &str, found: &Sniff) -> std::io::Result<PathBuf> {
    let dest = blob_path(root, sha, found.mime).ok_or_else(|| std::io::Error::other("not a content hash"))?;
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if dest.exists() {
        let _ = std::fs::remove_file(tmp);
    } else if std::fs::rename(tmp, &dest).is_err() {
        std::fs::copy(tmp, &dest)?;
        let _ = std::fs::remove_file(tmp);
    }
    if found.kind == media_kind::IMAGE && found.mime != "image/svg+xml" {
        if let Some(thumb) = thumb_path(root, sha).filter(|t| !t.exists()) {
            let _ = make_thumb(&dest, &thumb);
        }
    }
    Ok(dest)
}

fn make_thumb(src: &Path, dest: &Path) -> Result<(), String> {
    process::header_dimensions(src).map_err(|e| e.to_string())?;
    let mut reader = image::ImageReader::open(src).map_err(|e| e.to_string())?.with_guessed_format().map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(process::MAX_SIDE);
    limits.max_image_height = Some(process::MAX_SIDE);
    limits.max_alloc = Some(process::MAX_PIXELS * 4 + 64 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().map_err(|e| e.to_string())?;
    img.thumbnail(THUMB_EDGE, THUMB_EDGE).save_with_format(dest, ImageFormat::Png).map_err(|e| e.to_string())
}

/// Remove the file with this content and its thumbnail. Returns whether a file went.
pub fn remove_content(root: &Path, sha: &str, mime: &str) -> bool {
    let mut gone = false;
    if let Some(p) = blob_path(root, sha, mime) {
        gone = std::fs::remove_file(p).is_ok();
    }
    if let Some(p) = thumb_path(root, sha) {
        let _ = std::fs::remove_file(p);
    }
    gone
}

/// Clear what is left behind: uploads that never finished, and stored files no task names (`named`
/// says whether one does) once they are an hour old, so an upload waiting to be attached is not lost.
/// Returns how many files went.
pub fn sweep(root: &Path, named: impl Fn(&str) -> bool) -> usize {
    let mut removed = 0;
    let old = |meta: &std::fs::Metadata, age: Duration| {
        meta.modified().ok().and_then(|m| SystemTime::now().duration_since(m).ok()).is_some_and(|a| a > age)
    };
    if let Ok(dir) = std::fs::read_dir(root.join("incoming")) {
        for e in dir.flatten() {
            if e.metadata().is_ok_and(|m| old(&m, INCOMING_MAX_AGE)) && std::fs::remove_file(e.path()).is_ok() {
                removed += 1;
            }
        }
    }
    let Ok(shards) = std::fs::read_dir(root) else { return removed };
    for shard in shards.flatten() {
        if shard.file_name() == "incoming" || !shard.path().is_dir() {
            continue;
        }
        let Ok(files) = std::fs::read_dir(shard.path()) else { continue };
        for f in files.flatten() {
            let name = f.file_name().to_string_lossy().into_owned();
            let sha = name.split('.').next().unwrap_or("");
            if !valid_sha(sha) || named(sha) {
                continue;
            }
            if f.metadata().is_ok_and(|m| old(&m, ORPHAN_MAX_AGE)) && std::fs::remove_file(f.path()).is_ok() {
                removed += 1;
            }
        }
    }
    removed
}

#[cfg(test)]
#[path = "task_media_tests.rs"]
mod tests;
