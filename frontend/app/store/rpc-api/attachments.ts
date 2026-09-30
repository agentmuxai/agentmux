// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Attachments in the agent composer: hand paths to the backend for
// processing, cancel a batch, look up processed attachments, copy one into
// a container pane's working folder. See
// docs/specs/SPEC_AGENT_PANE_IMAGE_ATTACHMENTS_2026_09_26.md §6 and
// crates/srv/src/server/app_api/attachments.rs. Progress arrives as
// `attachment:*` events scoped to the pane's block.

import { RpcClient } from "../rpc-client";
import type { AttachmentsCopyToWorkdirResult } from "@/types/rpc/AttachmentsCopyToWorkdirResult";
import type { AttachmentsInfoResult } from "@/types/rpc/AttachmentsInfoResult";
import type { AttachmentsIngestResult } from "@/types/rpc/AttachmentsIngestResult";
import type { CommandAttachmentsCancelData } from "@/types/rpc/CommandAttachmentsCancelData";
import type { CommandAttachmentsCopyToWorkdirData } from "@/types/rpc/CommandAttachmentsCopyToWorkdirData";
import type { CommandAttachmentsInfoData } from "@/types/rpc/CommandAttachmentsInfoData";
import type { CommandAttachmentsIngestData } from "@/types/rpc/CommandAttachmentsIngestData";

// Request/response types are generated from the Rust structs by ts-rs.
export const AttachmentsApi = {
    AttachmentsIngestCommand(
        client: RpcClient,
        data: CommandAttachmentsIngestData,
        opts?: RpcOpts,
    ): Promise<AttachmentsIngestResult> {
        return client.rpcCall("attachments.ingest", data, opts);
    },

    AttachmentsCancelCommand(client: RpcClient, data: CommandAttachmentsCancelData, opts?: RpcOpts): Promise<void> {
        return client.rpcCall("attachments.cancel", data, opts);
    },

    AttachmentsInfoCommand(
        client: RpcClient,
        data: CommandAttachmentsInfoData,
        opts?: RpcOpts,
    ): Promise<AttachmentsInfoResult> {
        return client.rpcCall("attachments.info", data, opts);
    },

    AttachmentsCopyToWorkdirCommand(
        client: RpcClient,
        data: CommandAttachmentsCopyToWorkdirData,
        opts?: RpcOpts,
    ): Promise<AttachmentsCopyToWorkdirResult> {
        return client.rpcCall("attachments.copy-to-workdir", data, opts);
    },
};
