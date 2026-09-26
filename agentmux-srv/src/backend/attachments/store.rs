// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The on-disk attachment store: content-addressed originals, derived files,
//! and the retention sweep. SPEC_AGENT_PANE_IMAGE_ATTACHMENTS_2026_09_26.md §6.3.
//!
//! ```text
//! <root>/blobs/ab/<sha256>.<ext>       original bytes
//! <root>/derived/ab/<sha256>.<fp>.thumb.<ext>
//! <root>/derived/ab/<sha256>.<fp>.send.<ext>
//! <root>/derived/ab/<sha256>.<fp>.json   StoredMeta
//! <root>/derived/ab/<sha256>.sent        marker: sent to an agent at least once
//! <root>/incoming/<uuid>.part          in-flight copy / upload
//! <root>/sessions/<sha256(key)>.json     base64 inlined into one Claude session
//! ```
//!
//! `<fp>` is the transform fingerprint ([`fingerprint`]): derived bytes
//! depend on the send edge and the pipeline, not only on the original, so
//! a settings change re-derives instead of reusing stale files.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::process::{self, Derived};
use crate::backend::rpc_types::AttachmentInfo;

/// Bump when the derive pipeline's output changes, so existing derived
/// files stop being reused.
pub const DERIVE_VERSION: u32 = 1;

/// The key derived files are stored under for a given send edge.
pub fn fingerprint(send_max_edge: u32) -> String {
    format!("v{DERIVE_VERSION}-e{send_max_edge}")
}

/// `incoming/` files older than this are leftovers from a crash or a
/// cancelled copy.
const INCOMING_MAX_AGE: Duration = Duration::from_secs(60 * 60);

/// Attachments never sent to an agent (drafts, abandoned or lost on restart)
/// are kept at most this long, instead of the full retention window.
pub const UNSENT_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// Hex SHA-256, lowercase. Every public entry point that turns an id into a
/// path validates it with this first, so an id can never carry a separator.
pub fn is_valid_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// What `derived/<id>.json` records about an attachment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredMeta {
    pub v: u32,
    pub mime: String,
    pub ext: String,
    pub bytes: u64,
    pub width: u32,
    pub height: u32,
    pub send_mime: String,
    pub send_ext: String,
    pub send_bytes: u64,
    pub send_width: u32,
    pub send_height: u32,
    pub thumb_mime: String,
    pub thumb_ext: String,
    #[serde(default)]
    pub first_frame_only: bool,
}

impl StoredMeta {
    pub fn to_info(&self, id: &str, name: &str) -> AttachmentInfo {
        AttachmentInfo {
            id: id.to_string(),
            name: name.to_string(),
            mime: self.mime.clone(),
            bytes: self.bytes,
            width: self.width,
            height: self.height,
            send_mime: self.send_mime.clone(),
            send_bytes: self.send_bytes,
            send_width: self.send_width,
            send_height: self.send_height,
            first_frame_only: self.first_frame_only,
        }
    }
}

/// Which stored file to serve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Original,
    Thumb,
    Send,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "original" => Some(Kind::Original),
            "thumb" => Some(Kind::Thumb),
            "send" => Some(Kind::Send),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Create the directory tree, owner-only.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        let _ = agentmux_common::data_paths::ensure_owner_only_dir(&self.root);
        for sub in ["blobs", "derived", "incoming", "sessions"] {
            std::fs::create_dir_all(self.root.join(sub))?;
        }
        Ok(())
    }

    fn session_path(&self, key: &str) -> PathBuf {
        let hashed = hex::encode(Sha256::digest(key.as_bytes()));
        self.root.join("sessions").join(format!("{hashed}.json"))
    }

    /// Base64 bytes already inlined into the Claude session `key`. Kept on
    /// disk so a restart doesn't reset it: Claude Code stores every inline
    /// image in its own session transcript, and this is what bounds that.
    pub fn session_inline_bytes(&self, key: &str) -> u64 {
        std::fs::read_to_string(self.session_path(key))
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| v.get("bytes").and_then(|b| b.as_u64()))
            .unwrap_or(0)
    }

    pub fn add_session_inline_bytes(&self, key: &str, bytes: u64) {
        if bytes == 0 {
            return;
        }
        let total = self.session_inline_bytes(key).saturating_add(bytes);
        let path = self.session_path(key);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = write_atomic(
            &path,
            serde_json::json!({ "bytes": total }).to_string().as_bytes(),
        );
    }

    fn shard(id: &str) -> &str {
        &id[..2]
    }

    pub fn blob_path(&self, id: &str, ext: &str) -> PathBuf {
        self.root
            .join("blobs")
            .join(Self::shard(id))
            .join(format!("{id}.{ext}"))
    }

    fn derived_path(&self, id: &str, fp: &str, suffix: &str) -> PathBuf {
        self.root
            .join("derived")
            .join(Self::shard(id))
            .join(format!("{id}.{fp}.{suffix}"))
    }

    fn meta_path(&self, id: &str, fp: &str) -> PathBuf {
        self.derived_path(id, fp, "json")
    }

    /// The stored original for `id` and its format, found by content.
    pub fn find_blob(&self, id: &str) -> Option<(PathBuf, image::ImageFormat)> {
        if !is_valid_id(id) {
            return None;
        }
        let dir = self.root.join("blobs").join(Self::shard(id));
        let prefix = format!("{id}.");
        let path = std::fs::read_dir(dir)
            .ok()?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(&prefix))
            })?;
        match process::sniff_file(&path).ok()? {
            process::Sniffed::Image(f) => Some((path, f)),
            _ => None,
        }
    }

    pub fn new_incoming_path(&self) -> PathBuf {
        self.root
            .join("incoming")
            .join(format!("{}.part", uuid::Uuid::new_v4()))
    }

    /// Stored metadata for fingerprint `fp`, or `None` when the id is
    /// invalid, not in the store, or not yet derived for `fp`.
    pub fn meta(&self, id: &str, fp: &str) -> Option<StoredMeta> {
        if !is_valid_id(id) {
            return None;
        }
        let text = std::fs::read_to_string(self.meta_path(id, fp)).ok()?;
        let meta: StoredMeta = serde_json::from_str(&text).ok()?;
        // All three files must still exist; the sweep can race a lookup.
        let complete = self.blob_path(id, &meta.ext).is_file()
            && self
                .derived_path(id, fp, &format!("send.{}", meta.send_ext))
                .is_file()
            && self
                .derived_path(id, fp, &format!("thumb.{}", meta.thumb_ext))
                .is_file();
        complete.then_some(meta)
    }

    /// Path and MIME type of one stored file.
    pub fn file(&self, id: &str, fp: &str, kind: Kind) -> Option<(PathBuf, String)> {
        let meta = self.meta(id, fp)?;
        Some(match kind {
            Kind::Original => (self.blob_path(id, &meta.ext), meta.mime),
            Kind::Thumb => (
                self.derived_path(id, fp, &format!("thumb.{}", meta.thumb_ext)),
                meta.thumb_mime,
            ),
            Kind::Send => (
                self.derived_path(id, fp, &format!("send.{}", meta.send_ext)),
                meta.send_mime,
            ),
        })
    }

    /// Mark an attachment as used now, so the retention sweep keeps it.
    pub fn touch(&self, id: &str, fp: &str) {
        let Some(meta) = self.meta(id, fp) else {
            return;
        };
        let now = filetime_now();
        for p in [
            self.blob_path(id, &meta.ext),
            self.meta_path(id, fp),
            self.derived_path(id, fp, &format!("send.{}", meta.send_ext)),
            self.derived_path(id, fp, &format!("thumb.{}", meta.thumb_ext)),
            self.sent_marker(id),
        ] {
            let _ = set_mtime(&p, now);
        }
    }

    fn sent_marker(&self, id: &str) -> PathBuf {
        self.root
            .join("derived")
            .join(Self::shard(id))
            .join(format!("{id}.sent"))
    }

    /// Record that an attachment went to an agent, which moves it from the
    /// 7-day unsent window to the full retention window.
    pub fn mark_sent(&self, id: &str) {
        if !is_valid_id(id) {
            return;
        }
        let marker = self.sent_marker(id);
        if let Some(dir) = marker.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&marker, b"");
    }

    fn is_sent(&self, id: &str) -> bool {
        self.sent_marker(id).is_file()
    }

    /// Remove the original of an attachment that has no metadata under any
    /// fingerprint, i.e. one whose processing failed after [`Self::place`]
    /// stored it. Without this, a file that sniffs as an image but won't
    /// decode would sit in `blobs/` until the retention sweep.
    pub fn discard_if_unprocessed(&self, id: &str) {
        if !is_valid_id(id) {
            return;
        }
        let prefix = format!("{id}.");
        let names = |sub: &str| -> Vec<PathBuf> {
            std::fs::read_dir(self.root.join(sub).join(Self::shard(id)))
                .map(|rd| {
                    rd.filter_map(Result::ok)
                        .map(|e| e.path())
                        .filter(|p| {
                            p.file_name()
                                .and_then(|n| n.to_str())
                                .is_some_and(|n| n.starts_with(&prefix))
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        let processed = names("derived")
            .iter()
            .any(|p| p.extension().is_some_and(|e| e == "json"));
        if processed {
            return;
        }
        for blob in names("blobs") {
            let _ = std::fs::remove_file(blob);
        }
    }

    /// Copy `src` into `incoming/`, hashing as it goes. `on_progress` is
    /// called with the bytes copied so far; returning `false` cancels.
    /// Returns the incoming path and the hex SHA-256.
    pub fn copy_in(
        &self,
        src: &Path,
        mut on_progress: impl FnMut(u64) -> bool,
    ) -> std::io::Result<(PathBuf, String, u64)> {
        let mut input = std::fs::File::open(src)?;
        let tmp = self.new_incoming_path();
        let result = (|| {
            let mut out = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
            let mut hasher = Sha256::new();
            let mut buf = vec![0u8; 1024 * 1024];
            let mut done = 0u64;
            loop {
                let n = input.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
                out.write_all(&buf[..n])?;
                done += n as u64;
                if !on_progress(done) {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Interrupted,
                        "cancelled",
                    ));
                }
            }
            out.flush()?;
            Ok((hex::encode(hasher.finalize()), done))
        })();
        match result {
            Ok((id, n)) => Ok((tmp, id, n)),
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                Err(e)
            }
        }
    }

    /// Move a hashed file from `incoming/` into `blobs/`, or drop it when the
    /// same bytes are already stored. Refuses anything that isn't a
    /// decodable image format. Returns the blob path and its format.
    pub fn place(
        &self,
        incoming: &Path,
        id: &str,
    ) -> Result<(PathBuf, image::ImageFormat), process::ProcessError> {
        let cleanup = || {
            let _ = std::fs::remove_file(incoming);
        };
        let sniffed = process::sniff_file(incoming).map_err(|e| {
            cleanup();
            io_error(e)
        })?;
        if let Some(e) = process::unsupported_reason(sniffed) {
            cleanup();
            return Err(e);
        }
        let process::Sniffed::Image(format) = sniffed else {
            unreachable!()
        };
        let blob = self.blob_path(id, process::ext_of(format));
        let placed = (|| {
            std::fs::create_dir_all(blob.parent().unwrap())?;
            if blob.is_file() {
                std::fs::remove_file(incoming)?;
                set_mtime(&blob, filetime_now())
            } else {
                std::fs::rename(incoming, &blob)
            }
        })();
        placed.map_err(|e| {
            cleanup();
            io_error(e)
        })?;
        Ok((blob, format))
    }

    /// Decode a stored original and write its thumbnail, send-copy and
    /// metadata under fingerprint `fingerprint(send_max_edge)`.
    pub fn derive_and_save(
        &self,
        id: &str,
        blob: &Path,
        format: image::ImageFormat,
        send_max_edge: u32,
    ) -> Result<StoredMeta, process::ProcessError> {
        let fp = fingerprint(send_max_edge);
        let derived: Derived = process::derive(blob, format, send_max_edge)?;
        let bytes = std::fs::metadata(blob).map(|m| m.len()).map_err(io_error)?;
        let meta = StoredMeta {
            v: DERIVE_VERSION,
            mime: process::mime_of(format).to_string(),
            ext: process::ext_of(format).to_string(),
            bytes,
            width: derived.width,
            height: derived.height,
            send_mime: process::mime_of(derived.send.format).to_string(),
            send_ext: process::ext_of(derived.send.format).to_string(),
            send_bytes: derived.send.bytes.len() as u64,
            send_width: derived.send.width,
            send_height: derived.send.height,
            thumb_mime: process::mime_of(derived.thumb.format).to_string(),
            thumb_ext: process::ext_of(derived.thumb.format).to_string(),
            first_frame_only: derived.first_frame_only,
        };
        let write_all = || -> std::io::Result<()> {
            let meta_path = self.meta_path(id, &fp);
            std::fs::create_dir_all(meta_path.parent().unwrap())?;
            write_atomic(
                &self.derived_path(id, &fp, &format!("send.{}", meta.send_ext)),
                &derived.send.bytes,
            )?;
            write_atomic(
                &self.derived_path(id, &fp, &format!("thumb.{}", meta.thumb_ext)),
                &derived.thumb.bytes,
            )?;
            // Metadata last: its presence is what makes the attachment visible.
            write_atomic(&meta_path, serde_json::to_string(&meta).unwrap().as_bytes())
        };
        write_all().map_err(io_error)?;
        Ok(meta)
    }

    /// [`Self::place`] then derive, reusing derived files when they exist.
    /// Test convenience: the service calls the two steps separately so it
    /// can reserve decode memory in between.
    #[cfg(test)]
    pub fn commit(
        &self,
        incoming: &Path,
        id: &str,
        send_max_edge: u32,
    ) -> Result<StoredMeta, process::ProcessError> {
        let (blob, format) = self.place(incoming, id)?;
        let fp = fingerprint(send_max_edge);
        if let Some(meta) = self.meta(id, &fp) {
            self.touch(id, &fp);
            return Ok(meta);
        }
        self.derive_and_save(id, &blob, format, send_max_edge)
    }

    /// Metadata for `id` under the current send edge, re-deriving from the
    /// stored original when the fingerprint changed since it was processed.
    /// Test helper: production code goes through `Service::ensure_derived`,
    /// which applies the CPU and memory limits.
    #[cfg(test)]
    pub fn ensure_derived(&self, id: &str, send_max_edge: u32) -> Option<StoredMeta> {
        let fp = fingerprint(send_max_edge);
        if let Some(meta) = self.meta(id, &fp) {
            return Some(meta);
        }
        let (blob, format) = self.find_blob(id)?;
        self.derive_and_save(id, &blob, format, send_max_edge).ok()
    }

    /// Delete attachment files unused for `retention` (or for
    /// [`UNSENT_MAX_AGE`] when the attachment was never sent), and
    /// `incoming/` leftovers older than an hour. Returns how many files were
    /// removed.
    pub fn sweep(&self, retention: Duration) -> usize {
        let now = SystemTime::now();
        let unsent = UNSENT_MAX_AGE.min(retention);
        let mut sent_cache: std::collections::HashMap<String, bool> =
            std::collections::HashMap::new();
        let mut removed = 0;
        for sub in ["blobs", "derived", "incoming", "sessions"] {
            let dir = self.root.join(sub);
            let Ok(entries) = walk_files(&dir) else {
                continue;
            };
            for path in entries {
                let max_age = if sub == "incoming" {
                    INCOMING_MAX_AGE
                } else if sub == "sessions" {
                    // Session counters are named by a hash, not an id.
                    retention
                } else {
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    match name.get(..64).filter(|id| is_valid_id(id)) {
                        Some(id) => {
                            let sent = *sent_cache
                                .entry(id.to_string())
                                .or_insert_with(|| self.is_sent(id));
                            if sent {
                                retention
                            } else {
                                unsent
                            }
                        }
                        None => retention,
                    }
                };
                let old = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| now.duration_since(t).ok())
                    .map(|age| age > max_age)
                    .unwrap_or(false);
                if old && std::fs::remove_file(&path).is_ok() {
                    removed += 1;
                }
            }
        }
        removed
    }
}

fn io_error(e: std::io::Error) -> process::ProcessError {
    process::ProcessError {
        code: "io",
        message: e.to_string(),
    }
}

fn walk_files(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        if ft.is_dir() {
            if let Ok(mut inner) = walk_files(&entry.path()) {
                out.append(&mut inner);
            }
        } else if ft.is_file() {
            out.push(entry.path());
        }
    }
    Ok(out)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

fn filetime_now() -> SystemTime {
    SystemTime::now()
}

fn set_mtime(path: &Path, t: SystemTime) -> std::io::Result<()> {
    let f = std::fs::OpenOptions::new().write(true).open(path)?;
    f.set_modified(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageFormat, Rgb, RgbImage};

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::new(dir.path().join("attachments"));
        s.ensure_dirs().unwrap();
        (dir, s)
    }

    fn png(dir: &Path, name: &str, shade: u8) -> PathBuf {
        let p = dir.join(name);
        DynamicImage::ImageRgb8(RgbImage::from_pixel(40, 30, Rgb([shade, 0, 0])))
            .save_with_format(&p, ImageFormat::Png)
            .unwrap();
        p
    }

    fn ingest(s: &Store, src: &Path) -> Result<(String, StoredMeta), process::ProcessError> {
        let (tmp, id, _) = s.copy_in(src, |_| true).unwrap();
        s.commit(&tmp, &id, 2000).map(|m| (id, m))
    }

    #[test]
    fn id_validation_rejects_anything_path_like() {
        assert!(is_valid_id(&"a".repeat(64)));
        assert!(!is_valid_id(&"A".repeat(64)));
        assert!(!is_valid_id("../../etc/passwd"));
        assert!(!is_valid_id(&format!("{}/", "a".repeat(63))));
        assert!(!is_valid_id(""));
    }

    #[test]
    fn ingest_stores_original_thumb_send_and_meta() {
        let (dir, s) = store();
        let src = png(dir.path(), "a.png", 10);
        let (id, meta) = ingest(&s, &src).unwrap();
        let fp = fingerprint(2000);
        assert!(is_valid_id(&id));
        assert_eq!(meta.mime, "image/png");
        assert_eq!((meta.width, meta.height), (40, 30));
        assert_eq!(s.meta(&id, &fp), Some(meta));
        for kind in [Kind::Original, Kind::Thumb, Kind::Send] {
            assert!(s.file(&id, &fp, kind).unwrap().0.is_file());
        }
        // incoming/ is empty again.
        assert_eq!(
            std::fs::read_dir(s.root().join("incoming"))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn same_bytes_are_stored_once() {
        let (dir, s) = store();
        let a = png(dir.path(), "a.png", 10);
        let b = dir.path().join("copy-of-a.png");
        std::fs::copy(&a, &b).unwrap();
        let (id1, _) = ingest(&s, &a).unwrap();
        let (id2, _) = ingest(&s, &b).unwrap();
        assert_eq!(id1, id2);
        assert_eq!(walk_files(&s.root().join("blobs")).unwrap().len(), 1);
    }

    #[test]
    fn non_image_is_refused_and_leaves_nothing_behind() {
        let (dir, s) = store();
        let src = dir.path().join("notes.txt");
        std::fs::write(&src, "hello").unwrap();
        let e = ingest(&s, &src).unwrap_err();
        assert_eq!(e.code, "unsupported");
        assert!(walk_files(&s.root()).unwrap().is_empty());
    }

    #[test]
    fn cancelled_copy_leaves_nothing_behind() {
        let (dir, s) = store();
        let src = png(dir.path(), "a.png", 10);
        assert!(s.copy_in(&src, |_| false).is_err());
        assert!(walk_files(&s.root()).unwrap().is_empty());
    }

    #[test]
    fn a_failed_decode_is_discarded_but_a_processed_one_is_kept() {
        let (dir, s) = store();
        let bad = png(dir.path(), "bad.png", 1);
        let bytes = std::fs::read(&bad).unwrap();
        std::fs::write(&bad, &bytes[..bytes.len() / 3]).unwrap();
        let (tmp, id, _) = s.copy_in(&bad, |_| true).unwrap();
        let (blob, format) = s.place(&tmp, &id).unwrap();
        assert!(s.derive_and_save(&id, &blob, format, 2000).is_err());
        s.discard_if_unprocessed(&id);
        assert!(walk_files(&s.root().join("blobs")).unwrap().is_empty());

        let (good, _) = ingest(&s, &png(dir.path(), "good.png", 2)).unwrap();
        s.discard_if_unprocessed(&good);
        assert!(s.meta(&good, &fingerprint(2000)).is_some());
    }

    #[test]
    fn session_inline_bytes_accumulate_and_persist() {
        let (_dir, s) = store();
        assert_eq!(s.session_inline_bytes("sess-1"), 0);
        s.add_session_inline_bytes("sess-1", 1000);
        s.add_session_inline_bytes("sess-1", 500);
        assert_eq!(s.session_inline_bytes("sess-1"), 1500);
        assert_eq!(s.session_inline_bytes("sess-2"), 0);
        // A fresh Store over the same root sees it (a restart).
        let again = Store::new(s.root().to_path_buf());
        assert_eq!(again.session_inline_bytes("sess-1"), 1500);
        // Kept for the full retention window, not the 7-day unsent one.
        let then = SystemTime::now() - Duration::from_secs(8 * 24 * 3600);
        for p in walk_files(&s.root().join("sessions")).unwrap() {
            set_mtime(&p, then).unwrap();
        }
        assert_eq!(s.sweep(Duration::from_secs(30 * 24 * 3600)), 0);
        assert_eq!(s.session_inline_bytes("sess-1"), 1500);
    }

    fn age_all(s: &Store, id: &str, days: u64) {
        let then = SystemTime::now() - Duration::from_secs(days * 24 * 3600);
        for p in walk_files(s.root()).unwrap() {
            if p.to_string_lossy().contains(id) {
                set_mtime(&p, then).unwrap();
            }
        }
    }

    #[test]
    fn never_sent_attachments_go_after_seven_days_sent_ones_after_retention() {
        let (dir, s) = store();
        let (draft, _) = ingest(&s, &png(dir.path(), "draft.png", 1)).unwrap();
        let (sent, _) = ingest(&s, &png(dir.path(), "sent.png", 2)).unwrap();
        s.mark_sent(&sent);
        age_all(&s, &draft, 8);
        age_all(&s, &sent, 8);
        let thirty = Duration::from_secs(30 * 24 * 3600);
        assert_eq!(s.sweep(thirty), 4, "the draft's four files go");
        let fp = fingerprint(2000);
        assert!(s.meta(&draft, &fp).is_none());
        assert!(
            s.meta(&sent, &fp).is_some(),
            "a sent attachment keeps the full window"
        );
        age_all(&s, &sent, 31);
        assert_eq!(s.sweep(thirty), 5, "four files plus the sent marker");
        assert!(s.meta(&sent, &fp).is_none());
    }

    #[test]
    fn sweep_removes_stale_files_and_keeps_recent_ones() {
        let (dir, s) = store();
        let (old_id, _) = ingest(&s, &png(dir.path(), "old.png", 1)).unwrap();
        let (new_id, _) = ingest(&s, &png(dir.path(), "new.png", 2)).unwrap();
        let long_ago = SystemTime::now() - Duration::from_secs(40 * 24 * 3600);
        for p in walk_files(&s.root()).unwrap() {
            if p.to_string_lossy().contains(&old_id) {
                set_mtime(&p, long_ago).unwrap();
            }
        }
        let removed = s.sweep(Duration::from_secs(30 * 24 * 3600));
        assert_eq!(removed, 4);
        let fp = fingerprint(2000);
        assert!(s.meta(&old_id, &fp).is_none());
        assert!(s.meta(&new_id, &fp).is_some());
    }

    #[test]
    fn touch_rescues_an_attachment_from_the_sweep() {
        let (dir, s) = store();
        let (id, _) = ingest(&s, &png(dir.path(), "a.png", 1)).unwrap();
        let long_ago = SystemTime::now() - Duration::from_secs(40 * 24 * 3600);
        for p in walk_files(&s.root()).unwrap() {
            set_mtime(&p, long_ago).unwrap();
        }
        s.touch(&id, &fingerprint(2000));
        assert_eq!(s.sweep(Duration::from_secs(30 * 24 * 3600)), 0);
        assert!(s.meta(&id, &fingerprint(2000)).is_some());
    }

    #[test]
    fn changing_the_send_edge_rederives_from_the_stored_original() {
        let (dir, s) = store();
        let p = dir.path().join("wide.png");
        DynamicImage::ImageRgb8(RgbImage::from_pixel(1200, 600, Rgb([5, 5, 5])))
            .save_with_format(&p, ImageFormat::Png)
            .unwrap();
        let (id, first) = ingest(&s, &p).unwrap();
        assert_eq!(first.send_width, 1200);
        assert!(s.meta(&id, &fingerprint(800)).is_none());
        let again = s.ensure_derived(&id, 800).unwrap();
        assert_eq!((again.send_width, again.send_height), (800, 400));
        // The original fingerprint is untouched.
        assert_eq!(s.meta(&id, &fingerprint(2000)).unwrap().send_width, 1200);
    }
}
