// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Claude sign-in's page: its frame (a dialog, or the section it is in), then signed in, an error with Retry, or the login panel. */

import { Show, type JSX } from "solid-js";

import { InAppLoginPanel } from "@/app/view/agent/components/InAppLoginPanel";
import { Modal, type ModalScope } from "@/element/modal";
import { Button } from "@/app/element/ui";
import { CLAUDE_PROVIDER } from "./claude-login-model";
import type { LoginSession } from "./claude-login-session";
import type { LoginState } from "./claude-login-state";

/** The sign-in's frame: a dialog, or, `inline`, the section it is already in (Connectors' add section). */
function LoginShell(p: {
    inline?: boolean;
    scope?: ModalScope;
    busy: boolean;
    onClose: () => void;
    children: JSX.Element;
}): JSX.Element {
    if (p.inline) return <div class="accounts-chooser accounts-chooser--inline">{p.children}</div>;
    return (
        <Modal
            open={true}
            onClose={() => !p.busy && p.onClose()}
            scope={p.scope ?? "window"}
            closeOnBackdropClick={!p.busy}
            size="md"
            ariaLabel="Connect Anthropic"
            panelClass="accounts-chooser"
        >
            {p.children}
        </Modal>
    );
}

function SignedIn(props: { s: LoginState }): JSX.Element {
    return (
        <div class="accounts-chooser-modes">
            <div class="oauth-byo-note">✓ Signed in to Anthropic.</div>
            <div class="identity-key-actions">
                <Button tone="accent" class="identity-btn identity-btn-primary" onClick={() => props.s.props.onClose()}>
                    Done
                </Button>
            </div>
        </div>
    );
}

function LoginError(props: { s: LoginState; session: LoginSession }): JSX.Element {
    return (
        <div class="accounts-chooser-modes">
            <div class="oauth-byo-note">{props.s.error()}</div>
            <div class="identity-key-actions">
                <Button tone="accent" class="identity-btn identity-btn-primary" onClick={() => void props.session.start()}>
                    Retry
                </Button>
                <Button class="identity-btn identity-btn-secondary" onClick={() => props.s.props.onClose()}>
                    Close
                </Button>
            </div>
        </div>
    );
}

export function LoginView(props: { s: LoginState; session: LoginSession }): JSX.Element {
    const { s, session } = props;
    return (
        <LoginShell inline={s.props.inline} scope={s.props.scope} busy={s.inFlight()} onClose={s.props.onClose}>
            <Show when={!s.done()} fallback={<SignedIn s={s} />}>
                <Show when={!s.error()} fallback={<LoginError s={s} session={session} />}>
                    <InAppLoginPanel
                        providerId={CLAUDE_PROVIDER.id}
                        // "Anthropic" (the account/brand), not
                        // CLAUDE_PROVIDER.displayName ("Claude Code", the
                        // internal CLI tool AgentMux runs under the hood) —
                        // this panel is Connectors-facing, where the CLI
                        // implementation detail shouldn't surface. Contrast
                        // the agent pane's login (AgentDocumentView), a
                        // different call site of this same shared component,
                        // which correctly keeps showing the real
                        // provider.displayName since that surface is
                        // provider-generic across many CLIs.
                        providerLabel="Anthropic"
                        authUrl={s.authUrl()}
                        phase={s.phase()}
                        onCancel={session.onCancel}
                        onUseTerminal={session.useTerminal}
                    />
                </Show>
            </Show>
        </LoginShell>
    );
}
