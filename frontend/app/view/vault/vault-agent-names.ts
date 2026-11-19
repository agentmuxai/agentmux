// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The names of this AgentMux's agents by their permanent id, for the places in the Vault pane that hold only the
// id: a grant given for a login that is shared with an agent is stored under the agent's id.

import { createResource } from "solid-js";

import type { AgentNames } from "./vault-grant-labels";
import { loadShares } from "./vault-store";

/** A lookup from an agent's id to its name, read with the share list of the entry `entryId` gives (srv names every
 *  agent there: the ones the entry is shared with and the ones it could be). Nothing is read while it gives
 *  `false`, and an id it does not know has no name, so the caller shows what was stored. */
export function createAgentNames(entryId: () => string | false): AgentNames {
    const [data] = createResource(entryId, (id) => loadShares(id).catch(() => undefined));
    return (agent) => {
        const list = data();
        return list?.shares?.find((s) => s.agent === agent)?.name ?? list?.agents?.find((a) => a.uid === agent)?.name;
    };
}
