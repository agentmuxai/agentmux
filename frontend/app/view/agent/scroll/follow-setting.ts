// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { untrack } from "solid-js";

import { getSettingsKeyAtom, isDev } from "@/app/store/global";

import { resolveFollowController } from "./follow-controller";

/**
 * `agent:followcontroller`, read once by whatever mounts (transcript, tool
 * preview, install log), so all of them use the same follow code.
 */
export function followControllerOn(): boolean {
    return resolveFollowController(
        untrack(() => getSettingsKeyAtom("agent:followcontroller")()),
        isDev
    );
}
