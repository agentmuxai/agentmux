// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * MemoryAdoptionApprovalWindow — the entire content of the approval
 * subwindow the host opens for `memory_adoption_request`
 * (agentmux-cef `memory_adoption`), confirming that an agent adopts memory
 * from its earlier accounts (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md
 * §2.1.4).
 *
 * A separate top-level window, never a modal, for the same reason as
 * `CredentialApprovalWindow` (see its doc comment): anything in the main
 * window outside a pane is reachable by every agent's `UIQuery`/`UIClick`.
 * This component must never render inside the pane tree and must never
 * stamp a `data-blockid` anywhere in its DOM.
 */

import { createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { invokeCommand } from "@/app/platform/ipc";
import { Button } from "@/element/button";

import "./memory-adoption-approval-window.scss";

export type AdoptionSummaryFolder = { account: string; files: string[] };
export type AdoptionSummary = { agentName: string; folders: AdoptionSummaryFolder[] };

export function parseAdoptionMeta(raw: string | null): { approvalId: string; summary: AdoptionSummary } | null {
    if (!raw) return null;
    try {
        const parsed = JSON.parse(raw);
        if (typeof parsed?.approval_id !== "string" || !parsed.approval_id) return null;
        const s = parsed.summary ?? {};
        const folders: AdoptionSummaryFolder[] = Array.isArray(s.folders)
            ? s.folders.map((f: any) => ({
                  account: typeof f?.account === "string" ? f.account : "?",
                  files: Array.isArray(f?.files) ? f.files.filter((n: unknown) => typeof n === "string") : [],
              }))
            : [];
        return {
            approvalId: parsed.approval_id,
            summary: { agentName: typeof s.agentName === "string" ? s.agentName : "this agent", folders },
        };
    } catch {
        return null;
    }
}

/** How a folder is named to the human. */
export function accountLabel(account: string): string {
    if (account === "held") return "Files already in its folder that match another agent's memory";
    if (account === "default") return "The default (unlinked) account";
    return `Account ${account.slice(0, 8)}`;
}

export const MemoryAdoptionApprovalWindow = (): JSX.Element => {
    const meta = parseAdoptionMeta(new URLSearchParams(window.location.search).get("initialMeta"));
    const [busy, setBusy] = createSignal(false);

    const decide = (approve: boolean) => {
        if (!meta || busy()) return;
        setBusy(true);
        // The host adopts (on approve), reports to the Armory, and closes
        // this window itself.
        void invokeCommand("memory_adoption_decide", { approval_id: meta.approvalId, approve }).catch(() => {
            setBusy(false);
        });
    };

    onMount(() => {
        const onKeyDown = (e: KeyboardEvent) => {
            if (e.key === "Escape") {
                e.preventDefault();
                decide(false);
            }
        };
        window.addEventListener("keydown", onKeyDown);
        onCleanup(() => window.removeEventListener("keydown", onKeyDown));
    });

    if (!meta) {
        return (
            <div class="memory-adoption-approval-window memory-adoption-approval-window-error">
                <p>This window is missing its request and can't be used. Close it and try again from the Armory.</p>
            </div>
        );
    }

    return (
        <div class="memory-adoption-approval-window">
            <header class="memory-adoption-approval-header">
                <h2>Adopt earlier memory into {meta.summary.agentName}?</h2>
            </header>
            <div class="memory-adoption-approval-body">
                <p>
                    These files become part of {meta.summary.agentName}'s memory. Files it already has keep their
                    current content; older versions are kept in their history. They appear in its folder at its next
                    launch.
                </p>
                <For each={meta.summary.folders}>
                    {(folder) => (
                        <section class="memory-adoption-approval-folder">
                            <h3>{accountLabel(folder.account)}</h3>
                            <ul>
                                <For each={folder.files}>{(name) => <li>{name}</li>}</For>
                            </ul>
                        </section>
                    )}
                </For>
                <Show when={meta.summary.folders.length === 0}>
                    <p>No folders were chosen.</p>
                </Show>
            </div>
            <footer class="memory-adoption-approval-footer">
                <Button onClick={() => decide(false)} disabled={busy()}>
                    Cancel
                </Button>
                <Button onClick={() => decide(true)} className="green solid" disabled={busy()}>
                    {busy() ? "Adopting…" : "Adopt"}
                </Button>
            </footer>
        </div>
    );
};

MemoryAdoptionApprovalWindow.displayName = "MemoryAdoptionApprovalWindow";
