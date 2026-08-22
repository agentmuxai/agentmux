// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Settings -> Recording section: live existence check for the local
// whisper.cpp CLI/model file paths. See
// docs/specs/SPEC_SETTINGS_RECORDING_INPUT_SECTION_2026_08_19.md §3 and
// agentmux-srv/src/server/app_api/voice.rs.

import { RpcClient } from "../rpc-client";

export const VoiceApi = {
    CheckPathCommand(
        client: RpcClient,
        data: { path: string },
        opts?: RpcOpts,
    ): Promise<{ exists: boolean }> {
        return client.rpcCall("voice.checkPath", data, opts);
    },
};
