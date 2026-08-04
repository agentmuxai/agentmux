// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Armory Bundle Format (ABF) import — Phase 3
// (agentmux-srv/src/server/app_api/bundle.rs). Window-scoped, no agent_id
// gate, same as the rest of `bundle.*`. See
// docs/specs/SPEC_ABF_IMPORT_UI_PHASE3_2026_08_02.md.

import { RpcClient } from "../rpc-client";

export const BundleImportApi = {
    BundleImportPreviewCommand(
        client: RpcClient,
        data: { file_path: string },
        opts?: RpcOpts,
    ): Promise<BundleImportPreviewResponse> {
        return client.rpcCall("bundle.import.preview", data, opts);
    },

    BundleImportCommitCommand(
        client: RpcClient,
        data: {
            file_path: string;
            expected_content_digest: string;
            bundle_name?: string;
            include_instructions: boolean;
            include_context_files: number[];
            include_skills: { source_dir: string; import_as?: string }[];
            include_mcp_servers: string[];
        },
        opts?: RpcOpts,
    ): Promise<BundleImportCommitResponse> {
        return client.rpcCall("bundle.import.commit", data, opts);
    },
};
