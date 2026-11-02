// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The My agents list's row exit: a deleted row plays the "poof" and the rows around it reflow; a row that
// only left the list because of the name filter goes at once. Split out of MyAgentsList.tsx (#13044 S6).

import { shouldReduceMotion } from "@/app/motion/motion";
import type { Accessor, Setter } from "solid-js";

export function rowExitHandlers(deletingIds: Accessor<Set<string>>, setDeletingIds: Setter<Set<string>>) {
    return {
        onBeforeExit: (el: Element): void => {
            const id = (el as HTMLElement).dataset.definitionId;
            const isGenuineDelete = id !== undefined && deletingIds().has(id);
            if (shouldReduceMotion("transition") || !isGenuineDelete) return;
            const r = (el as HTMLElement).getBoundingClientRect();
            (el as HTMLElement).style.width = `${r.width}px`;
            (el as HTMLElement).style.height = `${r.height}px`;
        },
        onExit: (el: Element, done: () => void): void => {
            const id = (el as HTMLElement).dataset.definitionId;
            const isGenuineDelete = id !== undefined && deletingIds().has(id);
            const clearDeleting = () => {
                if (id === undefined) return;
                setDeletingIds((prev) => {
                    if (!prev.has(id)) return prev;
                    const next = new Set(prev);
                    next.delete(id);
                    return next;
                });
            };
            if (!isGenuineDelete || shouldReduceMotion("transition")) {
                clearDeleting();
                done();
                return;
            }
            const htmlEl = el as HTMLElement;
            htmlEl.classList.add("agent-row-exit-active");
            htmlEl.addEventListener(
                "animationend",
                () => {
                    clearDeleting();
                    done();
                },
                { once: true }
            );
        },
    };
}
