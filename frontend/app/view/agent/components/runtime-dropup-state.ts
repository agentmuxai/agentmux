// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The dropup's state: its signals, the two DOM refs, what the pane is configured to run, and the labels and
 * the change that follow from it. Nothing here makes an effect; the hook groups that do take this object.
 */

import { createSignal } from "solid-js";
import { getRuntimeConfig } from "../buildRuntimeArgs";
import { getProvider, type ProviderModel } from "../providers";
import type { RuntimeAgreement } from "../process-runtime";
import { patchRuntime } from "../runtime-apply";
import { PROVIDER_FLAGS_META_KEY } from "../launch-args";
import { codexEffort, codexModeFor, effectiveRuntime, effortApplies } from "../runtime-capabilities";
import type { AgentRuntimeConfig, PermissionMode } from "../../../agent-model/types";
import { createRoutePanel, routeModelOf } from "./runtime-dropup-routes";
import {
    EFFORT_OPTIONS,
    FALLBACK_MODEL_OPTIONS,
    MODE_OPTIONS,
    type AgentRuntimeDropupProps,
} from "./runtime-dropup-options";

/** The dropup's signals. */
function createDropupSignals() {
    const [open, setOpen] = createSignal(false);
    const [selectedOptIndex, setSelectedOptIndex] = createSignal(0);
    // Why the last change did not apply, or null. The menu shows what was
    // REQUESTED (`agent:runtime`); when applying it failed — the process may be
    // running something else — it says so instead of looking fine.
    const [applyError, setApplyError] = createSignal<string | null>(null);
    const [shownAgreement, setShownAgreement] = createSignal<RuntimeAgreement>({ kind: "unknown" });
    // The trigger's width while the panel is open: what it was when it opened.
    // Its label changes with every pick, and the composer strip lays its slots
    // out by measured width, so a trigger that resized could move, or move to
    // another row, and take the panel (anchored to it) along.
    const [heldWidth, setHeldWidth] = createSignal<number | null>(null);
    return {
        open,
        setOpen,
        selectedOptIndex,
        setSelectedOptIndex,
        applyError,
        setApplyError,
        shownAgreement,
        setShownAgreement,
        heldWidth,
        setHeldWidth,
    };
}

export function createDropupState(props: AgentRuntimeDropupProps) {
    const sig = createDropupSignals();
    const runtime = (): AgentRuntimeConfig => getRuntimeConfig(props.blockAtom()?.meta);

    // Goes through `patchRuntime`: changes to a pane are applied one at a time,
    // each built on the last, so two quick selections (the panel stays open for
    // exactly that) cannot undo each other.
    const updateRuntime = async (patch: Partial<AgentRuntimeConfig>) => {
        try {
            await patchRuntime(props.blockId, getProvider(props.providerId), patch, () => props.blockAtom()?.meta);
            sig.setApplyError(null);
        } catch (err: any) {
            sig.setApplyError(err?.message ?? String(err));
        }
    };

    // A model id the catalog has replaced is corrected quietly: opening a menu
    // must not restart the agent, nor count as the user's pick. The process
    // keeps running until its next spawn; the drift notice covers the gap.
    const migrateModel = async (model: string) => {
        try {
            await patchRuntime(props.blockId, getProvider(props.providerId), { model }, () => props.blockAtom()?.meta, {
                migration: true,
            });
        } catch (err) {
            console.warn("could not migrate the stored model id", err);
        }
    };

    // Codex's model list comes from its account's cache, which a fresh sign-in
    // only gets on its first turn: fetch it again (throttled) as a Codex
    // process starts, and when this menu opens.
    const isCodex = () => getProvider(props.providerId)?.id === "codex";
    const modelOptions = (): ProviderModel[] => getProvider(props.providerId)?.models ?? FALLBACK_MODEL_OPTIONS;

    // "" is no model named: the CLI picks (Codex before its account's list is known).
    const modelLabel = (value: string): string =>
        modelOptions().find((o) => o.value === value)?.label ?? (value === "" ? "CLI default" : value);
    const effortLabel = (value: string): string => EFFORT_OPTIONS.find((o) => o.value === value)?.label ?? value;
    const modeLabel = (value: string): string =>
        isCodex()
            ? codexModeFor(value as PermissionMode).label
            : (MODE_OPTIONS.find((o) => o.value === value)?.label ?? value);

    // What the process actually runs. An agent definition's own `--model` /
    // `--effort` (appended after the runtime's) win over the stored selection, so
    // the menu shows THAT — it must never claim a selection the definition
    // overrides. Picking something here removes the overriding flag from this
    // pane (`patchRuntime`), so the pick then holds.
    const shown = (): AgentRuntimeConfig =>
        effectiveRuntime(runtime(), props.blockAtom()?.meta?.[PROVIDER_FLAGS_META_KEY]);
    const runningModel = (): string => shown().model;
    // The effort the agent runs at. Codex: the selection when its model takes
    // it, otherwise that model's default (codexEffort).
    const runningEffort = (): string =>
        isCodex() ? (codexEffort(runningModel(), shown().effort).effort ?? shown().effort) : shown().effort;

    // The model that really answers when the agent is on a model route (srv writes it on the pane at each launch).
    const routeModel = () => routeModelOf(props.blockAtom()?.meta);
    const routes = createRoutePanel(props);

    const compactSummary = (): string => {
        const r = shown();
        // Effort is left out when it does nothing for this pane: the label says
        // what the agent runs, and "Haiku · high" would claim an effort it never gets.
        const parts = [modeLabel(r.permissionMode), routeModel()?.label ?? modelLabel(r.model)];
        if (effortApplies(props.providerId, runningModel())) parts.push(effortLabel(runningEffort()));
        return parts.join(" · ");
    };

    return {
        props,
        ...sig,
        runtime,
        updateRuntime,
        migrateModel,
        isCodex,
        modelOptions,
        routeModel,
        routes,
        modelLabel,
        effortLabel,
        modeLabel,
        shown,
        runningModel,
        runningEffort,
        compactSummary,
        /** The trigger button and the floating panel, set by the view. */
        referenceEl: undefined as HTMLButtonElement | undefined,
        floatingEl: undefined as HTMLDivElement | undefined,
    };
}

export type DropupState = ReturnType<typeof createDropupState>;
