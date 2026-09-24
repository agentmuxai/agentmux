// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * BtwOverlay — the floating, dismissable panel `/btw <question>` opens.
 *
 * Shows the question immediately (mounted the instant
 * `ctx.askSideQuestion` is called — see `useAgentCommands.ts`'s
 * `btwOverlay` signal), then the streamed answer as `WpsEvent.BtwAnswerChunk`
 * events arrive, scoped `block:<blockId>:btw:<requestId>` — the same generic
 * MPS-subscription mechanism `useCompactionStream.ts` uses, kept entirely
 * separate from the pane's real transcript/`agent-document` reducer: this
 * component owns its own local streaming state and touches neither
 * `dispatchDoc` nor `useAgentStream`, so it can never interfere with — or be
 * blocked by — the pane's own live turn.
 *
 * Each event's payload wraps the SAME tagged `AgentEvent` union every normal
 * turn streams (`frontend/types/srv-types.d.ts`) — see `mps-events.ts`'s
 * `BtwAnswerChunk` doc comment for the exact wire shape. Only
 * `assistant_text`/`done`/`error` are rendered; `tool_use`/`tool_result`
 * never appear on this path (the backend turn runs with every tool
 * disallowed) and `cost`/`compaction_boundary` aren't relevant to a
 * one-shot, no-session-resume answer.
 *
 * Chrome/mounting follows `SlashHelpPanel`'s precedent (Esc closes, a click
 * outside the panel closes) rather than inventing a new overlay mechanism,
 * but renders as a genuinely floating panel (position: absolute over the
 * whole pane, see `styles/_btw.scss`) rather than a composer-region row —
 * point 3/4 of the spec this was built against: "a transient UI element,
 * not a new Block/pane/tab" that must not block the pane's own turn.
 *
 * A second `/btw` asked while the overlay from a prior ask is still mounted
 * does NOT unmount/remount this component: `<Show when={commands.btwOverlay()}>`
 * in agent-view.tsx never sees a falsy value between two truthy asks, so
 * SolidJS reuses the same instance — `props.requestId` genuinely does cycle
 * old-id -> null -> new-id across two overlapping asks (reagentx P1 on PR
 * #3440 caught an earlier version of this file's own doc comment claiming
 * otherwise). This component's local `answer`/`streamError`/`done` signals
 * are therefore reset explicitly, keyed on `props.askId` — see that effect
 * below — rather than relying on remount to clear them.
 */

import { muxEventSubscribe } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { type JSX, Show, createEffect, createSignal, on, onCleanup, onMount } from "solid-js";
import { eventBelongsToPaneOf } from "@/util/focusutil";

export interface BtwOverlayProps {
    /** Block this overlay belongs to — scopes the MPS subscription. */
    blockId: string;
    /**
     * Unique per ask (`BtwOverlayState.askId`) — this component's local
     * streaming state is reset whenever this changes, since the component
     * instance itself may be reused across two overlapping asks. See this
     * file's own module doc comment.
     */
    askId: number;
    /** The question exactly as typed, shown immediately. */
    question: string;
    /** `null` until the backend request is accepted; see `BtwOverlayState`. */
    requestId: string | null;
    /** Set if the backend request itself failed. */
    error: string | null;
    onClose: () => void;
}

/**
 * `WpsEvent.BtwAnswerChunk` payload — mirrors `publish_chunk` in
 * `agentmux-srv/src/server/agent_handlers/side_question.rs`. See that
 * constant's doc comment (`mps-events.ts`) for the full contract.
 */
interface BtwAnswerChunkPayload {
    blockId: string;
    requestId: string;
    event: AgentEvent;
    done: boolean;
}

export function BtwOverlay(props: BtwOverlayProps): JSX.Element {
    const [answer, setAnswer] = createSignal("");
    const [streamError, setStreamError] = createSignal<string | null>(null);
    const [done, setDone] = createSignal(false);

    // Reset local streaming state whenever a NEW ask starts, keyed on
    // `askId` rather than `requestId`/`question` — this component's
    // instance is reused across two overlapping asks (see the module doc
    // comment), so without this a second `/btw` would inherit the first
    // one's leftover `answer` text and have new deltas appended onto it.
    // Fires on initial mount too (harmless — the signals already start
    // empty), and runs before the subscription effect below sees the
    // corresponding `requestId` reset to `null`, so there's no window where
    // a stale answer is visible under a fresh question.
    createEffect(on(() => props.askId, () => {
        setAnswer("");
        setStreamError(null);
        setDone(false);
    }));

    // Subscribe once `requestId` is known — nothing to scope the
    // subscription to before then. `requestId` DOES cycle old-id -> null ->
    // new-id across two overlapping asks (see the module doc comment) — this
    // effect re-runs each time, tearing down the previous subscription via
    // its own `onCleanup` before (re)subscribing, or just returning early
    // while `requestId` is transiently `null` between asks.
    createEffect(() => {
        const requestId = props.requestId;
        if (!requestId) return;
        const unsub = muxEventSubscribe({
            eventType: WpsEvent.BtwAnswerChunk,
            scope: `block:${props.blockId}:btw:${requestId}`,
            handler: (event: any) => {
                const data = event?.data as BtwAnswerChunkPayload | undefined;
                if (!data) return;
                // Bound to a local so TS narrows on `ev.type` — narrowing
                // does not reliably survive re-accessing `data.event` a
                // second time per switch case.
                const ev = data.event;
                switch (ev.type) {
                    case "assistant_text":
                        setAnswer((prev) => prev + ev.delta);
                        break;
                    case "done":
                        // `response` is the FULL final text, not a delta —
                        // it supersedes whatever was accumulated from
                        // streamed `assistant_text` chunks rather than
                        // appending to it.
                        if (ev.response) setAnswer(ev.response);
                        break;
                    case "error":
                        setStreamError(ev.message);
                        break;
                    // tool_use/tool_result never appear on this path (the
                    // backend turn runs with every tool disallowed);
                    // cost/compaction_boundary aren't relevant to a
                    // one-shot, no-session-resume answer.
                    default:
                        break;
                }
                // The outer `done` flag (not `event.type === "done"`) is the
                // authoritative completion signal — it also fires on a
                // terminal `error` event, which `event.type` alone would not
                // indicate as "done". See mps-events.ts's doc comment.
                if (data.done) {
                    setDone(true);
                }
            },
        });
        onCleanup(() => {
            try {
                unsub();
            } catch {
                /* ignore */
            }
        });
    });

    let rootRef: HTMLDivElement | undefined;
    const handleKeyDown = (e: KeyboardEvent): void => {
        // Escape in another pane must not close this pane's side question.
        if (!eventBelongsToPaneOf(e, rootRef)) return;
        if (e.key === "Escape") {
            e.preventDefault();
            props.onClose();
        }
    };

    const handlePointerDown = (e: PointerEvent): void => {
        // "Outside" means elsewhere in THIS pane — clicking into the pane
        // next to it (to read or type there) leaves this overlay open.
        if (!eventBelongsToPaneOf(e, rootRef)) return;
        if (rootRef && e.target instanceof Node && !rootRef.contains(e.target)) {
            props.onClose();
        }
    };

    onMount(() => {
        document.addEventListener("keydown", handleKeyDown, true);
        // Capture phase, and no preventDefault/stopPropagation anywhere in
        // this handler — a click outside the panel closes the overlay but
        // otherwise passes through untouched, so the pane underneath (the
        // composer, the transcript, another command) stays fully
        // interactive. This is the "non-blocking" requirement in practice.
        document.addEventListener("pointerdown", handlePointerDown, true);
    });
    onCleanup(() => {
        document.removeEventListener("keydown", handleKeyDown, true);
        document.removeEventListener("pointerdown", handlePointerDown, true);
    });

    return (
        <div class="btw-overlay" role="dialog" aria-label="Side question" ref={rootRef}>
            <div class="btw-overlay__header">
                <span class="btw-overlay__title">/btw</span>
                <button
                    type="button"
                    class="btw-overlay__close"
                    aria-label="Close side question"
                    onClick={() => props.onClose()}
                >
                    ×
                </button>
            </div>
            <div class="btw-overlay__question">{props.question}</div>
            <div class="btw-overlay__body">
                {/* props.error: the RPC call itself failed (never got a
                 * requestId). streamError: the RPC succeeded but the turn
                 * itself later reported an `error` event. Mutually
                 * exclusive in practice (streamError can't be set before a
                 * requestId exists to subscribe with), checked separately
                 * since they come from different sources. */}
                <Show when={props.error} keyed>
                    {(err) => <div class="btw-overlay__error">{err}</div>}
                </Show>
                <Show when={!props.error && streamError()} keyed>
                    {(err) => <div class="btw-overlay__error">{err}</div>}
                </Show>
                <Show when={!props.error && !streamError()}>
                    <Show when={props.requestId != null} fallback={<div class="btw-overlay__loading">Asking…</div>}>
                        <div class="btw-overlay__answer">
                            {answer()}
                            <Show when={!done()}>
                                <span class="btw-overlay__cursor" aria-hidden="true" />
                            </Show>
                        </div>
                    </Show>
                </Show>
            </div>
            <div class="btw-overlay__hint">
                <span>{done() ? "Done · Esc to close" : "Esc to close"}</span>
            </div>
        </div>
    );
}
