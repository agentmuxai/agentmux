// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Fleet control — select, broadcast, and bulk-act on many agents at once.
// See docs/specs/SPEC_MULTI_AGENT_FLEET_CONTROL_2026_08_20.md and
// crates/srv/src/server/app_api/fleet.rs (the RPC handlers this binds to).

// Every shape below is GENERATED from its Rust definition by ts-rs and
// re-exported here under the name the frontend already used, so the public
// import surface (`@/app/store/rpc-api`) is unchanged. They were previously
// four hand-written `export interface` declarations plus five inline anonymous
// request shapes. See docs/specs/SPEC_RPC_BINDINGS_CODEGEN_2026_09_07.md §3.4
// step 2.
//
// `FleetStagePlan` is an alias: the Rust type is `StagePlanInput` (it is an
// input-only shape), but the frontend has always called it `FleetStagePlan`
// and `swarm-model.ts` imports that name. Renaming the Rust type to match
// would churn the server for a frontend spelling, so the alias lives here.
export type { FleetActionFailure } from "@/types/rpc/FleetActionFailure";
export type { FleetActionResult } from "@/types/rpc/FleetActionResult";
export type { FleetGroup } from "@/types/rpc/FleetGroup";
export type { StagePlanInput as FleetStagePlan } from "@/types/rpc/StagePlanInput";
