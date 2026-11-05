// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The document listeners that live while the panel is open (keys, outside clicks, focus leaving) and the trigger's held width. */

import { createEffect, onCleanup } from "solid-js";
import type { DropupActions } from "./runtime-dropup-rows";
import type { DropupState } from "./runtime-dropup-state";

/** Creates two effects: the listeners scoped to the open panel, then the release of the held width on close. */
export function useDropupListeners(s: DropupState, act: DropupActions) {
    // True only while DOM focus is actually inside the trigger or panel.
    // The panel deliberately stays open across selections (§9.2), so a user
    // can select a row and then Tab — or something else calls
    // textareaRef.focus() programmatically (e.g. AgentFooter's
    // acceptCompletion()) — without any mousedown outside the panel ever
    // firing. Without this guard, handleKeyDown would still be attached and
    // would swallow the next Enter/letter typed into the now-focused
    // composer textarea.
    const focusWithinDropup = (): boolean => {
        const active = document.activeElement;
        if (!active) return false;
        return !!(s.referenceEl?.contains(active) || s.floatingEl?.contains(active));
    };

    const handleKeyDown = (e: KeyboardEvent) => {
        if (!focusWithinDropup()) return;
        if (e.key === "ArrowDown") {
            e.preventDefault();
            act.move(1);
        } else if (e.key === "ArrowUp") {
            e.preventDefault();
            act.move(-1);
        } else if (e.key === "Enter") {
            e.preventDefault();
            void act.applySelection(s.selectedOptIndex());
        } else if (e.key === "Escape") {
            e.preventDefault();
            s.setOpen(false);
        } else if (!e.ctrlKey && !e.metaKey && !e.altKey && e.key.length === 1 && /[a-z0-9]/i.test(e.key)) {
            const k = e.key.toLowerCase();
            const idx = act.build().options.findIndex((o) => o.label.toLowerCase().startsWith(k));
            if (idx >= 0) {
                e.preventDefault();
                s.setSelectedOptIndex(idx);
            }
        }
    };

    const handleClickOutside = (e: MouseEvent) => {
        const target = e.target as Node;
        if (s.referenceEl?.contains(target) || s.floatingEl?.contains(target)) return;
        s.setOpen(false);
    };

    // Closes the panel as soon as focus moves outside it by ANY means (Tab,
    // a programmatic .focus() call elsewhere, etc.), not just a mousedown —
    // keeps the panel from staying invisibly "open" (and its listeners
    // attached) once the user has clearly moved on. Belt-and-suspenders with
    // the focusWithinDropup() guard in handleKeyDown above.
    const handleFocusChange = () => {
        if (!focusWithinDropup()) s.setOpen(false);
    };

    // All three listeners are scoped to the panel's open lifetime via this
    // effect (not onMount, which would span the trigger's entire mount
    // lifetime).
    createEffect(() => {
        if (!s.open()) return;
        document.addEventListener("mousedown", handleClickOutside);
        document.addEventListener("keydown", handleKeyDown, true);
        document.addEventListener("focusin", handleFocusChange);
        onCleanup(() => {
            document.removeEventListener("mousedown", handleClickOutside);
            document.removeEventListener("keydown", handleKeyDown, true);
            document.removeEventListener("focusin", handleFocusChange);
        });
    });

    createEffect(() => {
        if (!s.open()) s.setHeldWidth(null);
    });
}
