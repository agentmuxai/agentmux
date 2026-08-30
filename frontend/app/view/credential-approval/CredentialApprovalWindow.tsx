// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * CredentialApprovalWindow — the entire content of the credential-
 * approval subwindow opened by `credential_broker::open_approval_window`
 * (host, agentmux-cef) via `open_subwindow(initial_view="credential-approval")`.
 *
 * Deliberately NOT a `<Modal>` / `<ModalLayer>` panel like
 * `BrowserAuthModalPanel` — this is the sole content of a genuinely
 * separate top-level CEF window/DOM. That's load-bearing, not cosmetic:
 * AgentMux's `UIQuery`/`UIClick` MCP tools resolve reachable elements via
 * `__amq_allowed_for()` (agentmux-cef's browser_api/scripts/query.js),
 * which treats anything outside a `[data-blockid]` pane subtree as
 * "shared chrome" reachable by ANY agent. A `<Modal>` rendered in the
 * main window would put the Approve button in that shared-chrome bucket
 * — clickable by an agent other than the one whose credential is being
 * approved, not just the intended human. This component must never
 * render inside the normal pane/block tree and must never stamp a
 * `data-blockid` attribute anywhere in its own DOM.
 *
 * See docs/status/majestic-painting-minsky plan, "Why this shape."
 */

import { createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { invokeCommand } from "@/app/platform/ipc";
import { Button } from "@/element/button";

import "./credential-approval-window.scss";

type ApprovalMeta = {
    approvalId: string;
    origin: string;
    realm: string;
    isProxy: boolean;
    maskedUsername: string;
};

function parseMeta(): ApprovalMeta | null {
    const raw = new URLSearchParams(window.location.search).get("initialMeta");
    if (!raw) return null;
    try {
        const parsed = JSON.parse(raw);
        if (typeof parsed?.approval_id !== "string" || !parsed.approval_id) return null;
        return {
            approvalId: parsed.approval_id,
            origin: typeof parsed.origin === "string" ? parsed.origin : "",
            realm: typeof parsed.realm === "string" ? parsed.realm : "",
            isProxy: Boolean(parsed.is_proxy),
            maskedUsername: typeof parsed.masked_username === "string" ? parsed.masked_username : "",
        };
    } catch {
        return null;
    }
}

export const CredentialApprovalWindow = (): JSX.Element => {
    const meta = parseMeta();
    const [busy, setBusy] = createSignal(false);
    const [decided, setDecided] = createSignal(false);

    const decide = (approve: boolean) => {
        if (!meta || busy() || decided()) return;
        setBusy(true);
        void invokeCommand("credential_approval_decide", {
            approval_id: meta.approvalId,
            approve,
        })
            .catch(() => { /* host-side failure already falls through to the normal prompt */ })
            .finally(() => {
                // The host closes this window itself once the decision
                // resolves (approve or deny) — see the
                // `credential_approval_decide` IPC handler in agentmux-cef.
                // Disabling the buttons here just prevents a double-decide
                // in the brief window before that close arrives.
                setDecided(true);
                setBusy(false);
            });
    };

    onMount(() => {
        const onKeyDown = (e: KeyboardEvent) => {
            if (e.key === "Escape") {
                e.preventDefault();
                decide(false);
            } else if (e.key === "Enter") {
                e.preventDefault();
                decide(true);
            }
        };
        window.addEventListener("keydown", onKeyDown);
        onCleanup(() => window.removeEventListener("keydown", onKeyDown));
    });

    if (!meta) {
        // Malformed/missing initialMeta — shouldn't happen (the host only
        // ever opens this window with a well-formed payload), but a saved
        // human can't act on nothing. No approval_id means no way to
        // decide anything; the CEF AuthCallback(s) waiting on this window
        // will resolve via TTL fall-through instead.
        return (
            <div class="credential-approval-window credential-approval-window-error">
                <p>
                    This approval window is missing its request details and can't be used.
                    Close it and try signing in again.
                </p>
            </div>
        );
    }

    return (
        <div class="credential-approval-window">
            <header class="credential-approval-header">
                <h2>{meta.isProxy ? "Use saved proxy credential?" : "Use saved sign-in?"}</h2>
                <p class="credential-approval-origin">
                    <strong>{meta.origin || "This site"}</strong>
                </p>
                {meta.realm ? (
                    <p class="credential-approval-realm">
                        says: <em>{meta.realm}</em>
                    </p>
                ) : null}
            </header>
            <div class="credential-approval-body">
                <p>
                    AgentMux has a saved credential for <strong>{meta.maskedUsername || "this account"}</strong>.
                    Approve to sign in without exposing the password to the agent controlling this pane.
                </p>
            </div>
            <footer class="credential-approval-footer">
                <Button onClick={() => decide(false)} disabled={busy() || decided()}>
                    Deny
                </Button>
                <Button onClick={() => decide(true)} className="green solid" disabled={busy() || decided()}>
                    Approve
                </Button>
            </footer>
        </div>
    );
};

CredentialApprovalWindow.displayName = "CredentialApprovalWindow";
