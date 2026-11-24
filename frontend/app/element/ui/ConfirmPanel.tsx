// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { createSignal, createUniqueId, onMount, Show, type JSX } from "solid-js";

import { useMotion } from "@/app/motion/solid";
import { Button } from "./Button";
import "./confirm-panel.scss";

export interface ConfirmPanelProps {
    /** The question ("Delete me@example.com?"); it also names the panel for assistive technology. */
    title: string;
    /** What the confirming button says ("Delete account"). */
    confirmLabel: string;
    cancelLabel: string;
    /** The confirm is a deletion: the danger line tone. */
    destructive?: boolean;
    /** What the question tells the person: shown between the title and the buttons. */
    children?: JSX.Element;
    /** Runs when the person confirms; the buttons wait (the confirm shows busy) until it settles. */
    onConfirm: () => void | Promise<void>;
    /** Escape and Cancel. Neither works while a confirm is running. */
    onCancel: () => void;
    class?: string;
}

/**
 * A confirm that asks inside the element that asked (the row, card or pane the person just acted in), not over the
 * window: it takes its place in the flow, pushing what is below it down, and covers nothing. The question is the
 * panel's title, what the answer does is its body, and the buttons are Cancel and the confirm (the `ui-confirm-panels`
 * skill). Focus moves to the panel when it opens; whoever closes it puts focus back where the person was (CardGrid does,
 * for the card the panel hangs under). Escape inside it is Cancel. Nothing here deletes anything: `onConfirm` does.
 */
export function ConfirmPanel(props: ConfirmPanelProps): JSX.Element {
    const titleId = createUniqueId();
    const [pending, setPending] = createSignal(false);
    const motion = useMotion();
    let root: HTMLDivElement | undefined;
    onMount(() => {
        if (!root) return;
        motion.enter(root, "slideY");
        root.focus({ preventScroll: true });
    });
    const cancel = () => {
        if (!pending()) props.onCancel();
    };
    const confirm = async () => {
        if (pending()) return;
        setPending(true);
        try {
            await props.onConfirm();
        } finally {
            setPending(false);
        }
    };
    return (
        <div
            ref={root}
            class={clsx("ui-confirm-panel", props.destructive && "ui-confirm-panel--destructive", props.class)}
            role="group"
            aria-labelledby={titleId}
            tabIndex={-1}
            onKeyDown={(e) => {
                if (e.key !== "Escape") return;
                // Not the pane's Escape, nor the card's: this panel is the innermost thing that is open.
                e.stopPropagation();
                e.preventDefault();
                cancel();
            }}
        >
            <div class="ui-confirm-panel__title" id={titleId}>
                {props.title}
            </div>
            <Show when={props.children}>
                <div class="ui-confirm-panel__body">{props.children}</div>
            </Show>
            <div class="ui-confirm-panel__actions">
                <Button onClick={cancel} disabled={pending()}>
                    {props.cancelLabel}
                </Button>
                <Button tone={props.destructive ? "danger" : "accent"} busy={pending()} onClick={() => void confirm()}>
                    {props.confirmLabel}
                </Button>
            </div>
        </div>
    );
}
