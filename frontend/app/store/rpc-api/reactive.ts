// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Jekt/muxbus registration status for one agent — powers the Stash
// "Registration" tab (issue #2696). Backed by the `reactive.registrations`
// WS RPC command (agentmux-srv/src/server/reactive.rs's
// register_reactive_ws_handlers), not the pre-existing `/agentmux/reactive/*`
// HTTP routes (those exist for cross-instance/LAN server-to-server
// forwarding, not frontend consumption — this file is the first frontend
// caller of anything in that domain).

import { RpcClient } from "../rpc-client";

// The four shapes below are GENERATED from their Rust definitions by ts-rs and
// re-exported here under the names the frontend already uses, so the public
// import surface (`@/app/store/rpc-api`) is unchanged. They were previously
// four hand-written interfaces, one of which literally said "Mirrors
// agentmux-srv's AgentRegistration (backend/reactive/types.rs)" — exactly the
// kind of promise `scripts/check-rpc-bindings.sh` now enforces instead.
// See docs/specs/SPEC_RPC_BINDINGS_CODEGEN_2026_09_07.md §3.4 step 2.
export type { AgentRegistration as ReactiveAgentRegistration } from "@/types/rpc/AgentRegistration";
export type { RemoteRegistrationEntry as ReactiveRemoteRegistration } from "@/types/rpc/RemoteRegistrationEntry";
export type { MismatchAuditSummary as ReactiveMismatchSummary } from "@/types/rpc/MismatchAuditSummary";
export type { ReactiveRegistrationsResult } from "@/types/rpc/ReactiveRegistrationsResult";

import type { ReactiveRegistrationsParams } from "@/types/rpc/ReactiveRegistrationsParams";
import type { ReactiveRegistrationsResult as ReactiveRegistrationsResultT } from "@/types/rpc/ReactiveRegistrationsResult";

export const ReactiveApi = {
    GetReactiveRegistrationsCommand(
        client: RpcClient,
        data: ReactiveRegistrationsParams,
        opts?: RpcOpts,
    ): Promise<ReactiveRegistrationsResultT> {
        return client.rpcCall("reactive.registrations", data, opts);
    },
};
