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

export function SettingRow(p: { label: string; description?: string; control: JSX.Element; indent?: boolean; stacked?: boolean }): JSX.Element {
    return (
        <div class="setting-row" classList={{ "setting-row--indent": p.indent, "setting-row--stacked": p.stacked }}>
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
