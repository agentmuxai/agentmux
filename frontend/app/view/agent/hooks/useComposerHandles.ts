// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the composer registers on its AgentViewModel while it is on screen: the voice handle the mic writes
 * through, and the textarea for the pane's focus contract.
 */

import { onCleanup, onMount } from "solid-js";
import type { PaneVoiceHandle } from "@/app/hook/useVoiceInput";
import { focusManager } from "@/app/store/focusManager";
import type { AgentViewModel } from "../agent-model";
import { focusComposerWhenReady, takeComposerFocusRequest } from "../../../agent-model/composer-focus";

// ── Voice input handle ───────────────────────────────────────────
// Registers a textarea-backed PaneVoiceHandle on the AgentViewModel
// so the frame-header mic button can write voice transcripts into
// the composer. `baseValue` is the prefix the user typed (or the
// last appended-final state) — interim updates render *after* it
// without committing, final updates fold in and become the new
// base. Cleared on unmount.
export function useComposerVoiceHandle(vm: AgentViewModel | undefined, textarea: () => HTMLTextAreaElement | undefined): void {
    onMount(() => {
        if (!vm) return;
        // Two pieces of state interleave with the user's manual typing:
        //   * baseValue — the textarea content as of the last user-or-
        //     final-voice event. Interim updates render AFTER it but
        //     don't promote into it.
        //   * lastVoiceWrite — the exact string we last wrote to the
        //     textarea. If ta.value diverges from this between events,
        //     the user typed something manually and we re-snapshot
        //     baseValue from ta.value so the next voice event preserves
        //     what they added.
        //
        // An earlier guard (`!startsWith(baseValue)`) never fired for the
        // common case where the user APPENDED to existing content —
        // their additions were silently clobbered by the next voice
        // append (#930).
        let baseValue = textarea()?.value ?? "";
        let lastVoiceWrite = baseValue;
        const handle: PaneVoiceHandle = {
            appendFinal: (text) => {
                const ta = textarea();
                if (!ta) return;
                if (ta.value !== lastVoiceWrite) baseValue = ta.value;
                const next = baseValue + text + " ";
                ta.value = next;
                ta.dispatchEvent(new Event("input", { bubbles: true }));
                baseValue = next;
                lastVoiceWrite = next;
            },
            setInterim: (text) => {
                const ta = textarea();
                if (!ta) return;
                if (ta.value !== lastVoiceWrite) baseValue = ta.value;
                const next = baseValue + text;
                ta.value = next;
                ta.dispatchEvent(new Event("input", { bubbles: true }));
                lastVoiceWrite = next;
            },
        };
        vm.voiceTargetRef.current = handle;
        onCleanup(() => {
            if (vm.voiceTargetRef.current === handle) {
                vm.voiceTargetRef.current = null;
            }
        });
    });
}

// ── Focus handle ──────────────────────────────────────────────────
// Registers the live textarea on the AgentViewModel so giveFocus() (the
// shared pane-selection focus contract — see focusManager.ts) can
// actually focus it instead of the `return false` stub it used to be.
// SPEC_PANE_SELECT_AUTOFOCUS_2026_09_22.md §3a: same ref-handoff shape
// as the voice handle above, which is the precedent this follows.
//
// Also claims focus immediately if this pane is ALREADY the active
// tab's selected pane at the moment the textarea becomes available —
// covers a freshly created agent pane (mounts after already being
// marked focused) and switching into a tab whose agent pane wasn't
// mounted yet, neither of which `focusManager.refocusNode()`'s own
// one-shot attempt can reach.
//
// Right after a launch (a one-shot request from the launch path) the
// claim is replaced by a retrying one: a launch started from a modal
// mounts this footer while the modal is still open, which
// claimFocusOnMount deliberately skips, so it would never land.
// SPEC_AGENT_PANE_HOVER_CLOSE_FOCUS_REFINEMENTS_2026_09_23.md §3.
export function useComposerFocusHandle(vm: AgentViewModel | undefined, textarea: () => HTMLTextAreaElement | undefined): void {
    onMount(() => {
        const ta = textarea();
        if (!vm || !ta) return;
        vm.focusTargetRef.current = ta;
        const cancelLaunchFocus = takeComposerFocusRequest(vm.blockId) ? focusComposerWhenReady(ta, vm.blockId) : null;
        onCleanup(() => {
            cancelLaunchFocus?.();
            if (vm.focusTargetRef.current === ta) {
                vm.focusTargetRef.current = null;
            }
        });
        if (!cancelLaunchFocus) focusManager.claimFocusOnMount(vm.blockId, () => vm.giveFocus());
    });
}
