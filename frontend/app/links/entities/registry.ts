// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Every kind of thing the recogniser knows (spec §4.2). A new kind is one file and one line here. */

import { addressKind } from "./kinds/address";
import { agentKind } from "./kinds/agent";
import { pullKind } from "./kinds/pull";
import { taskKind } from "./kinds/task";
import type { Destination, EntityKind, EntityKindId, OpenOptions, OpenResult } from "./types";

export const KINDS: readonly EntityKind[] = [addressKind, pullKind, taskKind, agentKind];

const BY_ID = new Map<EntityKindId, EntityKind>(KINDS.map((k) => [k.id, k]));

export function kindById(id: EntityKindId): EntityKind | undefined {
    return BY_ID.get(id);
}

const KIND_OF_DESTINATION: Record<Destination["kind"], EntityKindId> = {
    agent: "agent",
    task: "task",
    pull: "pull",
    url: "address",
};

/** A click on a link: the kind that made it opens its destination, through the framework's wrapper. */
export function openEntity(destination: Destination, options: OpenOptions = {}): Promise<OpenResult> {
    const kind = BY_ID.get(KIND_OF_DESTINATION[destination.kind]);
    return kind ? kind.open(destination, options) : Promise.resolve({ ok: false, reason: "unknown kind" });
}
