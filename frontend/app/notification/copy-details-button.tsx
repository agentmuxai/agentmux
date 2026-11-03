// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Copy details button on a toast (toast-debug.ts says what it copies), and the "Copied" a
// copy shows for a moment, whether it came from the button or from a click on the toast.

import { IconButton } from "@/app/element/ui";
import clsx from "clsx";
import { createSignal, onCleanup, type Accessor, type JSX } from "solid-js";

/** How long "Copied" stays up. */
const COPIED_MS = 1500;

/** `copy` runs `onCopy` and, once it has put the text on the clipboard, raises `copied` for a moment. */
export function useCopyFeedback(onCopy: () => void | Promise<void>): { copied: Accessor<boolean>; copy: () => void } {
    const [copied, setCopied] = createSignal(false);
    let timer: ReturnType<typeof setTimeout> | undefined;
    onCleanup(() => clearTimeout(timer));
    const copy = () => {
        void Promise.resolve(onCopy())
            .then(() => {
                setCopied(true);
                clearTimeout(timer);
                timer = setTimeout(() => setCopied(false), COPIED_MS);
            })
            .catch(() => {});
    };
    return { copied, copy };
}

export function CopyDetailsButton(props: { copied: boolean; onCopy: () => void }): JSX.Element {
    return (
        <span class={clsx("copy-btn", props.copied && "copied")}>
            <IconButton
                icon={props.copied ? "check" : "copy"}
                label="Copy details"
                tooltipPlacement="left"
                onClick={(e: MouseEvent) => {
                    e.stopPropagation();
                    props.onCopy();
                }}
            />
            {/* Outside the button, whose name would otherwise hide it from a screen reader. */}
            <span class="copy-btn-status" role="status">
                {props.copied ? "Copied" : ""}
            </span>
        </span>
    );
}
