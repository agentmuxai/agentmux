// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { writeText as clipboardWriteText } from "@/util/clipboard";
import clsx from "clsx";
import { createEffect, createSignal, onCleanup, Show, type JSX } from "solid-js";
import { redactSecrets } from "./redact";
import "./CopyErrorButton.scss";

export type CopyErrorButtonVariant = "action" | "icon";
export type CopyErrorButtonTransport = "auto" | "dom";

export interface CopyErrorButtonProps {
    /** Lazy — a big stack is only formatted on click (§4.2). */
    report: () => string;
    /** `"action"`: a text button, for surfaces with an action row (Retry,
     * Restore, …). `"icon"`: shown on hover/focus, for inline rows. Default
     * `"action"`. */
    variant?: CopyErrorButtonVariant;
    /** `"dom"` skips straight to the `execCommand` fallback — for surfaces
     * whose host bridge is known to be down (e.g. the connection-lost
     * card). Default `"auto"` (IPC first, `#2535`'s verified path). */
    transport?: CopyErrorButtonTransport;
    /** Button text for `variant="action"` in its idle state — "Copy error",
     * or "Copy details" when the report includes a stack/stderr tail. The
     * caller decides which fits; this component never inspects `report()`
     * before the user clicks. Default "Copy error". */
    label?: string;
    className?: string;
}

async function writeViaDom(text: string): Promise<boolean> {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.setAttribute("readonly", "");
    ta.style.position = "fixed";
    ta.style.top = "-1000px";
    ta.style.left = "-1000px";
    document.body.appendChild(ta);
    ta.focus();
    ta.select();
    let ok = false;
    try {
        ok = document.execCommand("copy");
    } catch {
        ok = false;
    }
    document.body.removeChild(ta);
    return ok;
}

/**
 * The transport half of `<CopyErrorButton>`, exported standalone for
 * surfaces that render their own feedback UI instead of this component —
 * e.g. the agent failure row, whose action is a `PaneRowAction` data
 * descriptor with no room for `<CopyErrorButton>`'s own fallback textarea.
 * Redacts, then IPC first / DOM-`execCommand` fallback second, same as the
 * component. Returns whether the copy actually succeeded.
 */
export async function copyErrorReport(text: string, transport: CopyErrorButtonTransport = "auto"): Promise<boolean> {
    const redacted = redactSecrets(text);
    if (transport !== "dom") {
        try {
            await clipboardWriteText(redacted);
            return true;
        } catch {
            // fall through to the DOM transport
        }
    }
    return writeViaDom(redacted);
}

/**
 * The one copy control every error surface uses
 * (`SPEC_ERROR_COPY_EVERYWHERE_2026_09_24.md` §4.2). Transport order:
 * `writeText` (IPC, `#2535`'s verified path) first, then a hidden
 * `<textarea>` + `execCommand("copy")` if that throws — `BlockErrorBoundary`'s
 * approach, in reverse order. On total failure the report text stays
 * visible, selected, with a "Press Ctrl+C" hint, so the user is never left
 * with nothing.
 *
 * Redacts `report()`'s text itself (§4.5) before it ever reaches a
 * transport — every caller gets this for free instead of having to
 * remember it, including callers that pass a raw message rather than a
 * `formatErrorReport` result (which already redacts on its own; redacting
 * twice is a no-op, not a bug).
 */
export function CopyErrorButton(props: CopyErrorButtonProps): JSX.Element {
    const variant = () => props.variant ?? "action";
    const transport = () => props.transport ?? "auto";
    const label = () => props.label ?? "Copy error";

    const [state, setState] = createSignal<"idle" | "copied" | "failed">("idle");
    const [fallbackText, setFallbackText] = createSignal<string | null>(null);
    let resetTimer: ReturnType<typeof setTimeout> | undefined;
    let fallbackRef: HTMLTextAreaElement | undefined;

    onCleanup(() => clearTimeout(resetTimer));

    const scheduleReset = () => {
        clearTimeout(resetTimer);
        resetTimer = setTimeout(() => setState("idle"), 2000);
    };

    const handleClick = async () => {
        setFallbackText(null);
        clearTimeout(resetTimer);
        const text = redactSecrets(props.report());
        const ok = await copyErrorReport(text, transport());
        if (ok) {
            setState("copied");
            scheduleReset();
        } else {
            // Total failure: no auto-reset — the fallback text and the
            // Ctrl+C hint stay until the user has actually copied it.
            setState("failed");
            setFallbackText(text);
        }
    };

    createEffect(() => {
        if (fallbackText() !== null && fallbackRef) {
            fallbackRef.focus();
            fallbackRef.select();
        }
    });

    const buttonLabel = () => (state() === "copied" ? "Copied ✓" : state() === "failed" ? "Copy failed" : label());

    return (
        <span class={clsx("copy-error-button", `copy-error-button--${variant()}`, props.className)}>
            <Show
                when={variant() === "icon"}
                fallback={
                    <button
                        type="button"
                        class={clsx("copy-error-button-action", {
                            "is-copied": state() === "copied",
                            "is-failed": state() === "failed",
                        })}
                        onClick={handleClick}
                    >
                        {buttonLabel()}
                    </button>
                }
            >
                <button
                    type="button"
                    class={clsx("copy-error-button-icon", {
                        "is-copied": state() === "copied",
                        "is-failed": state() === "failed",
                    })}
                    onClick={handleClick}
                    title={buttonLabel()}
                    aria-label={buttonLabel()}
                >
                    <i
                        class={`fa-solid fa-${state() === "copied" ? "check" : state() === "failed" ? "triangle-exclamation" : "copy"}`}
                        aria-hidden="true"
                    />
                </button>
            </Show>
            <Show when={fallbackText() !== null}>
                <span class="copy-error-button-fallback">
                    <textarea
                        ref={fallbackRef}
                        readonly
                        value={fallbackText() ?? ""}
                        class="copy-error-button-fallback-text"
                    />
                    <span class="copy-error-button-fallback-hint">Press Ctrl+C</span>
                </span>
            </Show>
        </span>
    );
}
