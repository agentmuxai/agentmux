// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** What the process was actually spawned with, against what is selected: the report, the settle delay and the drift text. */

import { createEffect, createMemo, onCleanup } from "solid-js";
import { refreshCodexModels } from "../providers/codex-models-refresh";
import { getProvider } from "../providers";
import { compareRuntime, useProcessRuntime, type AxisDrift } from "../process-runtime";
import { PROVIDER_FLAGS_META_KEY } from "../launch-args";
import type { DropupState } from "./runtime-dropup-state";

// A difference is shown only once it has held for a moment. Right after any
// change the old process is still alive for a beat before it is replaced;
// that is the change landing, not a mismatch.
const DIFFERENCE_SETTLE_MS = 1500;

/** The agreement between the selection and the process, and the text that says so. Creates two effects and a memo, in this order. */
export function useRuntimeAgreement(s: DropupState) {
    const props = s.props;
    // What the process was actually spawned with, against what is selected. The
    // label above shows the selection; this says when the agent is NOT running it.
    const processRuntime = useProcessRuntime(props.blockId);
    createEffect(() => {
        processRuntime();
        if (s.isCodex()) refreshCodexModels();
    });
    const agreement = createMemo(() => {
        const meta = props.blockAtom()?.meta;
        return compareRuntime(
            getProvider(props.providerId),
            meta?.["agentMode"] as string | undefined,
            s.runtime(),
            meta?.[PROVIDER_FLAGS_META_KEY],
            processRuntime()
        );
    });
    createEffect(() => {
        const a = agreement();
        if (a.kind !== "differs") {
            s.setShownAgreement(a);
            return;
        }
        const t = setTimeout(() => s.setShownAgreement(a), DIFFERENCE_SETTLE_MS);
        onCleanup(() => clearTimeout(t));
    });
    const driftOf = (): AxisDrift[] => {
        const a = s.shownAgreement();
        return a.kind === "differs" || a.kind === "pending" ? a.drift : [];
    };
    const describeDrift = (d: AxisDrift): string => {
        const show = (v: string | undefined, axis: AxisDrift["axis"]) =>
            v === undefined
                ? "the CLI default"
                : axis === "model"
                  ? s.modelLabel(v)
                  : axis === "effort"
                    ? s.effortLabel(v)
                    : s.modeLabel(v);
        const name = d.axis === "permissionMode" ? "Mode" : d.axis === "model" ? "Model" : "Effort";
        return `${name}: selected ${show(d.wanted, d.axis)}, running ${show(d.running, d.axis)}`;
    };
    const driftSummary = (): string => driftOf().map(describeDrift).join("; ");
    // Re-applies the current selection: rebuilds the args and restarts the agent.
    const restartToApply = () => void s.updateRuntime({});

    const statusHeadline = (): string => {
        const kind = driftOf().length > 0 ? s.shownAgreement().kind : "agrees";
        if (kind === "pending") return "Applies after the current turn.";
        if (kind === "differs") return "The agent is not running what is selected.";
        return "Changes apply on the next turn.";
    };
    const differs = () => driftOf().length > 0 && s.shownAgreement().kind === "differs";

    return { processRuntime, agreement, driftOf, driftSummary, restartToApply, statusHeadline, differs };
}

export type DropupAgreement = ReturnType<typeof useRuntimeAgreement>;
