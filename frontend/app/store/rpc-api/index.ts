// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Hand-maintained RPC bindings. Keep in sync with the agentmux-srv RPC
// handlers (backend/rpc_types.rs, server/websocket.rs). The original Go
// generator (cmd/generate/main-generatets.go) was removed with the Go backend.
//
// This module was split from a single ~1,454-line rpc-api.ts into domain
// files. `RpcApi` is composed here from the per-domain partials; its public
// shape, method names, signatures, and call syntax (`RpcApi.SomeMethod(...)`)
// are identical to the original single-object export. None of the methods use
// `this`, so composing them into one plain object is behaviour-preserving.

import { AgentApi } from "./agent";
import { BlockApi } from "./block";
import { BundleImportApi } from "./bundle";
import { FileApi } from "./file";
import { IdentityApi } from "./identity";
import { McpApi } from "./mcp";
import { MemoryApi } from "./memory";
import { MiscApi } from "./misc";
import { SessionApi } from "./session";
import { SkillApi } from "./skill";
import { WorkspaceApi } from "./workspace";

export type { OAuthFlowStatus } from "./types";

// WshServerCommandToDeclMap
export const RpcApi = {
    ...MiscApi,
    ...BlockApi,
    ...FileApi,
    ...WorkspaceApi,
    ...AgentApi,
    ...IdentityApi,
    ...MemoryApi,
    ...SessionApi,
    ...McpApi,
    ...SkillApi,
    ...BundleImportApi,
};
