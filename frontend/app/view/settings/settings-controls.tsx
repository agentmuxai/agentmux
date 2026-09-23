// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, For, onCleanup, Show, type JSX } from "solid-js";

import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

// ── Helpers ───────────────────────────────────────────────────────────────────

export function set(key: string, value: unknown): void {
    void RpcApi.SetConfigCommand(TabRpcClient, { [key]: value } as any);
}

// ── SettingRow primitive ──────────────────────────────────────────────────────

export function SettingRow(p: { id?: string; label: string; description?: string; control: JSX.Element; indent?: boolean; stacked?: boolean }): JSX.Element {
    return (
        <div
            id={p.id ? `setting-${p.id}` : undefined}
            class="setting-row"
            classList={{ "setting-row--indent": p.indent, "setting-row--stacked": p.stacked }}
        >
            <div class="setting-row-label">
                <span class="setting-row-name">{p.label}</span>
                <Show when={p.description}>
                    <span class="setting-row-desc">{p.description}</span>
                </Show>
            </div>
            <div class="setting-row-control">{p.control}</div>
        </div>
    );
}

export function SectionHeader(p: { label: string }): JSX.Element {
    return <div class="settings-subheader">{p.label}</div>;
}

export function ToggleControl(p: { checked: boolean; onChange: (v: boolean) => void }): JSX.Element {
    return (
        <button
            type="button"
            role="switch"
            aria-checked={p.checked}
            class="setting-toggle"
            classList={{ "setting-toggle--on": p.checked }}
            onClick={() => p.onChange(!p.checked)}
        >
            <span class="setting-toggle-thumb" />
        </button>
    );
}

export function SliderControl(p: { min: number; max: number; step: number; value: number; onChange: (v: number) => void }): JSX.Element {
    const [local, setLocal] = createSignal(p.value);
    createEffect(() => setLocal(p.value));
    let timer: ReturnType<typeof setTimeout> | null = null;
    onCleanup(() => { if (timer != null) clearTimeout(timer); });
    return (
        <div class="setting-slider">
            <input
                type="range"
                min={p.min} max={p.max} step={p.step}
                value={local()}
                onInput={(e) => {
                    const v = parseFloat(e.currentTarget.value);
                    setLocal(v);
                    if (timer != null) clearTimeout(timer);
                    timer = setTimeout(() => { timer = null; p.onChange(v); }, 180);
                }}
            />
            <span class="setting-slider-val">{Math.round(local() * 100) / 100}</span>
        </div>
    );
}

/**
 * Debounced numeric settings input — every hand-rolled `<input
 * type="number" onBlur={...}>` in this directory committed only on blur,
 * so a setting never took effect until the user clicked away from the
 * field entirely (confirmed live: typing a new value and waiting, still
 * focused, left the setting unchanged; tabbing away committed it
 * instantly). Reported as "I need to select the terminal pane for the
 * update to take place" — the click on the terminal pane was just the
 * nearest thing to click, not a meaningful "select". See
 * SPEC_SETTINGS_LIVE_COMMIT_AND_TERMINAL_APPLY_GAPS_2026_09_22.md.
 *
 * Copies `SliderControl`'s own debounce shape verbatim (`local` signal
 * for the displayed value, debounced commit timer, cleared and restarted
 * on every keystroke) — that component already solved this correctly, it
 * just never got extended to a plain number field. `onBlur` additionally
 * flushes immediately rather than waiting out the debounce — the one case
 * `SliderControl` doesn't need an equivalent for, since a range input has
 * no "half-typed" state a user can tab away from mid-debounce.
 *
 * Uses a LONGER debounce than `SliderControl`'s 180ms — 400ms, not copied
 * verbatim. Confirmed live (not assumed) that 180ms is too short for
 * typing specifically: a drag gesture's every intermediate tick is a
 * legitimate value (that's the whole point of a slider), but a number
 * field's intermediate typed states are usually NOT — pausing mid-entry
 * (e.g. typing "7700" of an intended "77000" and hesitating before the
 * final digit) committed the incomplete value with 180ms, since "7700"
 * alone is already in-range for `term:scrollback`. 400ms sits inside the
 * normal range for "debounce after typing" UI conventions and comfortably
 * covers ordinary inter-keystroke gaps while still committing well within
 * half a second of the user stopping — "as reactive as possible with the
 * least additional user effort" without firing mid-word.
 *
 * `min`/`max`/`parse` intentionally mirror each existing site's own
 * validation exactly (`v >= min && (max == null || v <= max)`,
 * `parseInt` vs `parseFloat`) — an invalid or out-of-range value is
 * silently dropped, same as every current `onBlur` guard; this component
 * changes WHEN a value commits, not what is accepted.
 */
export function NumberControl(p: {
    min: number;
    max?: number;
    step: number;
    /** Must match what the setting itself stores — silently defaulting an
     *  integer setting to float parsing would let a fractional value
     *  through where the consumer expects a whole number (e.g.
     *  `term:scrollback`, in xterm.js display rows). Default "float". */
    parse?: "int" | "float";
    value: number;
    onChange: (v: number) => void;
    class?: string;
}): JSX.Element {
    const [local, setLocal] = createSignal(p.value);
    createEffect(() => setLocal(p.value));
    let timer: ReturnType<typeof setTimeout> | null = null;
    onCleanup(() => { if (timer != null) clearTimeout(timer); });
    const parseVal = (raw: string) => (p.parse === "int" ? parseInt(raw, 10) : parseFloat(raw));
    const inRange = (v: number) => !isNaN(v) && v >= p.min && (p.max == null || v <= p.max);
    return (
        <input
            class={p.class ?? "setting-number"}
            type="number" min={p.min} max={p.max} step={p.step}
            value={local()}
            onInput={(e) => {
                const v = parseVal(e.currentTarget.value);
                if (!isNaN(v)) setLocal(v); // mirror what's actually typed, valid or not
                if (timer != null) clearTimeout(timer);
                timer = setTimeout(() => { timer = null; if (inRange(v)) p.onChange(v); }, 400);
            }}
            onBlur={(e) => {
                // Only flush if a debounce is actually PENDING — if it
                // already fired (timer is null), the current value was
                // already committed and re-committing it here would fire
                // a second, redundant onChange for the same value.
                if (timer == null) return;
                clearTimeout(timer);
                timer = null;
                const v = parseVal(e.currentTarget.value);
                if (inRange(v)) p.onChange(v);
            }}
        />
    );
}

/**
 * Masked credential input — at-rest shows a fixed-width dot mask with a
 * "Replace" button (no partial/tail hint: this is a flat settings.json
 * string, not keychain-backed like the Armory identity form, so there's no
 * separate masked_tail metadata to show); clicking Replace reveals a
 * password-type entry field with Save/Cancel. Modeled on
 * `identity-account-form.tsx`'s masked-key UX without its keychain
 * lifecycle. See docs/specs/SPEC_SETTINGS_RECORDING_INPUT_SECTION_2026_08_19.md §2
 * (designed for `voice:groqApiKey`; also intended for messaging-bridge bot
 * tokens per that spec's Open Question 1 — keep this generic, no
 * voice-specific naming).
 */
export function MaskedKeyField(p: {
    value: string | undefined;
    onSave: (key: string) => void;
    placeholder?: string;
    disabled?: boolean;
}): JSX.Element {
    const [replacing, setReplacing] = createSignal(false);
    const [draft, setDraft] = createSignal("");
    const hasValue = () => !!p.value;

    const save = () => {
        const v = draft().trim();
        if (!v) return;
        p.onSave(v);
        setDraft("");
        setReplacing(false);
    };
    const cancel = () => {
        setDraft("");
        setReplacing(false);
    };

    return (
        <Show
            when={hasValue() && !replacing()}
            fallback={
                <div class="setting-masked-key setting-masked-key--entry">
                    <input
                        class="setting-text"
                        type="password"
                        autocomplete="off"
                        spellcheck={false}
                        value={draft()}
                        placeholder={p.placeholder}
                        disabled={p.disabled}
                        onInput={(e) => setDraft(e.currentTarget.value)}
                        onKeyDown={(e) => {
                            if (e.key === "Enter") save();
                            if (e.key === "Escape") cancel();
                        }}
                    />
                    <div class="setting-masked-key-actions">
                        <button
                            type="button"
                            class="setting-masked-key-btn setting-masked-key-btn--primary"
                            disabled={p.disabled || !draft().trim()}
                            onClick={save}
                        >
                            Save
                        </button>
                        <Show when={hasValue()}>
                            <button type="button" class="setting-masked-key-btn" onClick={cancel}>
                                Cancel
                            </button>
                        </Show>
                    </div>
                </div>
            }
        >
            <div class="setting-masked-key setting-masked-key--locked">
                <span class="setting-masked-key-dots">••••••••</span>
                <button
                    type="button"
                    class="setting-masked-key-btn"
                    disabled={p.disabled}
                    onClick={() => setReplacing(true)}
                >
                    Replace
                </button>
            </div>
        </Show>
    );
}

export function KeyValueEditor(p: { value: Record<string, string>; onChange: (v: Record<string, string>) => void }): JSX.Element {
    const keys = () => Object.keys(p.value ?? {});
    const [newKey, setNewKey] = createSignal("");
    const [newVal, setNewVal] = createSignal("");

    const updateEntry = (key: string, val: string) => p.onChange({ ...p.value, [key]: val });
    const removeEntry = (key: string) => {
        const next = { ...p.value };
        delete next[key];
        p.onChange(next);
    };
    const addEntry = () => {
        const k = newKey().trim();
        if (!k) return;
        p.onChange({ ...p.value, [k]: newVal() });
        setNewKey("");
        setNewVal("");
    };

    return (
        <div class="setting-kv-editor">
            <For each={keys()}>
                {(k) => (
                    <div class="setting-kv-row">
                        <input class="setting-text setting-kv-key" type="text" value={k} disabled />
                        <input
                            class="setting-text setting-kv-val"
                            type="text"
                            value={p.value[k] ?? ""}
                            onBlur={(e) => updateEntry(k, e.currentTarget.value)}
                        />
                        <button type="button" class="setting-kv-remove" aria-label={`Remove ${k}`} onClick={() => removeEntry(k)}>
                            <i class="fa-solid fa-xmark" />
                        </button>
                    </div>
                )}
            </For>
            <div class="setting-kv-row setting-kv-row--new">
                <input
                    class="setting-text setting-kv-key"
                    type="text"
                    placeholder="KEY"
                    value={newKey()}
                    onInput={(e) => setNewKey(e.currentTarget.value)}
                />
                <input
                    class="setting-text setting-kv-val"
                    type="text"
                    placeholder="value"
                    value={newVal()}
                    onInput={(e) => setNewVal(e.currentTarget.value)}
                    onKeyDown={(e) => { if (e.key === "Enter") addEntry(); }}
                />
                <button type="button" class="setting-kv-remove setting-kv-add" aria-label="Add environment variable" onClick={addEntry}>
                    <i class="fa-solid fa-plus" />
                </button>
            </div>
        </div>
    );
}
