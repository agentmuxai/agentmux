// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The composer's placeholder, in precedence order:
 *   1. Ghost-text next-prompt suggestion (term:next_prompt_suggestion) —
 *      Haiku-predicted next message, accept with Tab (see AgentFooter's
 *      handleKeyDown). Only meaningful while the box is empty, which is exactly
 *      when a native `placeholder` attribute renders anyway.
 *   2. "Speak to <agent>…" while voice is listening AND this pane owns
 *      the session, so it's obvious transcription is live and routed here.
 *   3. The default "Send message to <agent>…" prompt.
 * Reactive: reads the block meta atom + the voice singleton's SignalAtoms
 * so it flips the instant either changes. term:next_prompt_suggestion is
 * NOT cleared by editing the composer (handleInput/handleKeyDown deliberately
 * leave it alone) — typing over it and deleting back to empty shows the same
 * suggestion again, rather than falling through to band 3. It's cleared only
 * by an actual new turn starting or the session ending (useNextPromptSuggestion.ts's
 * own guards). See docs/specs/SPEC_NEXT_PROMPT_SUGGESTION_RESTORE_ON_CLEAR_2026_08_10.md.
 */

import { createMemo, createSignal } from "solid-js";
import { getVoiceSession } from "@/app/hook/useVoiceInput";
import { META_SUGGESTION, META_SUGGESTION_GEN } from "@/app/store/meta-keys";
import type { AgentViewModel } from "../agent-model";

export function useComposerPlaceholder(opts: { viewModel: AgentViewModel | undefined; agentName: () => string }): {
    /** The suggestion as the composer shows it: absent while masked. */
    visibleSuggestion: () => string | undefined;
    placeholder: () => string;
    /** Hide the suggestion that is current right now (called as a send empties the composer). */
    maskCurrentSuggestion: () => void;
} {
    const voice = getVoiceSession();
    const vm = opts.viewModel;
    // Masks a specific stale WRITE (not a text value) from band 1 — set by
    // the send right as it synchronously empties the composer. Without
    // this, every send has a real window where the box is empty (so the
    // native placeholder wants to render) but the PREVIOUS turn's
    // suggestion is still sitting in meta, since useNextPromptSuggestion.ts
    // guard 1 clears it via an async fire-and-forget RPC, not synchronously
    // with the send — the old suggestion would briefly flash as placeholder
    // text before that RPC round-trips.
    //
    // Compares term:next_prompt_suggestion_gen, a monotonic counter
    // useNextPromptSuggestion.ts bumps on every write to
    // term:next_prompt_suggestion (fresh or cleared) — NOT the suggestion
    // text itself. An earlier version compared text values directly: if a
    // later turn's genuinely fresh suggestion happened to be the exact same
    // string masked at the previous send (a plausible repeat, e.g. Haiku
    // predicting "Run the tests" twice), value-equality would suppress a
    // legitimate current suggestion until the next send (#2515). The
    // generation counter can't collide that way: any real write (same text
    // or not) bumps it.
    //
    // A signal, not a plain variable: the placeholder memo below must
    // actually re-run the instant this is set, and mutating a bare closure
    // variable doesn't trigger Solid's reactivity at all.
    const [maskedAtSend, setMaskedAtSend] = createSignal<number | undefined>(undefined);
    // Tab/→ accept reads this too, so a suggestion hidden at send time can't be
    // inserted while its clear is still on its way.
    const visibleSuggestion = createMemo((): string | undefined => {
        const meta = vm?.blockAtom()?.meta;
        const suggestion = meta?.[META_SUGGESTION] as string | undefined;
        const suggestionGen = meta?.[META_SUGGESTION_GEN] as number | undefined;
        return suggestion && suggestionGen !== maskedAtSend() ? suggestion : undefined;
    });
    const placeholder = createMemo(() => {
        const suggestion = visibleSuggestion();
        if (suggestion) return suggestion;
        const listeningHere = !!vm && voice.isListening() && voice.currentTargetId() === vm.blockId;
        return listeningHere ? `Speak to ${opts.agentName()}...` : `Send message to ${opts.agentName()}...`;
    });
    const maskCurrentSuggestion = () => setMaskedAtSend(vm?.blockAtom()?.meta?.[META_SUGGESTION_GEN] as number | undefined);
    return { visibleSuggestion, placeholder, maskCurrentSuggestion };
}
