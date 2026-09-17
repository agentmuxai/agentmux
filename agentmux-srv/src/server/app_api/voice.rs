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

fn register_voice_check_path(engine: &Arc<WshRpcEngine>) {
    // Typed registration (SPEC_RPC_BINDINGS_CODEGEN_2026_09_07.md §3.1): the
    // request/response structs are named in `rpc_types` and carry
    // `#[derive(ts_rs::TS)]`, so `frontend/types/rpc/` gets them generated
    // and `scripts/check-rpc-bindings.sh` fails the build if the two sides
    // drift. Replaces a private `VoiceCheckPathReq` + an anonymous
    // `json!({"exists": ..})`, neither of which the generator could see.
    engine.register_typed(
        COMMAND_VOICE_CHECK_PATH,
        move |req: CommandVoiceCheckPathData, _ctx| async move {
            let trimmed = req.path.trim();
            let exists = !trimmed.is_empty() && std::path::Path::new(trimmed).exists();
            Ok(VoiceCheckPathResult { exists })
        },
    );
}
