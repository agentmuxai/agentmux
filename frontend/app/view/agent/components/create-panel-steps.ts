// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Which of the create panel's steps apply, and which is current. */

import { createMemo } from "solid-js";

import { imageBlocksContainer } from "../defaults/container-default";
import type { Step } from "./create-panel-model";
import type { InstallStep } from "./create-panel-install";
import type { PanelState } from "./create-panel-state";
import type { ToolsCheck } from "./create-panel-tools";

export function useSteps(s: PanelState, tools: ToolsCheck, install: InstallStep) {
    const props = s.props;
    const { runtime, hostSteps } = s;
    const steps = createMemo<Step[]>(() => {
        const out: Step[] = [];
        if (hostSteps()) {
            if (tools.stillMissing().length > 0 || tools.installedPendingRestart().size > 0) out.push("tools");
            if (props.installed === false || install.session()?.state() === "running" || install.session()?.state() === "done") out.push("install");
        }
        out.push("create");
        return out;
    });
    const step = createMemo<Step>(() => {
        if (!hostSteps()) return "create";
        if (tools.missing() === null || runtime.deciding()) return "tools";
        if (tools.stillMissing().length > 0 && !tools.skipTools()) return "tools";
        if ((props.installed === false && !install.installDone()) || install.session()?.state() === "running") return "install";
        return "create";
    });
    // While a host step is asking for something, a container is the way past it, when it can run.
    const offerContainer = () =>
        hostSteps() &&
        step() !== "create" &&
        !runtime.deciding() &&
        runtime.canPickContainer() &&
        !imageBlocksContainer(runtime.imageAccess());

    return { steps, step, offerContainer };
}

export type StepFlow = ReturnType<typeof useSteps>;
