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
//! <root>/derived/ab/<sha256>.<fp>.text.txt  text version of a document
//! <root>/named/<sha256>/<file name>     per-delivery copy of the original
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

use super::extract;
use super::kind::{self, FileKind};
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
    // ── Any-file attachments (SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md) ──
    // Absent in metadata written before files existed: those are images.
    /// `FileKind::as_str`: image, svg, text, pdf, word, excel, …
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_count: Option<u32>,
    /// Extension of the derived text version (`text.<ext>`), if one exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_ext: Option<String>,
    #[serde(default)]
    pub text_bytes: u64,
    /// Why there is no text version, when one was expected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_note: Option<String>,
    #[serde(default)]
    pub macros: bool,
}

fn default_kind() -> String {
    "image".to_string()
}

impl StoredMeta {
    pub fn is_image(&self) -> bool {
        self.kind == "image"
    }

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
            kind: self.kind.clone(),
            page_count: self.page_count,
            text_bytes: self.text_bytes,
            text_note: self.text_note.clone(),
            macros: self.macros,
        }
    }
}

/// Which stored file to serve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Original,
    /// The tile's preview: the image thumbnail, a text file's first lines,
    /// or an SVG itself.
    Thumb,
    Send,
    /// The extracted text version of a document.
    Text,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "original" => Some(Kind::Original),
            "thumb" => Some(Kind::Thumb),
            "send" => Some(Kind::Send),
            "text" => Some(Kind::Text),
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

    /// Fold the counter kept under `from` into `to` and remove `from`. A new
    /// Claude session has no id on its first turn, so that turn is counted
    /// under a per-block key; once the id is known it belongs to the session.
    pub fn adopt_session_inline_bytes(&self, from: &str, to: &str) {
        let carried = self.session_inline_bytes(from);
        if carried == 0 {
            return;
        }
        self.add_session_inline_bytes(to, carried);
        let _ = std::fs::remove_file(self.session_path(from));
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

    /// The stored original for `id` and what kind of file it is.
    pub fn find_blob(&self, id: &str) -> Option<(PathBuf, FileKind)> {
        if !is_valid_id(id) {
            return None;
        }
        let path = self.existing_blob(id)?;
        // The blob's own name carries the extension it was stored with.
        let name = path.file_name()?.to_string_lossy().into_owned();
        let kind = kind::classify(&path, &name).ok()?;
        Some((path, kind))
    }

    /// Copy the original of `id` into `dir` as `name`, with the same copy a
    /// drop makes (`agentmux_common::copy_into_dir`): the name kept valid,
    /// dotfiles included, and de-conflicted exclusively. For container panes,
    /// whose agents can't see the store
    /// (SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md §7).
    pub fn copy_original_to(&self, id: &str, dir: &Path, name: &str) -> std::io::Result<PathBuf> {
        if !is_valid_id(id) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid attachment id",
            ));
        }
        let blob = self.existing_blob(id).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "attachment not found")
        })?;
        if !dir.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "working folder not found",
            ));
        }
        use agentmux_common::copy_into_dir::{copy_into_dir, CopyControl};
        copy_into_dir(&blob, dir, name, &CopyControl::default())
    }

    /// The stored original and its MIME type, whatever kind it is.
    pub fn original(&self, id: &str) -> Option<(PathBuf, String)> {
        let (path, kind) = self.find_blob(id)?;
        let mime = match kind {
            FileKind::Image(f) => process::mime_of(f).to_string(),
            _ => mime_for(kind, &blob_ext(&path)),
        };
        Some((path, mime))
    }

    /// An existing original for `id`, whatever extension it was stored with.
    fn existing_blob(&self, id: &str) -> Option<PathBuf> {
        let dir = self.root.join("blobs").join(Self::shard(id));
        let prefix = format!("{id}.");
        std::fs::read_dir(dir)
            .ok()?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(&prefix))
            })
    }

    fn named_path(&self, id: &str, name: &str) -> PathBuf {
        self.root.join("named").join(id).join(safe_file_name(name))
    }

    /// A copy of the original named as the user's file, for delivery
    /// (agents recognise files by extension, and the real name reads
    /// better than a hash). A real copy, never a hard link: an agent that
    /// edits the file it was given must not change the content-addressed
    /// original. Re-copied on every delivery, so an earlier edit never
    /// reaches the next message.
    pub fn named_link(&self, id: &str, name: &str) -> Option<PathBuf> {
        if !is_valid_id(id) {
            return None;
        }
        let target = self.existing_blob(id)?;
        let link = self.named_path(id, name);
        std::fs::create_dir_all(link.parent()?).ok()?;
        let tmp = link.with_extension(format!("part-{}", uuid::Uuid::new_v4()));
        std::fs::copy(&target, &tmp).ok()?;
        if std::fs::rename(&tmp, &link).is_err() {
            let _ = std::fs::remove_file(&tmp);
            // The old copy may be open in an agent's editor (Windows keeps it
            // locked); it still holds the right name, so keep serving it.
            return link.is_file().then_some(link);
        }
        Some(link)
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
        // Every file it names must still exist; the sweep can race a lookup.
        let complete = self.blob_path(id, &meta.ext).is_file()
            && self.meta_files(id, fp, &meta).iter().all(|p| p.is_file());
        complete.then_some(meta)
    }

    /// The derived files `meta` says exist (not the original).
    fn meta_files(&self, id: &str, fp: &str, meta: &StoredMeta) -> Vec<PathBuf> {
        let mut out = Vec::new();
        if meta.is_image() {
            out.push(self.derived_path(id, fp, &format!("send.{}", meta.send_ext)));
        }
        if !meta.thumb_ext.is_empty() {
            out.push(self.derived_path(id, fp, &format!("thumb.{}", meta.thumb_ext)));
        }
        if let Some(ext) = &meta.text_ext {
            out.push(self.derived_path(id, fp, &format!("text.{ext}")));
        }
        out
    }

    /// Path and MIME type of one stored file. Non-images have no separate
    /// send-copy (the original is sent) and, except text, no thumbnail file
    /// (an SVG is its own thumbnail).
    pub fn file(&self, id: &str, fp: &str, kind: Kind) -> Option<(PathBuf, String)> {
        let meta = self.meta(id, fp)?;
        let original = (self.blob_path(id, &meta.ext), meta.mime.clone());
        Some(match kind {
            Kind::Original => original,
            Kind::Send if !meta.is_image() => original,
            Kind::Send => (
                self.derived_path(id, fp, &format!("send.{}", meta.send_ext)),
                meta.send_mime,
            ),
            Kind::Thumb if meta.kind == "svg" => original,
            Kind::Thumb if meta.thumb_ext.is_empty() => return None,
            Kind::Thumb => (
                self.derived_path(id, fp, &format!("thumb.{}", meta.thumb_ext)),
                meta.thumb_mime,
            ),
            Kind::Text => (
                self.derived_path(id, fp, &format!("text.{}", meta.text_ext.as_deref()?)),
                "text/plain; charset=utf-8".to_string(),
            ),
        })
    }

    /// Mark an attachment as used now, so the retention sweep keeps it.
    pub fn touch(&self, id: &str, fp: &str) {
        let Some(meta) = self.meta(id, fp) else {
            return;
        };
        let now = filetime_now();
        let mut files = vec![
            self.blob_path(id, &meta.ext),
            self.meta_path(id, fp),
            self.sent_marker(id),
        ];
        files.extend(self.meta_files(id, fp, &meta));
        for p in files {
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
    /// same bytes are already stored (under whatever extension they were
    /// stored with). `name` is the user's file name, used only to classify.
    /// Returns the blob path and what kind of file it is.
    pub fn place(
        &self,
        incoming: &Path,
        id: &str,
        name: &str,
    ) -> Result<(PathBuf, FileKind), process::ProcessError> {
        let cleanup = || {
            let _ = std::fs::remove_file(incoming);
        };
        if let Some(existing) = self.existing_blob(id) {
            cleanup();
            let _ = set_mtime(&existing, filetime_now());
            let stored_name = existing
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let kind = kind::classify(&existing, &stored_name).map_err(io_error)?;
            return Ok((existing, kind));
        }
        let kind = kind::classify(incoming, name).map_err(|e| {
            cleanup();
            io_error(e)
        })?;
        let blob = self.blob_path(id, &stored_extension(kind, name));
        let placed = (|| {
            std::fs::create_dir_all(blob.parent().unwrap())?;
            std::fs::rename(incoming, &blob)
        })();
        placed.map_err(|e| {
            cleanup();
            io_error(e)
        })?;
        Ok((blob, kind))
    }

    /// Derive whatever `kind` needs and write the metadata. Images go through
    /// the image pipeline (decode, thumbnail, send-copy); other files get a
    /// text preview, a text version or PDF facts.
    pub fn derive_and_save(
        &self,
        id: &str,
        blob: &Path,
        kind: FileKind,
        name: &str,
        send_max_edge: u32,
    ) -> Result<StoredMeta, process::ProcessError> {
        match kind {
            FileKind::Image(format) => self.derive_image(id, blob, format, send_max_edge),
            _ => self.derive_file(id, blob, kind, name, send_max_edge),
        }
    }

    /// Decode a stored image and write its thumbnail, send-copy and
    /// metadata under fingerprint `fingerprint(send_max_edge)`.
    fn derive_image(
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
            ext: blob_ext(blob),
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
            kind: default_kind(),
            page_count: None,
            text_ext: None,
            text_bytes: 0,
            text_note: None,
            macros: false,
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

    /// Metadata and derived files for a non-image attachment: a preview for
    /// text, a text version for Office documents and PDFs, a page count and
    /// macro flag where they apply. Failures to extract never fail the
    /// attachment; it is still sent by path, with a note.
    fn derive_file(
        &self,
        id: &str,
        blob: &Path,
        kind: FileKind,
        name: &str,
        send_max_edge: u32,
    ) -> Result<StoredMeta, process::ProcessError> {
        let fp = fingerprint(send_max_edge);
        let bytes = std::fs::metadata(blob).map(|m| m.len()).map_err(io_error)?;
        let ext = blob_ext(blob);
        let mime = mime_for(kind, &ext);
        let mut meta = StoredMeta {
            v: DERIVE_VERSION,
            mime: mime.clone(),
            ext: ext.clone(),
            bytes,
            width: 0,
            height: 0,
            send_mime: mime,
            send_ext: ext,
            send_bytes: bytes,
            send_width: 0,
            send_height: 0,
            thumb_mime: String::new(),
            thumb_ext: String::new(),
            first_frame_only: false,
            kind: kind.as_str().to_string(),
            page_count: None,
            text_ext: None,
            text_bytes: 0,
            text_note: None,
            macros: false,
        };
        let mut thumb: Option<String> = None;
        let mut text: Option<String> = None;
        match kind {
            FileKind::Text => thumb = extract::text_preview(blob).ok(),
            FileKind::Pdf => {
                let facts = extract::pdf_facts_isolated(blob, name);
                meta.page_count = facts.pages;
                if facts.text.is_none() {
                    meta.text_note = Some(
                        "no text could be read from this PDF (it may be scanned or protected)"
                            .into(),
                    );
                }
                text = facts.text;
            }
            FileKind::Word | FileKind::Excel | FileKind::PowerPoint => {
                meta.macros = extract::has_macros(blob);
                match extract::document_text_isolated(blob, kind, name) {
                    Ok(Some(t)) => text = Some(t),
                    Ok(None) => {
                        meta.text_note = Some(match meta.ext.as_str() {
                            "doc" => "no text version; save it as .docx".to_string(),
                            "ppt" => "no text version; save it as .pptx".to_string(),
                            _ => "no text version for this format".to_string(),
                        })
                    }
                    Err(e) => meta.text_note = Some(format!("no text version: {e}")),
                }
            }
            _ => {}
        }
        let write_all = |meta: &mut StoredMeta| -> std::io::Result<()> {
            let meta_path = self.meta_path(id, &fp);
            std::fs::create_dir_all(meta_path.parent().unwrap())?;
            if let Some(t) = &thumb {
                meta.thumb_ext = "txt".into();
                meta.thumb_mime = "text/plain; charset=utf-8".into();
                write_atomic(&self.derived_path(id, &fp, "thumb.txt"), t.as_bytes())?;
            }
            if let Some(t) = &text {
                meta.text_ext = Some("txt".into());
                meta.text_bytes = t.len() as u64;
                write_atomic(&self.derived_path(id, &fp, "text.txt"), t.as_bytes())?;
            }
            write_atomic(
                &meta_path,
                serde_json::to_string(&*meta).unwrap().as_bytes(),
            )
        };
        write_all(&mut meta).map_err(io_error)?;
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
        self.commit_named(incoming, id, "file.png", send_max_edge)
    }

    /// [`Self::commit`] with the user's file name (tests).
    #[cfg(test)]
    pub fn commit_named(
        &self,
        incoming: &Path,
        id: &str,
        name: &str,
        send_max_edge: u32,
    ) -> Result<StoredMeta, process::ProcessError> {
        let (blob, kind) = self.place(incoming, id, name)?;
        let fp = fingerprint(send_max_edge);
        if let Some(meta) = self.meta(id, &fp) {
            self.touch(id, &fp);
            return Ok(meta);
        }
        self.derive_and_save(id, &blob, kind, name, send_max_edge)
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
        let (blob, kind) = self.find_blob(id)?;
        let name = blob.file_name()?.to_string_lossy().into_owned();
        self.derive_and_save(id, &blob, kind, &name, send_max_edge)
            .ok()
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
        for sub in ["blobs", "derived", "incoming", "sessions", "named"] {
            let dir = self.root.join(sub);
            let Ok(entries) = walk_files(&dir) else {
                continue;
            };
            for path in entries {
                let max_age = if sub == "incoming" {
                    INCOMING_MAX_AGE
                } else if sub == "sessions" || sub == "named" {
                    // Session counters are named by a hash; named links
                    // exist only for sent attachments.
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
                    if sub == "named" {
                        // Drop the per-id folder once its last link is gone.
                        if let Some(dir) = path.parent() {
                            let _ = std::fs::remove_dir(dir);
                        }
                    }
                }
            }
        }
        removed
    }
}

/// The extension a new original is stored under. It follows the content
/// where the content decides it (image formats, PDF, SVG, RTF), and
/// otherwise the user's extension if it fits the kind (docx vs odt vs doc,
/// mp3 vs wav, …). Text and unknown files are `bin`: the same bytes can
/// arrive as notes.txt and script.py, and agents get a correctly named
/// link per delivery instead (`named_link`).
fn stored_extension(kind: FileKind, name: &str) -> String {
    let user = kind::extension_of(name);
    let pick = |allowed: &[&str], default: &str| -> String {
        user.as_deref()
            .filter(|e| allowed.contains(e))
            .unwrap_or(default)
            .to_string()
    };
    match kind {
        FileKind::Image(f) => process::ext_of(f).to_string(),
        FileKind::Svg => "svg".into(),
        FileKind::Pdf => "pdf".into(),
        FileKind::Word => pick(&["docx", "docm", "dotx", "odt", "doc", "rtf"], "docx"),
        FileKind::Excel => pick(&["xlsx", "xlsm", "xlsb", "xls", "ods"], "xlsx"),
        FileKind::PowerPoint => pick(&["pptx", "pptm", "ppsx", "odp", "ppt"], "pptx"),
        FileKind::Archive => pick(
            &["zip", "gz", "tgz", "tar", "7z", "rar", "bz2", "xz", "zst"],
            "bin",
        ),
        FileKind::Audio => pick(
            &[
                "mp3", "wav", "flac", "m4a", "aac", "ogg", "oga", "opus", "wma", "aiff", "aif",
            ],
            "bin",
        ),
        FileKind::Video => pick(
            &[
                "mp4", "m4v", "mov", "webm", "mkv", "avi", "wmv", "flv", "mpg", "mpeg",
            ],
            "bin",
        ),
        FileKind::ImageFile => pick(&["heic", "heif", "avif"], "bin"),
        FileKind::Text | FileKind::Other => "bin".into(),
    }
}

/// The extension part of a stored blob's file name.
fn blob_ext(blob: &Path) -> String {
    blob.extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_else(|| "bin".into())
}

/// MIME type of a non-image original.
fn mime_for(kind: FileKind, ext: &str) -> String {
    match (kind, ext) {
        (FileKind::Svg, _) => "image/svg+xml",
        (FileKind::Pdf, _) => "application/pdf",
        (FileKind::Text, _) => "text/plain; charset=utf-8",
        (_, "docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        (_, "xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        (_, "pptx") => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        (_, "odt") => "application/vnd.oasis.opendocument.text",
        (_, "ods") => "application/vnd.oasis.opendocument.spreadsheet",
        (_, "odp") => "application/vnd.oasis.opendocument.presentation",
        (_, "doc") => "application/msword",
        (_, "xls") => "application/vnd.ms-excel",
        (_, "ppt") => "application/vnd.ms-powerpoint",
        (_, "rtf") => "application/rtf",
        (_, "zip") => "application/zip",
        (FileKind::Audio, e) => return format!("audio/{e}"),
        (FileKind::Video, e) => return format!("video/{e}"),
        _ => "application/octet-stream",
    }
    .to_string()
}

/// A user's file name made safe as a single path component: separators,
/// control characters and Windows-reserved characters replaced, leading
/// dots and trailing dots/spaces trimmed, length bounded.
pub fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned
        .trim_start_matches('.')
        .trim_end_matches(['.', ' '])
        .trim();
    let short: String = trimmed.chars().take(120).collect();
    if short.is_empty() {
        "file".to_string()
    } else {
        short
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
    fn copy_to_workdir_keeps_a_dotfile_name_and_deconflicts() {
        let (dir, s) = store();
        let src = png(dir.path(), "a.png", 10);
        let (id, _) = ingest(&s, &src).unwrap();
        let work = dir.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let first = s.copy_original_to(&id, &work, ".env").unwrap();
        let second = s.copy_original_to(&id, &work, ".env").unwrap();
        assert_eq!(first.file_name().unwrap(), ".env");
        assert_eq!(second.file_name().unwrap(), ".env_1");
        assert_eq!(std::fs::read(&second).unwrap(), std::fs::read(&src).unwrap());
        assert!(s.copy_original_to(&id, &work.join("missing"), "x.png").is_err());
        assert!(s.copy_original_to("../escape", &work, "x.png").is_err());
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
    fn text_file_is_stored_with_a_preview() {
        let (dir, s) = store();
        let src = dir.path().join("notes.txt");
        std::fs::write(&src, "hello").unwrap();
        let (tmp, id, _) = s.copy_in(&src, |_| true).unwrap();
        let meta = s.commit_named(&tmp, &id, "notes.txt", 2000).unwrap();
        assert_eq!(meta.kind, "text");
        assert!(!meta.is_image());
        let (thumb, _) = s.file(&id, &fingerprint(2000), Kind::Thumb).unwrap();
        assert_eq!(std::fs::read_to_string(thumb).unwrap(), "hello");
        let (send, _) = s.file(&id, &fingerprint(2000), Kind::Send).unwrap();
        assert_eq!(std::fs::read_to_string(send).unwrap(), "hello");
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
        let (blob, kind) = s.place(&tmp, &id, "bad.png").unwrap();
        assert!(s
            .derive_and_save(&id, &blob, kind, "bad.png", 2000)
            .is_err());
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
        // A first turn counted before the session had an id carries over.
        s.add_session_inline_bytes("block:b1:new", 700);
        s.adopt_session_inline_bytes("block:b1:new", "sess-1");
        assert_eq!(s.session_inline_bytes("sess-1"), 2200);
        assert_eq!(
            s.session_inline_bytes("block:b1:new"),
            0,
            "moved, not copied"
        );
        s.adopt_session_inline_bytes("block:b1:new", "sess-1");
        assert_eq!(
            s.session_inline_bytes("sess-1"),
            2200,
            "adopting twice adds nothing"
        );
        // Kept for the full retention window, not the 7-day unsent one.
        let then = SystemTime::now() - Duration::from_secs(8 * 24 * 3600);
        for p in walk_files(&s.root().join("sessions")).unwrap() {
            set_mtime(&p, then).unwrap();
        }
        assert_eq!(s.sweep(Duration::from_secs(30 * 24 * 3600)), 0);
        assert_eq!(s.session_inline_bytes("sess-1"), 2200);
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
