// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The recorder under a line of Settings → Keyboard shortcuts (docs/specs/SPEC_KEYBOARD_SHORTCUTS_SETTINGS_2026_11_06.md
// §4.5, §5): the person presses the new shortcut, sees what it collides with, and saves it. The rules are in
// keyboard-recorder-state.ts; this is the field and its choices.

import { createMemo, createSignal, For, Show, type JSX } from "solid-js";

import { Button, Select, Switch, TextInput } from "@/app/element/ui";
import { captureKey } from "@/app/keybindings/capture";
import { formatKey, type KeyPlatform } from "@/app/keybindings/keys";
import type { CommandBindings } from "@/app/keybindings/registry";
import { useKeyCapture } from "./keyboard-capture-field";
import type { KeyboardEdit } from "./keyboard-section-parts";
import { committed, conflictText, MAX_STEPS, ownWhen, propose, recorderStatus, verdictOf, whereChoices, type Proposal, type Verdict } from "./keyboard-recorder-state";

/** Why a key cannot be saved, or the conflicts it has, under the field. */
function Findings(p: { proposal: Proposal; mine: string; platform: KeyPlatform }): JSX.Element {
    // Read through `p` each time: the proposal changes while this stays on screen (a switch, a second key).
    return (
        <ul class="kb-recorder-findings" role="list">
            <Show when={p.proposal.refusedPlain}>
                <li class="kb-recorder-finding--error">A plain key would fire while you type. Allow plain keys below to use it; it then works only outside text fields and terminals.</li>
            </Show>
            <Show when={p.proposal.blocked}>
                <li class="kb-recorder-finding--error">{p.proposal.blocked!.reason}. It cannot be used.</li>
            </Show>
            <Show when={p.proposal.warned}>
                <li class="kb-recorder-finding--warn">{p.proposal.warned!.reason}.</li>
            </Show>
            <For each={p.proposal.plan.conflicts}>
                {(c) => <li class={c.blocking ? "kb-recorder-finding--error" : "kb-recorder-finding--note"}>{conflictText(c, p.mine, p.platform)}</li>}
            </For>
        </ul>
    );
}

/** The switches and the place picker: what else decides how the key is saved. */
function Choices(p: {
    c: () => CommandBindings;
    proposal: Proposal | null;
    keepCurrent: boolean;
    setKeepCurrent: (v: boolean) => void;
    where: string;
    setWhere: (v: string) => void;
    allowPlain: boolean;
    setAllowPlain: (v: boolean) => void;
    confirmed: boolean;
    setConfirmed: (v: boolean) => void;
}): JSX.Element {
    return (
        <div class="kb-recorder-choices">
            <Show when={p.c().keys.length > 0 && !p.c().unbound}>
                <label class="kb-recorder-choice">
                    <Switch checked={p.keepCurrent} onChange={p.setKeepCurrent} ariaLabel="Keep the current keys too" />
                    <span>Keep the current keys too</span>
                </label>
            </Show>
            <label class="kb-recorder-choice">
                <span>Works</span>
                <Select
                    class="setting-select"
                    ariaLabel="Where this shortcut works"
                    value={p.where}
                    disabled={p.proposal?.plain && p.allowPlain}
                    onChange={p.setWhere}
                    options={whereChoices(p.c().command).map((w) => ({ value: w.value, label: w.label }))}
                />
            </label>
            <Show when={p.proposal?.plain}>
                <label class="kb-recorder-choice">
                    <Switch checked={p.allowPlain} onChange={p.setAllowPlain} ariaLabel="Allow plain keys" />
                    <span>Allow plain keys (only outside text fields and terminals)</span>
                </label>
            </Show>
            <Show when={p.proposal?.warned && !p.proposal?.blocked}>
                <label class="kb-recorder-choice">
                    <Switch checked={p.confirmed} onChange={p.setConfirmed} ariaLabel="Use it anyway" />
                    <span>Use it anyway</span>
                </label>
            </Show>
        </div>
    );
}

/** Save, or Replace when the key is taken, and Cancel. */
function Actions(p: { verdict: Verdict | null; onSave: () => void; onReplace: () => void; onClose: () => void }): JSX.Element {
    return (
        <div class="kb-recorder-actions">
            <Show when={p.verdict === "replace"} fallback={<Button tone="accent" disabled={p.verdict !== "save"} onClick={p.onSave}>Save</Button>}>
                <Button tone="accent" onClick={p.onReplace}>
                    Replace
                </Button>
            </Show>
            <Button onClick={p.onClose}>Cancel</Button>
        </div>
    );
}

/** The recorder: press the new shortcut (a second press makes it a two-step shortcut), then Save. */
export function KeyRecorder(p: { c: () => CommandBindings; edit: KeyboardEdit; onSave: (next: unknown[]) => void; onClose: () => void }): JSX.Element {
    const { c, edit } = p;
    const [steps, setSteps] = createSignal<string[]>([]);
    const [keepCurrent, setKeepCurrent] = createSignal(false);
    const [where, setWhere] = createSignal(ownWhen(c().command));
    const [allowPlain, setAllowPlain] = createSignal(false);
    const [confirmed, setConfirmed] = createSignal(false);
    const [altGr, setAltGr] = createSignal(false);
    let field: HTMLInputElement | undefined;

    const proposal = createMemo(() => propose(edit.entries(), c().command, steps(), { keepCurrent: keepCurrent(), where: where(), allowPlain: allowPlain() }, edit.platform));
    const verdict = createMemo(() => {
        const prop = proposal();
        return prop ? verdictOf(prop, confirmed()) : null;
    });
    const save = (replace: boolean) => {
        const prop = proposal();
        if (prop) p.onSave(committed(edit.entries(), c().command, prop, edit.platform, replace));
    };
    const record = (key: string) => {
        setConfirmed(false);
        setAltGr(false);
        setSteps(steps().length >= MAX_STEPS ? [key] : [...steps(), key]);
    };

    // Every press is the shortcut, not a command: the dispatcher is off while this is open, and the field takes the
    // press before anything else sees it. A bare Tab still moves on, so the keyboard is not trapped here.
    const onKeyDown = (e: KeyboardEvent) => {
        const bare = !e.ctrlKey && !e.altKey && !e.metaKey;
        if (e.key === "Tab" && bare) return;
        e.preventDefault();
        e.stopPropagation();
        const alone = bare && !e.shiftKey;
        if (alone && e.key === "Escape") return p.onClose();
        // Enter saves what is recorded; to record Enter itself, press its button.
        if (alone && e.key === "Enter") return verdict() === "save" ? save(false) : undefined;
        const got = captureKey(e);
        if ("key" in got) record(got.key);
        else if (got.ignored === "altgr") setAltGr(true);
    };
    useKeyCapture(() => field, onKeyDown);

    return (
        <div
            class="kb-recorder"
            role="group"
            aria-label={`Set the shortcut for ${c().label}`}
            onKeyDown={(e) => {
                // From a button or the picker (the field handles its own presses): Escape cancels.
                if (e.key !== "Escape") return;
                e.stopPropagation();
                p.onClose();
            }}
        >
            <p class="kb-recorder-status" role="status" aria-live="polite">
                {recorderStatus(c().label, proposal(), verdict(), edit.platform)}
            </p>
            <Show when={altGr()}>
                <p class="kb-recorder-finding--note">AltGr types a character on this keyboard, so it cannot be part of a shortcut. Try Ctrl or Alt on their own.</p>
            </Show>
            <div class="kb-recorder-field">
                <TextInput
                    detached
                    readOnly
                    ref={(el) => (field = el)}
                    class="kb-capture"
                    aria-label={`New shortcut for ${c().label}`}
                    placeholder="Press the new shortcut"
                    value={steps().length > 0 ? formatKey(steps().join(" "), edit.platform) : ""}
                />
                <Button onClick={() => record("Enter")}>Enter</Button>
                <Button onClick={() => setSteps([])} disabled={steps().length === 0}>
                    Clear
                </Button>
            </div>
            <Show when={proposal()}>
                <Findings proposal={proposal()!} mine={c().label} platform={edit.platform} />
            </Show>
            <Choices
                c={c}
                proposal={proposal()}
                keepCurrent={keepCurrent()}
                setKeepCurrent={setKeepCurrent}
                where={where()}
                setWhere={setWhere}
                allowPlain={allowPlain()}
                setAllowPlain={setAllowPlain}
                confirmed={confirmed()}
                setConfirmed={setConfirmed}
            />
            <Actions verdict={verdict()} onSave={() => save(false)} onReplace={() => save(true)} onClose={p.onClose} />
        </div>
    );
}
