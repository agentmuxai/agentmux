// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// An agent changing the person's shortcuts, as the page does it (MuxAPI `UI` actions `bind` and `reset`,
// docs/specs/SPEC_KEYBOARD_SHORTCUTS_SETTINGS_2026_11_06.md §10): the rules are keybindings/agent-edit.ts; this
// writes the `keybindings` setting and tells the person what changed, with a button that puts it back.

import { keyPlatform } from "@/app/keybindings";
import { planAgentReset, planAgentSet, type AgentEdit, type AgentResetRequest, type AgentSetRequest, type Taken } from "@/app/keybindings/agent-edit";
import { setNotificationAction } from "@/app/notification/usenotification";
import { pushNotification, settingsAtom } from "@/app/store/global";
import { openKeyboardSettings } from "@/app/store/keyboard-focus-request";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";

/** What the agent is told. */
export type AgentEditResult = { changed: true; summary: string; keys: string[]; note: string } | { changed: false; reason: string; taken?: Taken[] };

/** How long the person has to press Undo on the notice. */
const UNDO_MS = 60_000;

const entriesNow = (): unknown[] => ((settingsAtom() as Record<string, unknown> | undefined)?.["keybindings"] as unknown[] | undefined) ?? [];
const write = (entries: unknown[]): Promise<unknown> => RpcApi.SetConfigCommand(TabRpcClient, { keybindings: entries } as never);
const same = (a: unknown[], b: unknown[]): boolean => JSON.stringify(a) === JSON.stringify(b);
const changes = (n: number): string => (n === 1 ? "a shortcut" : `${n} shortcuts`);

function say(id: string, title: string, message: string): void {
    pushNotification({ id, icon: "fa-keyboard", title, message, timestamp: new Date().toISOString(), type: "info", expiration: Date.now() + 8000 });
}

/** Undo puts the old setting back, unless the person has changed their shortcuts since: theirs are not undone. */
async function undo(id: string, agent: string, count: number, before: unknown[], after: unknown[]): Promise<void> {
    if (!same(entriesNow(), after)) return say(`${id}:undone`, "Not undone", "Your shortcuts have changed since, so they were left as they are.");
    await write(before);
    say(`${id}:undone`, `Undone ${count === 1 ? "1 change" : `${count} changes`} by ${agent}`, "Your shortcuts are back as they were.");
}

/** Tells the person, with Undo and Show, that `agent` changed their shortcuts. */
function announce(agent: string, edit: Extract<AgentEdit, { changed: true }>, command: string | undefined, before: unknown[]): void {
    const id = `shortcuts-agent:${Date.now()}`;
    setNotificationAction(`${id}:undo`, () => void undo(id, agent, edit.count, before, edit.entries));
    setNotificationAction(`${id}:show`, () => void openKeyboardSettings(command));
    pushNotification({
        id,
        icon: "fa-keyboard",
        title: `${agent} changed ${changes(edit.count)}`,
        message: edit.summary,
        timestamp: new Date().toISOString(),
        type: "info",
        expiration: Date.now() + UNDO_MS,
        actions: [
            { label: "Undo", actionKey: `${id}:undo` },
            { label: "Show", actionKey: `${id}:show` },
        ],
    });
}

async function apply(edit: AgentEdit, agent: string, command: string | undefined): Promise<AgentEditResult> {
    if (edit.changed === false) return edit;
    const before = entriesNow();
    await write(edit.entries);
    announce(agent, edit, command, before);
    return { changed: true, summary: edit.summary, keys: edit.keys, note: "The person was told and can undo it from the notice." };
}

/** `bind`: give a command exactly these keys (none unbinds it). */
export function agentSetShortcut(req: AgentSetRequest, agent: string): Promise<AgentEditResult> {
    return apply(planAgentSet(entriesNow(), req, keyPlatform()), agent, req.command?.trim());
}

/** `reset`: a command, or every command, back on its defaults. */
export function agentResetShortcuts(req: AgentResetRequest, agent: string): Promise<AgentEditResult> {
    return apply(planAgentReset(entriesNow(), req, keyPlatform()), agent, req.command?.trim());
}
