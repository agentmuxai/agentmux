// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! This controller's side of the segment index
//! (`backend::continuity_segments`): each spawn is one segment of its
//! agent's conversation. Best-effort throughout: a failed write is logged and
//! never affects the process.

use super::*;
use crate::backend::continuity_segments as segs;

/// The segment a spawned process belongs to: `(agent_uid, segment_id)`.
pub(super) type SegmentRef = Option<(String, String)>;

/// Config-dir env vars, in the order providers are checked.
const CONFIG_DIR_VARS: &[&str] = &["CLAUDE_CONFIG_DIR", "CODEX_HOME", "GEMINI_CONFIG_DIR"];

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

/// Size of `output` in the global zone, read from the database rather than
/// this process's cache, so another channel's appends count.
fn zone_size(zone: &str) -> Option<i64> {
    let gfs = crate::backend::agent_session::global_transcript_store()?;
    gfs.line_state(zone, crate::backend::agent_session::OUTPUT_FILE).ok()?.map(|s| s.size)
}

impl PersistentSubprocessController {
    /// Records the start of the segment this spawn begins. `None` when the
    /// spawn carries no agent UID (quick-launch panes, a continuation that
    /// resumes before its row exists) or the write fails.
    pub(super) fn record_segment_start(
        &self,
        agent_uid: &str,
        config: &PersistentSpawnConfig,
        attempted_resume_sid: Option<&str>,
        carries_packet: bool,
    ) -> SegmentRef {
        let gfs = crate::backend::agent_session::global_transcript_store()?;
        let meta = self
            .mstore
            .as_deref()
            .and_then(|s| s.get::<crate::backend::obj::Block>(&self.block_id).ok().flatten())
            .map(|b| b.meta)
            .unwrap_or_default();
        let definition_id = Some(crate::backend::obj::meta_get_string(&meta, "agentId", "")).filter(|d| !d.is_empty());
        let zone = crate::backend::agent_session::agent_zone_for_block_meta(&meta);
        let start = segs::Start {
            segment_id: String::new(),
            agent_uid: agent_uid.to_string(),
            definition_id,
            provider: crate::backend::obj::meta_get_string(&meta, "agentProvider", ""),
            config_dir: CONFIG_DIR_VARS.iter().find_map(|v| config.env_vars.get(*v).cloned()),
            provider_session_id: attempted_resume_sid.map(str::to_string),
            cwd: config.working_dir.clone(),
            channel: crate::backend::reactive::registry::local_channel_id(),
            agentmux_version: crate::backend::base::get_version().to_string(),
            block_id: self.block_id.clone(),
            byte_start: zone.as_deref().and_then(zone_size),
            zone,
            started_at_ms: now_ms(),
            continuity_rung: segs::rung_for_spawn(attempted_resume_sid.is_some(), carries_packet),
            predecessor_segment_id: None,
        };
        let rung = start.continuity_rung;
        match segs::record_start(gfs, start) {
            Ok(id) => {
                tracing::info!(block_id = %self.block_id, segment_id = %id, rung = ?rung, "continuity: segment started");
                Some((agent_uid.to_string(), id))
            }
            Err(e) => {
                tracing::warn!(block_id = %self.block_id, error = %e, "continuity: segment start not recorded");
                None
            }
        }
    }
}

/// A fresh process reported its provider session id.
pub(super) fn record_segment_session(segment: &SegmentRef, provider_session_id: &str) {
    let (Some((uid, id)), Some(gfs)) = (segment, crate::backend::agent_session::global_transcript_store()) else {
        return;
    };
    if let Err(e) = segs::record_session(gfs, uid, id, provider_session_id, now_ms()) {
        tracing::warn!(segment_id = %id, error = %e, "continuity: segment session not recorded");
    }
}

/// The segment's process went away. `zone` is its global transcript zone.
pub(super) fn record_segment_end(segment: &SegmentRef, reason: segs::EndReason, zone: Option<&str>) {
    let (Some((uid, id)), Some(gfs)) = (segment, crate::backend::agent_session::global_transcript_store()) else {
        return;
    };
    if let Err(e) = segs::record_end(gfs, uid, id, reason, zone.and_then(zone_size), now_ms()) {
        tracing::warn!(segment_id = %id, error = %e, "continuity: segment end not recorded");
    } else {
        tracing::info!(segment_id = %id, reason = ?reason, "continuity: segment ended");
    }
}
