// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * Drop file(s) onto an agent pane → copy them into the agent's CWD AND
 * splice `@filename` tokens into the composer textarea at the caret, so the
 * agent sees the new file in its next turn.
 *
 * Spec: docs/specs/SPEC_PANE_FILE_DROP_2026_05_30.md §3.1, §3.6.
 */

import { createSignal, onCleanup, onMount } from "solid-js";
import { detectHost } from "@/app/platform/ipc";
import { getSettingsKeyAtom, pushNotification, WOS } from "@/app/store/global";
import { baseName, consumeDragPaths, copyFilesToDir } from "@/util/dnd";

interface Opts {
    blockId: string;
    /** Returns the agent-view DOM root that should listen for drop events. */
    rootRef: () => HTMLElement | undefined;
}

interface UseAgentDropAttachResult {
    isDragOver: () => boolean;
    dropMessage: () => string;
}

/**
 * Find the composer textarea inside the agent pane and splice the given
 * tokens at the current caret. If the textarea isn't mounted (rare race
 * during pane init), returns false so the caller can decide to queue.
 */
function spliceComposerTokens(root: HTMLElement, tokens: string[]): boolean {
    const ta = root.querySelector<HTMLTextAreaElement>("textarea.agent-input");
    if (!ta) return false;
    const joined = tokens.join(" ");
    const caretAtStart = ta.selectionStart === 0;
    const before = ta.value.slice(0, ta.selectionStart);
    const after = ta.value.slice(ta.selectionEnd);
    // Pad with a leading space if the caret isn't at the very start AND the
    // preceding character isn't already whitespace, so "summarise" + "@x.csv"
    // doesn't read as "summarise@x.csv".
    const needsLead = !caretAtStart && before.length > 0 && !/\s$/.test(before);
    const insert = (needsLead ? " " : "") + joined + (after.startsWith(" ") ? "" : " ");
    const newVal = before + insert + after;
    const newCaret = before.length + insert.length;
    // Use the native setter so React/SolidJS reactive bindings observe the change.
    const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set;
    if (setter) {
        setter.call(ta, newVal);
    } else {
        ta.value = newVal;
    }
    ta.dispatchEvent(new Event("input", { bubbles: true }));
    ta.setSelectionRange(newCaret, newCaret);
    ta.focus();
    return true;
}

export function useAgentDropAttach(opts: Opts): UseAgentDropAttachResult {
    const [isDragOver, setIsDragOver] = createSignal(false);

    const enabledAtom = getSettingsKeyAtom("dnd:enabled");
    const insertTokenAtom = getSettingsKeyAtom("dnd:agentinserttoken");
    const concurrencyAtom = getSettingsKeyAtom("dnd:concurrency");

    const enabled = () => (enabledAtom() ?? true) !== false;
    const insertToken = () => (insertTokenAtom() ?? true) !== false;
    const concurrency = () => {
        const v = concurrencyAtom();
        return typeof v === "number" && v > 0 ? v : undefined;
    };

    const cwd = (): string | undefined => {
        const block = WOS.getObjectValue<Block>(WOS.makeORef("block", opts.blockId));
        return block?.meta?.["cmd:cwd"];
    };

    const dropMessage = () => {
        const c = cwd();
        return c ? `Copy to ${c}` : "No working directory detected";
    };

    onMount(() => {
        if (detectHost() !== "cef") return;
        const root = opts.rootRef();
        if (!root) return;

        const onDragOver = (e: DragEvent) => {
            if (!enabled()) return;
            // Only treat file drags as drop targets — text/url drops keep
            // the browser default (paste into composer).
            const types = e.dataTransfer?.types;
            if (!types || !Array.from(types).includes("Files")) return;
            e.preventDefault();
            setIsDragOver(true);
        };
        const onDragLeave = (e: DragEvent) => {
            // Only clear when leaving the root, not when crossing inner elements.
            if (e.target === root) setIsDragOver(false);
        };
        const onDrop = (e: DragEvent) => {
            if (!enabled()) return;
            const files = e.dataTransfer?.files;
            if (!files || files.length === 0) return;
            e.preventDefault();
            setIsDragOver(false);
            const targetCwd = cwd();
            if (!targetCwd) {
                pushNotification({
                    icon: "fa-triangle-exclamation",
                    title: "Drop failed",
                    message: "No working directory detected for this agent pane.",
                    timestamp: new Date().toISOString(),
                    type: "warning",
                    expiration: Date.now() + 8000,
                });
                return;
            }
            void (async () => {
                const paths = await consumeDragPaths();
                if (paths.length === 0) {
                    pushNotification({
                        icon: "fa-triangle-exclamation",
                        title: "Drop failed",
                        message: `Couldn't read the OS paths for ${files.length} dropped file(s). Try again.`,
                        timestamp: new Date().toISOString(),
                        type: "warning",
                        expiration: Date.now() + 6000,
                    });
                    return;
                }
                const outcome = await copyFilesToDir(paths, targetCwd, { concurrency: concurrency() });
                const successes = outcome.results.filter((r) => r.dest);
                const failures = outcome.results.filter((r) => r.error);

                if (successes.length > 0) {
                    // Track whether the composer actually received tokens. The
                    // toast wording differs: "Attached" implies the agent will
                    // see the file via the spliced @-token in its next turn;
                    // "Copied" is just "the bytes are on disk now — mention
                    // them yourself."
                    let tokensInserted = false;
                    if (insertToken()) {
                        const tokens = successes.map((r) => `@${baseName(r.dest!)}`);
                        tokensInserted = spliceComposerTokens(root, tokens);
                        if (!tokensInserted) {
                            // Composer not mounted (rare race during pane init).
                            // Surface a hint so the user knows the file is on
                            // disk and can mention it manually.
                            const names = successes.map((r) => baseName(r.dest!)).join(", ");
                            pushNotification({
                                icon: "fa-info-circle",
                                title: "Files copied",
                                message: `${names} → ${targetCwd}. Mention them in your next message.`,
                                timestamp: new Date().toISOString(),
                                type: "info",
                                expiration: Date.now() + 6000,
                            });
                            return;
                        }
                    }
                    const verb = tokensInserted ? "Attached" : "Copied";
                    const title =
                        successes.length === 1
                            ? `${verb} ${baseName(successes[0].dest!)}`
                            : `${verb} ${successes.length} files`;
                    pushNotification({
                        icon: "fa-check",
                        title:
                            failures.length > 0 ? `${title} (${failures.length} failed)` : title,
                        message:
                            failures.length > 0
                                ? failures.map((f) => `${baseName(f.source)}: ${f.error}`).join("\n")
                                : "",
                        timestamp: new Date().toISOString(),
                        type: failures.length > 0 ? "warning" : "info",
                        expiration: Date.now() + 5000,
                    });
                } else if (failures.length > 0) {
                    pushNotification({
                        icon: "fa-triangle-exclamation",
                        title: `Copy failed (${failures.length} file${failures.length === 1 ? "" : "s"})`,
                        message: failures
                            .map((f) => `${baseName(f.source)}: ${f.error}`)
                            .join("\n"),
                        timestamp: new Date().toISOString(),
                        type: "error",
                        expiration: Date.now() + 12000,
                    });
                }
            })();
        };

        root.addEventListener("dragover", onDragOver);
        root.addEventListener("dragleave", onDragLeave);
        root.addEventListener("drop", onDrop);
        onCleanup(() => {
            root.removeEventListener("dragover", onDragOver);
            root.removeEventListener("dragleave", onDragLeave);
            root.removeEventListener("drop", onDrop);
        });
    });

    return { isDragOver, dropMessage };
}
