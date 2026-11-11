// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A remote's connection action, shared by its card menu (rowMenu in remotes-view.tsx) and its details.

import type { RemoteRecord } from "@/app/store/rpc-api/remotes";
import type { RemotesViewModel } from "./remotes-model";

/** A remote's connection action: Connect when it is down, Disconnect when it
 *  is up, and neither while a login is under way (an ssh at a password
 *  prompt): nothing is up to close yet, and Connect would start a second login. */
export function connectionAction(r: RemoteRecord): "Connect" | "Disconnect" | null {
    switch (r.status.state) {
        case "connected":
            return "Disconnect";
        case "connecting":
            return null;
        default:
            return "Connect";
    }
}

/** Run a remote's {@link connectionAction}. */
export function runConnectionAction(model: RemotesViewModel, r: RemoteRecord, action: "Connect" | "Disconnect"): Promise<void> {
    return action === "Connect" ? model.connect(r.name) : model.disconnect(r.name);
}
