// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The create panel's Tools step: the system tools the harness needs, asked once at setup and again on request. */

import { createMemo, createSignal } from "solid-js";

import type { MissingPrereq } from "./create-panel-model";
import type { PanelState } from "./create-panel-state";

export function useToolsCheck(s: PanelState) {
    const [missing, setMissing] = createSignal<MissingPrereq[] | null>(null);
    const [skipTools, setSkipTools] = createSignal(false);
    // Installed by the inline installer in this panel; srv's PATH is stale
    // until restart, so a re-check may still report them missing.
    const [installedPendingRestart, setInstalledPendingRestart] = createSignal<ReadonlySet<string>>(new Set());
    const checkTools = async (fresh: boolean) => setMissing(await s.props.probePrereqs(fresh));
    void checkTools(false);
    const stillMissing = createMemo(() => (missing() ?? []).filter((m) => !installedPendingRestart().has(m.tool)));

    return { missing, skipTools, setSkipTools, installedPendingRestart, setInstalledPendingRestart, checkTools, stillMissing };
}

export type ToolsCheck = ReturnType<typeof useToolsCheck>;
