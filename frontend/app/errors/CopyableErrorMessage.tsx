// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createSignal, type JSX } from "solid-js";
import { writeText as clipboardWriteText } from "@/util/clipboard";
import { redactSecrets } from "./redact";

/**
 * An inline error message with its own small copy button — for a message
 * that wraps + scrolls instead of blowing up the panel, and lets the user
 * hand the exact text to support/an issue without retyping it. `class`
 * names the container; `-text` / `-copy-btn` suffixes get their own rules
 * alongside it (see `_identity-panel.scss` / `_form-overlay.scss`).
 *
 * Moved here from `view/accounts/AgentMuxConnectPanel.tsx` and exported
 * (`SPEC_ERROR_COPY_EVERYWHERE_2026_09_24.md` §3, §5) so any surface can use
 * it, not just AgentMux Cloud connect. Kept as its own small implementation
 * rather than rebuilt on `<CopyErrorButton>`: its two existing call sites'
 * stylesheets target this exact button structure directly, and the only
 * change P1 asks for here is redaction, not a new DOM/CSS shape.
 */
export function CopyableErrorMessage(props: { message: string; class: string }): JSX.Element {
    const [copied, setCopied] = createSignal(false);
    let copiedTimer: ReturnType<typeof setTimeout> | null = null;

    const copy = () => {
        void clipboardWriteText(redactSecrets(props.message))
            .then(() => {
                if (copiedTimer) clearTimeout(copiedTimer);
                setCopied(true);
                copiedTimer = setTimeout(() => setCopied(false), 1500);
            })
            .catch(() => {});
    };

    return (
        <div class={props.class}>
            <span class={`${props.class}-text`}>{props.message}</span>
            <button
                type="button"
                class={`${props.class}-copy-btn`}
                onClick={copy}
                title={copied() ? "Copied!" : "Copy error message"}
                aria-label="Copy error message"
            >
                <i class={copied() ? "fa-solid fa-check" : "fa-solid fa-copy"} aria-hidden="true" />
            </button>
        </div>
    );
}
