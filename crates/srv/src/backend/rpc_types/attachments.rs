// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wire types for attachments in the agent composer
//! (docs/specs/SPEC_AGENT_PANE_IMAGE_ATTACHMENTS_2026_09_26.md §6,
//! SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md).

use serde::{Deserialize, Serialize};

/// `attachments.ingest` — hand the backend OS paths (from a drop or a native
/// clipboard read) to copy into the attachment store and process.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandAttachmentsIngestData {
    /// The agent pane asking. Progress events are scoped `block:<id>`.
    pub block_id: String,
    /// Client-chosen id for this batch; echoed in every event.
    pub batch_id: String,
    pub paths: Vec<String>,
    /// Only classify and apply the limits; copy and process nothing.
    /// Used by the drop overlay to describe a drag before it lands.
    #[serde(default)]
    pub dry_run: bool,
    /// Attachments already in the draft, so the limits cover the whole prompt.
    #[serde(default)]
    pub existing_count: u32,
    #[serde(default)]
    #[ts(type = "number")]
    pub existing_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct AttachmentsIngestResult {
    pub batch_id: String,
    /// Files accepted into the batch, in order. `index` keys every event.
    pub accepted: Vec<AttachmentPending>,
    /// Files refused before processing (limits, unreadable).
    pub rejected: Vec<AttachmentRejected>,
    /// Always empty since any file can be attached; kept so a frontend from
    /// the images-only release still reads the result.
    pub non_images: Vec<String>,
    /// Sum of `accepted[].bytes`.
    #[ts(type = "number")]
    pub total_bytes: u64,
    /// The limits applied, so the UI can show "N / max".
    pub max_files: u32,
    #[ts(type = "number")]
    pub max_total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct AttachmentPending {
    pub index: u32,
    pub path: String,
    pub name: String,
    #[ts(type = "number")]
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct AttachmentRejected {
    pub path: String,
    pub name: String,
    /// `too_many` | `too_large` | `not_found` | `unreadable`
    pub code: String,
    pub reason: String,
}

/// `attachments.cancel` — stop processing the rest of a batch.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandAttachmentsCancelData {
    pub batch_id: String,
}

/// `attachments.copy-to-workdir` — copy a stored attachment into the pane's
/// working folder (`cmd:cwd`). Container panes use this for pasted files:
/// their agents can't see the store.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandAttachmentsCopyToWorkdirData {
    pub block_id: String,
    pub id: String,
    /// The user's file name; de-conflicted in the folder.
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct AttachmentsCopyToWorkdirResult {
    /// Where the file landed.
    pub path: String,
}

/// `attachments.info` — look up processed attachments by id (transcript
/// replay, lightbox details).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct CommandAttachmentsInfoData {
    pub ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct AttachmentsInfoResult {
    /// One entry per requested id that is still in the store. Ids that were
    /// swept by retention are simply absent.
    pub items: Vec<AttachmentInfo>,
}

/// A processed attachment. `id` is the hex SHA-256 of the original bytes.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct AttachmentInfo {
    pub id: String,
    /// Display name. Not part of the stored content — the same bytes can
    /// arrive under different names — so it is empty from `attachments.info`.
    pub name: String,
    pub mime: String,
    #[ts(type = "number")]
    pub bytes: u64,
    pub width: u32,
    pub height: u32,
    /// What the agent receives (downscaled, metadata stripped).
    pub send_mime: String,
    #[ts(type = "number")]
    pub send_bytes: u64,
    pub send_width: u32,
    pub send_height: u32,
    /// Set when the original was an animated GIF (only the first frame is sent).
    #[serde(default)]
    pub first_frame_only: bool,
    /// What the file is: `image`, `svg`, `text`, `pdf`, `word`, `excel`,
    /// `powerpoint`, `archive`, `audio`, `video`, `image_file` (an image
    /// format that isn't decoded, e.g. HEIC) or `other`. The tile shows a
    /// thumbnail for image/svg/text and a type icon for the rest.
    /// SPEC_AGENT_PANE_FILE_ATTACHMENTS_2026_09_26.md §5.
    #[serde(default)]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub page_count: Option<u32>,
    /// Size of the extracted text version; 0 when there is none.
    #[serde(default)]
    #[ts(type = "number")]
    pub text_bytes: u64,
    /// Why a document has no text version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub text_note: Option<String>,
    /// An Office file carrying a VBA project.
    #[serde(default)]
    pub macros: bool,
}

/// An attachment as carried by a message: the id plus the name the user saw.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct AttachmentRef {
    pub id: String,
    pub name: String,
}

/// `attachment:progress` event payload.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct AttachmentProgressEvent {
    pub batch_id: String,
    pub index: u32,
    /// `copying` | `processing`
    pub stage: String,
    #[ts(type = "number")]
    pub done_bytes: u64,
    #[ts(type = "number")]
    pub total_bytes: u64,
}

/// `attachment:ready` event payload.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct AttachmentReadyEvent {
    pub batch_id: String,
    pub index: u32,
    pub info: AttachmentInfo,
}

/// `attachment:failed` event payload.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct AttachmentFailedEvent {
    pub batch_id: String,
    pub index: u32,
    /// `unsupported` | `heic` | `too_large_dimensions` | `decode` | `io` | `cancelled`
    pub code: String,
    pub error: String,
}

/// `attachment:batch-done` event payload — every item is ready, failed or cancelled.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct AttachmentBatchDoneEvent {
    pub batch_id: String,
}
