// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Settings -> Recording section: live existence check for the local
// whisper.cpp CLI/model file paths. See
// docs/specs/SPEC_SETTINGS_RECORDING_INPUT_SECTION_2026_08_19.md §3 and
// agentmux-srv/src/server/app_api/voice.rs.

import { RpcClient } from "../rpc-client";
import type { CommandVoiceCheckPathData } from "@/types/rpc/CommandVoiceCheckPathData";
import type { VoiceCheckPathResult } from "@/types/rpc/VoiceCheckPathResult";

// Request/response types are GENERATED from the Rust structs by ts-rs and
// checked by scripts/check-rpc-bindings.sh — a srv-side shape change that is
// not reflected here now fails the build instead of drifting behind a "keep in
// sync" comment. First consumer of frontend/types/rpc/; see
// docs/specs/SPEC_RPC_BINDINGS_CODEGEN_2026_09_07.md §3.4 step 2.
export const VoiceApi = {
    CheckPathCommand(
        client: RpcClient,
        data: CommandVoiceCheckPathData,
        opts?: RpcOpts,
    ): Promise<VoiceCheckPathResult> {
        return client.rpcCall("voice.checkPath", data, opts);
    },
};
