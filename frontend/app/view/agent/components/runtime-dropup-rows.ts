// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The panel's rows and what the keyboard and the mouse do with them: build the list, move, apply, open. */

import { getProvider } from "../providers";
import { refreshCodexModels } from "../providers/codex-models-refresh";
import { runningModelNote } from "../resolved-model";
import { isPersistentLaunch } from "../launch-args";
import {
    codexModeFor,
    codexModes,
    effortApplies,
    effortLevels,
    effortNotUsedReason,
    permissionModeText,
} from "../runtime-capabilities";
import type { EffortLevel, PermissionMode } from "../../../agent-model/types";
import type { DropupAgreement } from "./runtime-dropup-agreement";
import {
    EFFORT_OPTIONS,
    MODE_OPTIONS,
    PERMISSION_TEXT_COLORS,
    type OptionRow,
    type Row,
    type Section,
} from "./runtime-dropup-options";
import type { DropupState } from "./runtime-dropup-state";

/** The mode section's options: Codex's own three sandbox modes, or Claude's five with what each does for this controller. */
function modeOptions(s: DropupState) {
    // What each mode does depends on the controller: a persistent agent's
    // permission prompts are answered for the user (see permissionModeText).
    const meta = s.props.blockAtom()?.meta;
    const provider = getProvider(s.props.providerId);
    const autoAnswers = !!provider && isPersistentLaunch(provider, meta?.["agentMode"] as string | undefined);
    // Codex: its own three sandbox modes; a Claude-only mode it holds shows as the one it runs.
    return s.isCodex()
        ? codexModes().map((m) => ({ value: m.value, label: m.label, menuLabel: m.menuLabel, description: m.note }))
        : MODE_OPTIONS.map((o) => {
              const t = permissionModeText(o.value, autoAnswers, s.props.providerId);
              return { ...o, menuLabel: t.label, description: t.note, detail: t.detail };
          });
}

/** The running-model line under the Model rows: what the CLI resolved the selection to, or a placeholder. */
function runningModelRow(s: DropupState, a: DropupAgreement): Row {
    // What the CLI resolved the selection to, on the last reply (the argv
    // only carries an alias).
    const last = runningModelNote({
        selectedModel: s.shown().model,
        selectedEffort: s.shown().effort,
        effortApplies: effortApplies(s.props.providerId, s.shown().model),
        effective: { model: a.processRuntime()?.effectiveModel, effort: a.processRuntime()?.effectiveEffort },
        lastReply: s.props.lastReplyModel,
        agreement: a.agreement(),
    });
    // Always there, so the panel doesn't grow when the first reply lands.
    return last
        ? { kind: "note", section: "model", text: last.differs ? `⚠ ${last.text}` : last.text }
        : { kind: "note", section: "model", text: "Running model: shown after the first reply", placeholder: true };
}

/** Single pass builds both the render list (rows, incl. section headers) and the flat option list keyboard nav / selection walks. */
export function buildRows(s: DropupState, a: DropupAgreement): { rows: Row[]; options: OptionRow[] } {
    const r = s.shown();
    const rows: Row[] = [];
    const options: OptionRow[] = [];

    const addSection = <T extends { value: string; label: string; menuLabel?: string; description?: string; detail?: string }>(
        section: Section,
        opts: readonly T[],
        currentValue: string,
        withColor: boolean,
        notApplied?: string
    ) => {
        rows.push({ kind: "header", section, note: notApplied });
        for (const o of opts) {
            const row: OptionRow = {
                section,
                value: o.value,
                label: o.menuLabel ?? o.label,
                description: o.description,
                detail: o.detail,
                current: currentValue === o.value,
                color: withColor ? PERMISSION_TEXT_COLORS[o.value as PermissionMode] : undefined,
                disabled: notApplied !== undefined,
            };
            rows.push({ kind: "option", ...row });
            // Keyboard navigation and selection walk only what can be picked.
            if (!row.disabled) options.push(row);
        }
    };

    addSection("mode", modeOptions(s), s.isCodex() ? codexModeFor(r.permissionMode).value : r.permissionMode, true);
    addSection("model", s.modelOptions(), r.model, false);
    rows.push(runningModelRow(s, a));
    // Claude: the five rows whether or not effort applies (Haiku), so picking
    // Haiku doesn't shrink the panel; they are just not selectable then.
    // Codex: the running model's own levels, which differ per model.
    const notUsed = effortNotUsedReason(s.props.providerId, s.runningModel());
    const effortRows = s.isCodex()
        ? effortLevels("codex", s.runningModel()).map((v) => ({ value: v, label: v }))
        : EFFORT_OPTIONS;
    addSection("effort", effortRows, s.runningEffort(), false, notUsed === null ? undefined : `Not applied — ${notUsed}`);
    return { rows, options };
}

/** Moving the active row, applying it, and opening the panel. */
export function createDropupActions(s: DropupState, a: DropupAgreement) {
    const build = () => buildRows(s, a);

    const move = (delta: number) => {
        const { options } = build();
        if (options.length === 0) return;
        s.setSelectedOptIndex((i) => (i + delta + options.length) % options.length);
    };

    // Enter applies and keeps the panel open — deliberate departure from
    // FlyoutMenu's close-on-select. This panel hosts three independent axes,
    // not one value from one list; SPEC §9.2.
    const applySelection = async (idx: number) => {
        const { options } = build();
        const choice = options[idx];
        if (!choice) return;
        if (choice.section === "mode") await s.updateRuntime({ permissionMode: choice.value as PermissionMode });
        else if (choice.section === "model") await s.updateRuntime({ model: choice.value });
        else await s.updateRuntime({ effort: choice.value as EffortLevel });
    };

    const toggleOpen = () => {
        if (s.open()) {
            s.setOpen(false);
            return;
        }
        const { options } = build();
        const idx = options.findIndex((o) => o.section === "mode" && o.current);
        s.setSelectedOptIndex(idx >= 0 ? idx : 0);
        // The computed width, not the on-screen rect: it is in the trigger's own
        // (possibly zoomed) CSS pixels, with fractions, so setting it back changes nothing.
        const w = s.referenceEl ? parseFloat(getComputedStyle(s.referenceEl).width) : NaN;
        s.setHeldWidth(Number.isFinite(w) && w > 0 ? w : null);
        if (s.isCodex()) refreshCodexModels();
        s.setOpen(true);
    };

    return { build, move, applySelection, toggleOpen };
}

export type DropupActions = ReturnType<typeof createDropupActions>;
