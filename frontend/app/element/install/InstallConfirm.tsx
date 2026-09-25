// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * InstallConfirm — the pre-start consent state for an install that needs
 * one (SPEC_UNIVERSAL_INSTALL_DIALOG_2026_09_23.md §5.1): what will run,
 * and a warning when the OS will ask for a password or permission. The
 * caller supplies the action button as children.
 */

import { Show, type JSX } from "solid-js";

import "./install-confirm.scss";

interface InstallConfirmProps {
    commandPreview: string;
    needsElevation: boolean;
    /** Font Awesome brand icon name, e.g. "node-js". */
    brandIcon?: string;
    children?: JSX.Element;
}

export const InstallConfirm = (props: InstallConfirmProps): JSX.Element => (
    <div class="install-confirm">
        <p class="install-confirm-text">
            <Show when={props.brandIcon}>
                <i class={`install-confirm-brand-icon fa-brands fa-${props.brandIcon}`} aria-hidden="true" />
            </Show>
            This will run:
        </p>
        <code class="install-confirm-command">{props.commandPreview}</code>
        <Show when={props.needsElevation}>
            <p class="install-confirm-elevation">
                <i class="fa-solid fa-shield-halved" aria-hidden="true" /> This will ask for your password or show a
                system permission prompt.
            </p>
        </Show>
        {props.children}
    </div>
);
