// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The panel's rows and what the keyboard and the mouse do with them: build the list, move, apply, open. */

import { getProvider } from "../providers";
import { refreshCodexModels } from "../providers/codex-models-refresh";
import { dismissModelUndone } from "../model-undone";
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
    backRow,
    BACK_VALUE,
    disclosureRow,
    disclosureValue,
    freshRow,
    FRESH_VALUE,
    parseCompactValue,
    parseDisclosureValue,
    parseRouteValue,
    readFailureNote,
    routeRows,
} from "./runtime-dropup-routes";
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
    // On a route the harness reports its own alias; the route and model are what answers.
    const onRoute = s.routeModel();
    if (onRoute) return { kind: "note", section: "model", text: `Running: ${onRoute.label} on ${onRoute.route_label}` };
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

/** A row of the panel: drawn, and walked by the keys unless srv says it cannot be picked. */
const adder = (rows: Row[], options: OptionRow[]) => (row: OptionRow) => {
    rows.push({ kind: "option", ...row });
    if (!row.disabled) options.push(row);
};

/**
 * The Model section: a Back row (when srv has one) and the harness's own models. The model routes are not here: they
 * are the last rows of the panel (`addRoutesSection`), so the models and Effort stay one run.
 */
function addModelSection(s: DropupState, rows: Row[], options: OptionRow[], selected: string) {
    const panel = s.routes.panel();
    const onRoute = s.routeModel() !== null || s.routes.onSavedRoute();
    rows.push({ kind: "header", section: "model" });
    const add = adder(rows, options);
    const back = panel ? backRow(panel) : null;
    if (back) add(back);
    for (const o of s.modelOptions()) {
        add({ section: "model", value: o.value, label: o.label, description: o.description, current: !onRoute && selected === o.value });
    }
    if (!panel) {
        const why = s.routes.failure();
        if (why) rows.push({ kind: "note", section: "model", text: readFailureNote(why) });
    }
}

/**
 * The last rows of the panel, below Effort: one row per model route serving the harness, which opens the route's
 * models in place. A model srv says cannot be picked is greyed with its reason and kept out of keyboard navigation.
 */
function addRoutesSection(s: DropupState, rows: Row[], options: OptionRow[]) {
    const panel = s.routes.panel();
    if (!panel) return;
    const add = adder(rows, options);
    add(freshRow(s.routes.fresh()));
    for (const { route, rows: routeModels } of routeRows(panel, s.props.contextTokens)) {
        const expanded = s.routes.isExpanded(route.id);
        add(disclosureRow(route, expanded));
        if (expanded) routeModels.forEach(add);
    }
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
    addModelSection(s, rows, options, r.model);
    rows.push(runningModelRow(s, a));
    // Claude: the five rows whether or not effort applies (Haiku), so picking
    // Haiku doesn't shrink the panel; they are just not selectable then.
    // Codex: the running model's own levels, which differ per model.
    const notUsed = effortNotUsedReason(s.props.providerId, s.runningModel());
    const effortRows = s.isCodex()
        ? effortLevels("codex", s.runningModel()).map((v) => ({ value: v, label: v }))
        : EFFORT_OPTIONS;
    addSection("effort", effortRows, s.runningEffort(), false, notUsed === null ? undefined : `Not applied — ${notUsed}`);
    addRoutesSection(s, rows, options);
    return { rows, options };
}

/** Opening and closing a model route's models: from its row, or with the arrow keys. */
function createRouteOpening(s: DropupState, build: () => { options: OptionRow[] }) {
    // The keys stay on the route's row: its models come and go below it.
    const setRouteOpen = (route: string, expanded: boolean) => {
        s.routes.setExpanded(route, expanded);
        const at = build().options.findIndex((o) => o.value === disclosureValue(route));
        if (at >= 0) s.setSelectedOptIndex(at);
    };

    // Right opens the models of the route the keys are on (its row, or one of its models), Left closes them and
    // goes back to its row. True when the key was used here.
    const expandActive = (expand: boolean): boolean => {
        const choice = build().options[s.selectedOptIndex()];
        const route = choice && (parseDisclosureValue(choice.value) ?? parseRouteValue(choice.value)?.route ?? parseCompactValue(choice.value)?.route);
        if (!route || s.routes.isExpanded(route) === expand) return !!route;
        setRouteOpen(route, expand);
        return true;
    };

    return { setRouteOpen, expandActive };
}

/** Moving the active row, applying it, and opening the panel. */
export function createDropupActions(s: DropupState, a: DropupAgreement) {
    const build = () => buildRows(s, a);
    const { setRouteOpen, expandActive } = createRouteOpening(s, build);

    const move = (delta: number) => {
        const { options } = build();
        if (options.length === 0) return;
        s.setSelectedOptIndex((i) => (i + delta + options.length) % options.length);
    };

    // A route model or Back moves the agent through srv; a harness model off a route is the runtime setting it always was,
    // and from a route it first leaves the route (without a restart of its own: the runtime change restarts).
    const pickModel = async (value: string) => {
        const panel = s.routes.panel();
        // The choice to start a fresh session is for a route model or Back, not for leaving the route to a harness model.
        const move = async (route: string, model: string, restart: boolean, fresh = false) => {
            try {
                const { failed } = await s.routes.moveTo(route, model, restart, fresh);
                // Saved, but a pane that could not restart still runs the old model: say so, as the pick cannot be retried.
                s.setApplyError(failed.length > 0 ? `Saved, but the running pane did not switch: ${failed.join("; ")}` : null);
                return true;
            } catch (err: any) {
                s.setApplyError(err?.message ?? String(err));
                return false;
            }
        };
        const target = parseRouteValue(value);
        const compactTarget = parseCompactValue(value);
        let moved = false;
        const fresh = s.routes.fresh();
        // A fresh session does not read the conversation, so a model too small for it needs no compacting first.
        if (compactTarget && fresh) moved = await move(compactTarget.route, compactTarget.model, true, true);
        else if (compactTarget && s.props.compactThenSwitch) {
            try {
                await s.props.compactThenSwitch(compactTarget.route, compactTarget.model);
                s.setApplyError(null);
                await s.routes.refresh();
            } catch (err: any) {
                s.setApplyError(err?.message ?? String(err));
            }
        } else if (value === BACK_VALUE && panel?.previous) moved = await move(panel.previous.route, panel.previous.model, true, fresh);
        else if (target) moved = await move(target.route, target.model, true, fresh);
        // A harness model: the runtime setting it always was (which shows its own failure), from a route after leaving it.
        else if (!(s.routeModel() || s.routes.onSavedRoute()) || (await move("", "", false))) {
            await s.updateRuntime({ model: value });
            moved = true;
        }
        // The agent was moved on by hand: a card about the change srv undid no longer says where it is.
        if (moved && s.props.blockId) await dismissModelUndone(s.props.blockId);
        // The choice was for this change: a later pick in the same opening resumes the conversation unless it is turned on again.
        if (moved && fresh && (compactTarget || target || value === BACK_VALUE)) s.routes.resetFresh();
    };

    // Enter applies and keeps the panel open — deliberate departure from
    // FlyoutMenu's close-on-select. This panel hosts three independent axes,
    // not one value from one list; SPEC §9.2.
    const applySelection = async (idx: number) => {
        const { options } = build();
        const choice = options[idx];
        if (!choice) return;
        // A route's row opens or closes its models; it picks nothing, so no model changes.
        const route = parseDisclosureValue(choice.value);
        if (route) {
            setRouteOpen(route, !s.routes.isExpanded(route));
            return;
        }
        if (choice.value === FRESH_VALUE) {
            s.routes.setFresh(!s.routes.fresh());
            return;
        }
        if (choice.section === "mode") await s.updateRuntime({ permissionMode: choice.value as PermissionMode });
        else if (choice.section === "model") await pickModel(choice.value);
        else await s.updateRuntime({ effort: choice.value as EffortLevel });
    };

    const toggleOpen = () => {
        if (s.open()) {
            s.setOpen(false);
            return;
        }
        s.routes.collapseAll();
        s.routes.resetFresh();
        const { options } = build();
        const idx = options.findIndex((o) => o.section === "mode" && o.current);
        s.setSelectedOptIndex(idx >= 0 ? idx : 0);
        // The computed width, not the on-screen rect: it is in the trigger's own
        // (possibly zoomed) CSS pixels, with fractions, so setting it back changes nothing.
        const w = s.referenceEl ? parseFloat(getComputedStyle(s.referenceEl).width) : NaN;
        s.setHeldWidth(Number.isFinite(w) && w > 0 ? w : null);
        if (s.isCodex()) refreshCodexModels();
        void s.routes.refresh();
        s.setOpen(true);
    };

    return { build, move, applySelection, toggleOpen, expandActive };
}

export type DropupActions = ReturnType<typeof createDropupActions>;
