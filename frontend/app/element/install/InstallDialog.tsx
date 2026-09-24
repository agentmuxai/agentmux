// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * InstallDialog — `<InstallProgress>` in the standard modal chrome with the
 * §4.3 layout of SPEC_UNIVERSAL_INSTALL_DIALOG_2026_09_23.md: header and
 * footer stay in view, the body is the only scroll container, and elapsed
 * time sits at the left of the footer. Callers own the footer actions,
 * since what "Continue" means differs per install.
 *
 * Rendered inside a ModalLayer panel; the panel sizing that makes the
 * layout work lives in install-dialog.scss, keyed on the modal kind.
 */

import { Show, type JSX } from "solid-js";

import type { InstallSession } from "./install-session";
import { InstallProgress } from "./InstallProgress";
import "./install-dialog.scss";

interface InstallDialogProps {
    session: InstallSession;
    /** Install kind for the remembered Details choice, e.g. "provider-cli". */
    kind: string;
    icon: JSX.Element;
    title: string;
    /** Version pill, e.g. "v0.73.1" or "latest". */
    version?: string;
    description: string;
    /** Shown above the steps, e.g. an `<InstallConfirm>` before starting. */
    before?: JSX.Element;
    /** Footer buttons; the primary one should carry `data-modal-initial-focus`. */
    actions: JSX.Element;
}

export function formatElapsed(ms: number): string {
    const s = Math.floor(ms / 1000);
    return `${Math.floor(s / 60)}:${(s % 60).toString().padStart(2, "0")}`;
}

export const InstallDialog = (props: InstallDialogProps): JSX.Element => (
    <div class="install-dialog">
        <header class="modal-panel-header">
            <h2 class="modal-panel-title">
                <span class="install-dialog-icon" aria-hidden="true">
                    {props.icon}
                </span>
                {props.title}
                <Show when={props.version}>
                    <span class="install-dialog-version">{props.version}</span>
                </Show>
            </h2>
            <p class="modal-panel-description">{props.description}</p>
        </header>
        <div class="modal-panel-body install-dialog-body">
            {props.before}
            <InstallProgress session={props.session} kind={props.kind} />
        </div>
        <footer class="modal-panel-footer">
            <Show when={props.session.state() !== "idle"}>
                <span class="install-dialog-elapsed" aria-label="Elapsed time">
                    {formatElapsed(props.session.elapsedMs())}
                </span>
            </Show>
            {props.actions}
        </footer>
    </div>
);
