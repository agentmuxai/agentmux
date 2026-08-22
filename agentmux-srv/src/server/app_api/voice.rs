// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `voice.checkPath` — live existence check for the local whisper.cpp CLI
//! binary or GGML model file paths configured in Settings -> Recording.
//!
//! Exposes the exact same `Path::new(&p).exists()` check
//! `agentmux-srv/src/server/voice.rs` already performs inline at actual
//! transcription time (`voice.rs`'s `transcribe_local_whisper` /
//! `ensure_local_model`) -- no new validation logic, just surfacing it
//! proactively so the Settings UI can show live status instead of only
//! failing at first-recording-attempt. See
//! docs/specs/SPEC_SETTINGS_RECORDING_INPUT_SECTION_2026_08_19.md §3.

use super::*;

pub fn register(engine: &Arc<WshRpcEngine>, _state: &AppState) {
    register_voice_check_path(engine);
}

#[derive(serde::Deserialize)]
struct VoiceCheckPathReq {
    path: String,
}

fn register_voice_check_path(engine: &Arc<WshRpcEngine>) {
    engine.register_handler(
        COMMAND_VOICE_CHECK_PATH,
        Box::new(move |data, _ctx| {
            Box::pin(async move {
                let req: VoiceCheckPathReq = serde_json::from_value(data)
                    .map_err(|e| format!("voice.checkPath: {e}"))?;
                let trimmed = req.path.trim();
                let exists = !trimmed.is_empty() && std::path::Path::new(trimmed).exists();
                Ok(Some(json!({ "exists": exists })))
            })
        }),
    );
}
