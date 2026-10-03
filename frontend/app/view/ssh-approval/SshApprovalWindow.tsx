// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * SshApprovalWindow — the entire content of the approval subwindow the host
 * opens for srv's `POST /agentmux/approval/ask` (agentmux-cef `ssh_approval`):
 * the user's consent for an agent to use an SSH host, ssh's own prompt (a
 * password, a passphrase, a new host key, or a notice) for an agent's ssh or
 * for a durable pane's, or a confirmation such as ending a durable session,
 * per SPEC_REMOTE_TERMINALS_AND_DURABLE_SESSIONS_2026_10_02.md §5.3, §7.6
 * and §8.2.
 *
 * A separate top-level window, never a modal, for the same reason as
 * `CredentialApprovalWindow` (see its doc comment): anything in the main
 * window outside a pane is reachable by every agent's `UIQuery`/`UIClick`,
 * and this is exactly the answer an agent must not give itself. This
 * component must never render inside the pane tree and must never stamp a
 * `data-blockid` anywhere in its DOM. The message is shown as plain text.
 */

import { createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import { getApi } from "@/app/store/app-api";
import { Button } from "@/element/button";

import "./ssh-approval-window.scss";

export type SshApprovalKind = "consent" | "secret" | "yesno" | "info";

export type SshApprovalMeta = {
    approvalId: string;
    kind: SshApprovalKind;
    title: string;
    message: string;
    checkbox: string;
    okLabel: string;
    cancelLabel: string;
};

const KINDS: SshApprovalKind[] = ["consent", "secret", "yesno", "info"];

export function parseSshApprovalMeta(raw: string | null): SshApprovalMeta | null {
    if (!raw) return null;
    try {
        const p = JSON.parse(raw);
        if (typeof p?.approval_id !== "string" || !p.approval_id) return null;
        const str = (v: unknown, fallback: string) => (typeof v === "string" && v ? v : fallback);
        const kind: SshApprovalKind = KINDS.includes(p.kind) ? p.kind : "yesno";
        return {
            approvalId: p.approval_id,
            kind,
            title: str(p.title, "SSH"),
            message: str(p.message, ""),
            checkbox: typeof p.checkbox === "string" ? p.checkbox : "",
            okLabel: str(p.ok_label, kind === "consent" ? "Allow" : kind === "yesno" ? "Yes" : "OK"),
            cancelLabel: str(p.cancel_label, kind === "consent" ? "Deny" : kind === "yesno" ? "No" : "Cancel"),
        };
    } catch {
        return null;
    }
}

export const SshApprovalWindow = (): JSX.Element => {
    const meta = parseSshApprovalMeta(new URLSearchParams(window.location.search).get("initialMeta"));
    const [busy, setBusy] = createSignal(false);
    const [checked, setChecked] = createSignal(false);
    let input: HTMLInputElement | undefined;

    const decide = (approve: boolean) => {
        if (!meta || busy()) return;
        setBusy(true);
        const text = approve && meta.kind === "secret" ? (input?.value ?? "") : "";
        if (input) input.value = "";
        // The host hands the answer to srv's waiting request and closes this
        // window itself.
        void getApi()
            .approvals.decideSsh(meta.approvalId, approve, text, approve && checked())
            .catch(() => setBusy(false));
    };

    onMount(() => {
        input?.focus();
        // Escape declines anything. Enter is never "allow": this window takes
        // focus when it opens, so an Enter meant for a terminal must not grant
        // access or trust a host key. It submits only a typed secret (the
        // input's own handler); consent and yes/no answer by a click.
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
            <div class="ssh-approval-window ssh-approval-window-error">
                <p>This window is missing its request and can't be used. Close it; the SSH request then fails.</p>
            </div>
        );
    }

    return (
        <div class="ssh-approval-window">
            <header class="ssh-approval-header">
                <h2>{meta.title}</h2>
            </header>
            <div class="ssh-approval-body">
                <p class="ssh-approval-message">{meta.message}</p>
                <Show when={meta.kind === "secret" || meta.kind === "yesno"}>
                    <p class="ssh-approval-note">
                        AgentMux relays this prompt from ssh and cannot verify its text. Answer only if you expect ssh
                        on this host to be asking.
                    </p>
                </Show>
                <Show when={meta.kind === "secret"}>
                    <input
                        ref={input}
                        type="password"
                        class="ssh-approval-input"
                        autocomplete="off"
                        spellcheck={false}
                        maxLength={1024}
                        aria-label={meta.title}
                        onKeyDown={(e) => {
                            if (e.key === "Enter") {
                                e.preventDefault();
                                decide(true);
                            }
                        }}
                    />
                </Show>
                <Show when={meta.checkbox}>
                    <label class="ssh-approval-checkbox">
                        <input type="checkbox" checked={checked()} onChange={(e) => setChecked(e.currentTarget.checked)} />
                        <span>{meta.checkbox}</span>
                    </label>
                </Show>
            </div>
            <footer class="ssh-approval-footer">
                <Show when={meta.kind !== "info"}>
                    <Button onClick={() => decide(false)} disabled={busy()}>
                        {meta.cancelLabel}
                    </Button>
                </Show>
                <Button onClick={() => decide(true)} className="green solid" disabled={busy()}>
                    {meta.okLabel}
                </Button>
            </footer>
        </div>
    );
};

SshApprovalWindow.displayName = "SshApprovalWindow";
