// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentRuntimeDropup — single trigger + floating panel consolidating the
 * Mode / Model / Effort drop-ups that used to be three separate FlyoutMenu
 * pills in AgentComposerStrip (SPEC_COMPOSER_STRIP_MODE_TOPLEVEL_2026_07_02
 * Fix 7). One button shows a live "Mode · Model · Effort" summary; the panel
 * floats upward from it, grouped into three labeled sections, and stays open
 * across selections so one visit can touch all three axes (deliberate
 * departure from FlyoutMenu's close-on-select — SPEC §9.2).
 *
 * Reuses the same positioning primitives FlyoutMenu itself uses
 * (@floating-ui/dom autoUpdate + computeMenuPosition + Portal +
 * data-pane-overlay) rather than FlyoutMenu directly: FlyoutMenu only renders
 * a flat MenuItem[] list and has no concept of grouped sections with headers.
 *
 * Model options stay registry-driven via getProvider(providerId)?.models
 * (live-overlaid from the providers.models RPC, same as the prior three-pill
 * implementation) so an API-sourced catalog surfaces new labels automatically.
 *
 * Also closes via an explicit button (top-right of the panel) in addition to
 * Esc/outside-click/re-clicking the trigger.
 *
 * Spec: docs/specs/SPEC_AGENT_RUNTIME_DROPUP_2026_07_09.md,
 * docs/specs/SPEC_AGENT_RUNTIME_DROPUP_CLOSE_BUTTON_2026_08_07.md.
 */

import { assertMenuInPaintableArea, computeMenuPosition } from "@/app/util/menu-position";
import { autoUpdate } from "@floating-ui/dom";
import { createEffect, createSignal, For, onCleanup, Show, type JSX } from "solid-js";
import { Portal } from "solid-js/web";
import { getRuntimeConfig } from "../buildRuntimeArgs";
import { familyKey, getProvider, type ProviderModel } from "../providers";
import { applyRuntimeChange } from "../runtime-apply";
import type { AgentRuntimeConfig, EffortLevel, PermissionMode } from "../types";

/** Serialize a MenuPositionResult.style the same way flyoutmenu.tsx does. */
function styleToString(s: JSX.CSSProperties): string {
    return `position:${s.position};left:${s.left};top:${s.top}`;
}

const PERMISSION_COLORS: Record<PermissionMode, string> = {
    bypass: "var(--error-color, #ef4444)",
    auto: "var(--accent-color, #3b82f6)",
    acceptEdits: "var(--warning-color, #eab308)",
    plan: "var(--success-color, #22c55e)",
    default: "var(--main-text-color)",
};

// Mode: trigger shows the short `label`; the panel shows `menuLabel` (the
// descriptive form) when present — matches the prior StripSelect convention.
const MODE_OPTIONS = [
    { value: "bypass", label: "Bypass", menuLabel: "Bypass (no prompts)" },
    { value: "auto", label: "Auto", menuLabel: "Auto (AI classifier)" },
    { value: "acceptEdits", label: "Accept Edits" },
    { value: "plan", label: "Plan", menuLabel: "Plan (read-only)" },
    { value: "default", label: "Default", menuLabel: "Default (prompt all)" },
] as const;

// Fallback when a provider defines no static model list — matches the prior
// StripSelect fallback exactly.
const FALLBACK_MODEL_OPTIONS: ProviderModel[] = [
    { value: "opus", label: "Opus" },
    { value: "sonnet", label: "Sonnet" },
    { value: "haiku", label: "Haiku" },
];

const EFFORT_OPTIONS = [
    { value: "low", label: "low" },
    { value: "medium", label: "medium" },
    { value: "high", label: "high" },
    { value: "xhigh", label: "xhigh" },
    { value: "max", label: "max" },
] as const;

type Section = "mode" | "model" | "effort";

interface OptionRow {
    section: Section;
    value: string;
    label: string;
    description?: string;
    current: boolean;
    color?: string;
}

type Row = { kind: "header"; section: Section } | (OptionRow & { kind: "option" });

interface AgentRuntimeDropupProps {
    blockId: string;
    blockAtom: () => Block | undefined;
    providerId: string;
}

export const AgentRuntimeDropup = (props: AgentRuntimeDropupProps): JSX.Element => {
    const [open, setOpen] = createSignal(false);
    const [selectedOptIndex, setSelectedOptIndex] = createSignal(0);
    const [floatingStyle, setFloatingStyle] = createSignal("position:fixed;left:0px;top:0px");

    let referenceEl: HTMLButtonElement | undefined;
    let floatingEl: HTMLDivElement | undefined;
    let cleanupAutoUpdate: (() => void) | null = null;

    const runtime = (): AgentRuntimeConfig => getRuntimeConfig(props.blockAtom()?.meta);

    const updateRuntime = async (patch: Partial<AgentRuntimeConfig>) => {
        try {
            await applyRuntimeChange(
                props.blockId,
                getProvider(props.providerId),
                { ...runtime(), ...patch },
                props.blockAtom()?.meta,
            );
        } catch {
            // Silent — settings retry on next change (matches the prior
            // AgentComposerStrip.updateRuntime tolerance).
        }
    };

    const modelOptions = (): ProviderModel[] => getProvider(props.providerId)?.models ?? FALLBACK_MODEL_OPTIONS;

    // Migrate a persisted model id the live-catalog overlay has superseded.
    //
    // `setProviderModels` refreshes CONCRETE (version-pinned) option values
    // when the authoritative catalog resolves, so an agent configured before
    // such a bump holds an id that is no longer present in `modelOptions()`.
    // Left alone, `modelLabel()` falls back to rendering the raw id and
    // `build()` marks no row `current` — the dropdown silently loses its
    // selection display for that agent.
    //
    // This migrates the PERSISTED value rather than making the lookups
    // family-tolerant. A display-only fix would show the new row as selected
    // while block meta still held the old id — reintroducing exactly the
    // advertise-one/select-another mismatch this PR exists to remove, one
    // layer further down. Alias values ("opus"/"sonnet") are never superseded,
    // so they never enter this path. reagent P1, PR #2990.
    createEffect(() => {
        const opts = modelOptions();
        const current = runtime().model;
        if (!current || opts.length === 0) return;
        if (opts.some((o) => o.value === current)) return;
        const replacement = opts.find((o) => familyKey(o.value) === familyKey(current));
        if (replacement) void updateRuntime({ model: replacement.value });
    });

    const modelLabel = (value: string): string => modelOptions().find((o) => o.value === value)?.label ?? value;
    const effortLabel = (value: string): string => EFFORT_OPTIONS.find((o) => o.value === value)?.label ?? value;
    const modeLabel = (value: string): string => MODE_OPTIONS.find((o) => o.value === value)?.label ?? value;

    const compactSummary = (): string => {
        const r = runtime();
        return [modeLabel(r.permissionMode), modelLabel(r.model), effortLabel(r.effort)].join(" · ");
    };

    // Single pass builds both the render list (rows, incl. section headers)
    // and the flat option list keyboard nav / selection walks.
    const build = (): { rows: Row[]; options: OptionRow[] } => {
        const r = runtime();
        const rows: Row[] = [];
        const options: OptionRow[] = [];

        const addSection = <T extends { value: string; label: string; menuLabel?: string; description?: string }>(
            section: Section,
            opts: readonly T[],
            currentValue: string,
            withColor: boolean
        ) => {
            rows.push({ kind: "header", section });
            for (const o of opts) {
                const row: OptionRow = {
                    section,
                    value: o.value,
                    label: o.menuLabel ?? o.label,
                    description: o.description,
                    current: currentValue === o.value,
                    color: withColor ? PERMISSION_COLORS[o.value as PermissionMode] : undefined,
                };
                rows.push({ kind: "option", ...row });
                options.push(row);
            }
        };

        addSection("mode", MODE_OPTIONS, r.permissionMode, true);
        addSection("model", modelOptions(), r.model, false);
        addSection("effort", EFFORT_OPTIONS, r.effort, false);
        return { rows, options };
    };

    const move = (delta: number) => {
        const { options } = build();
        if (options.length === 0) return;
        setSelectedOptIndex((i) => (i + delta + options.length) % options.length);
    };

    // Enter applies and keeps the panel open — deliberate departure from
    // FlyoutMenu's close-on-select. This panel hosts three independent axes,
    // not one value from one list; SPEC §9.2.
    const applySelection = async (idx: number) => {
        const { options } = build();
        const choice = options[idx];
        if (!choice) return;
        if (choice.section === "mode") await updateRuntime({ permissionMode: choice.value as PermissionMode });
        else if (choice.section === "model") await updateRuntime({ model: choice.value });
        else await updateRuntime({ effort: choice.value as EffortLevel });
    };

    // True only while DOM focus is actually inside the trigger or panel.
    // The panel deliberately stays open across selections (§9.2), so a user
    // can select a row and then Tab — or something else calls
    // textareaRef.focus() programmatically (e.g. AgentFooter's
    // acceptCompletion()) — without any mousedown outside the panel ever
    // firing. Without this guard, handleKeyDown would still be attached and
    // would swallow the next Enter/letter typed into the now-focused
    // composer textarea. reagentx-workflow P0 round 2 on this PR.
    const focusWithinDropup = (): boolean => {
        const active = document.activeElement;
        if (!active) return false;
        return !!(referenceEl?.contains(active) || floatingEl?.contains(active));
    };

    const handleKeyDown = (e: KeyboardEvent) => {
        if (!focusWithinDropup()) return;
        if (e.key === "ArrowDown") {
            e.preventDefault();
            move(1);
        } else if (e.key === "ArrowUp") {
            e.preventDefault();
            move(-1);
        } else if (e.key === "Enter") {
            e.preventDefault();
            void applySelection(selectedOptIndex());
        } else if (e.key === "Escape") {
            e.preventDefault();
            setOpen(false);
        } else if (!e.ctrlKey && !e.metaKey && !e.altKey && e.key.length === 1 && /[a-z0-9]/i.test(e.key)) {
            const k = e.key.toLowerCase();
            const idx = build().options.findIndex((o) => o.label.toLowerCase().startsWith(k));
            if (idx >= 0) {
                e.preventDefault();
                setSelectedOptIndex(idx);
            }
        }
    };

    const handleClickOutside = (e: MouseEvent) => {
        const target = e.target as Node;
        if (referenceEl?.contains(target) || floatingEl?.contains(target)) return;
        setOpen(false);
    };

    // Closes the panel as soon as focus moves outside it by ANY means (Tab,
    // a programmatic .focus() call elsewhere, etc.), not just a mousedown —
    // keeps the panel from staying invisibly "open" (and its listeners
    // attached) once the user has clearly moved on. Belt-and-suspenders with
    // the focusWithinDropup() guard in handleKeyDown above.
    const handleFocusChange = () => {
        if (!focusWithinDropup()) setOpen(false);
    };

    // All three listeners are scoped to the panel's open lifetime via this
    // effect (not onMount, which would span the trigger's entire mount
    // lifetime — reagentx-workflow P0 round 1 on this PR).
    createEffect(() => {
        if (!open()) return;
        document.addEventListener("mousedown", handleClickOutside);
        document.addEventListener("keydown", handleKeyDown, true);
        document.addEventListener("focusin", handleFocusChange);
        onCleanup(() => {
            document.removeEventListener("mousedown", handleClickOutside);
            document.removeEventListener("keydown", handleKeyDown, true);
            document.removeEventListener("focusin", handleFocusChange);
        });
    });
    onCleanup(() => cleanupAutoUpdate?.());

    // Positioning mirrors flyoutmenu.tsx's updatePosition/registerFloating
    // exactly (same primitive, same avoidNativePanes:false rationale — this
    // panel also carries data-pane-overlay so it should open in place at its
    // anchor, not get pushed toward the window edge by a native pane rect).
    const updatePosition = async () => {
        if (!referenceEl || !floatingEl) return;
        const pos = await computeMenuPosition(
            { anchor: referenceEl, placement: "top-start", avoidNativePanes: false },
            floatingEl
        );
        setFloatingStyle(styleToString(pos.style));
    };

    const registerFloating = (el: HTMLDivElement) => {
        floatingEl = el;
        requestAnimationFrame(() => {
            if (!(referenceEl instanceof Element) || !(floatingEl instanceof Element)) return;
            cleanupAutoUpdate?.();
            cleanupAutoUpdate = autoUpdate(referenceEl, floatingEl, updatePosition);
            assertMenuInPaintableArea(el, "agent-runtime-dropup");
        });
    };

    const toggleOpen = () => {
        if (open()) {
            setOpen(false);
            return;
        }
        const { options } = build();
        const idx = options.findIndex((o) => o.section === "mode" && o.current);
        setSelectedOptIndex(idx >= 0 ? idx : 0);
        setOpen(true);
    };

    return (
        <>
            <button
                type="button"
                ref={referenceEl}
                class="agent-runtime-dropup-trigger"
                style={{ "border-left": `3px solid ${PERMISSION_COLORS[runtime().permissionMode]}` }}
                title="Mode / Model / Effort — applies on the next turn"
                aria-haspopup="listbox"
                aria-expanded={open()}
                aria-label={`Runtime settings: ${compactSummary()}`}
                onClick={() => toggleOpen()}
            >
                <span class="agent-runtime-dropup-trigger-label">{compactSummary()}</span>
            </button>
            <Show when={open()}>
                <Portal>
                    <div
                        ref={registerFloating}
                        class="menu agent-runtime-dropup-panel"
                        style={floatingStyle()}
                        data-pane-overlay
                    >
                        {/* Sibling of the listbox below, not a child of it — a
                            role="listbox" should only contain role="option"
                            rows (plus an optional label); an interactive
                            button dropped inside it would be an invalid
                            listbox structure for assistive tech. Placed first
                            (matching Modal's showCloseButton convention) so
                            it's the first Tab stop after the trigger. */}
                        <button
                            type="button"
                            class="agent-runtime-dropup-close-btn"
                            aria-label="Close"
                            onClick={() => setOpen(false)}
                        >
                            {"✕"}
                        </button>
                        <div role="listbox" aria-label="Runtime settings">
                            <For each={build().rows}>
                                {(row) => {
                                    if (row.kind === "header") {
                                        return <div class="agent-runtime-dropup-section">{row.section}</div>;
                                    }
                                    const optIndex = () =>
                                        build().options.findIndex(
                                            (o) => o.section === row.section && o.value === row.value
                                        );
                                    return (
                                        <div
                                            class="menu-item agent-runtime-dropup-row"
                                            classList={{ active: optIndex() === selectedOptIndex() }}
                                            role="option"
                                            aria-selected={row.current}
                                            onMouseEnter={() => setSelectedOptIndex(optIndex())}
                                            // These rows are plain non-focusable divs — without this, a
                                            // mousedown here blurs the trigger and shifts
                                            // document.activeElement to <body> (outside the panel), which
                                            // handleFocusChange reads as "focus left" and closes on. That
                                            // made every selection close the panel despite §9.2's
                                            // stays-open decision already being implemented correctly —
                                            // this was a second, independent close path.
                                            onMouseDown={(e) => e.preventDefault()}
                                            onClick={() => {
                                                const idx = optIndex();
                                                setSelectedOptIndex(idx);
                                                void applySelection(idx);
                                            }}
                                        >
                                            <i
                                                class={`fa-solid fa-fw menu-item-icon menu-item-check${row.current ? " fa-check" : ""}`}
                                                style={row.color ? { color: row.color } : undefined}
                                            />
                                            <span class="label">{row.label}</span>
                                            <Show when={row.description}>
                                                <span class="agent-runtime-dropup-description">{row.description}</span>
                                            </Show>
                                        </div>
                                    );
                                }}
                            </For>
                        </div>
                    </div>
                </Portal>
            </Show>
        </>
    );
};

AgentRuntimeDropup.displayName = "AgentRuntimeDropup";
