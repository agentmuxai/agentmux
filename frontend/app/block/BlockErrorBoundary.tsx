// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Per-block error boundary — localized fallback UI so one pane's renderer
// fault cannot blank the whole tab.
//
// Retro: docs/retro/retro-agent-pane-cascade-replacechild-2026-05-23.md.
//
// Design contract (cascade-safe):
//   - The fallback reads ONLY from the props the boundary passes in. It does
//     NOT subscribe to the broken pane's reactive graph (no agent-pane-state
//     store reads, no documentAtom, no useAtomValue against per-pane atoms).
//     The pane that just crashed may have a half-flushed reactive owner; any
//     subscription into it from the fallback would re-throw.
//   - The fallback's "Reload pane" action delegates to SolidJS's
//     `ErrorBoundary` `reset` callback, which re-mounts the children fresh —
//     i.e., re-runs the component's entire reactive setup. Any half-flushed
//     state in the broken pane is discarded.
//   - On catch, we forward a structured payload to the host via the
//     `fe_log_structured` IPC channel — same channel used by
//     `frontend/log/log-pipe.ts` and `frontend/log/error-forwarder.ts` — so
//     the host log shows block_id + view_type + error_name + stack alongside
//     the existing cascade-detection warnings.

import { invokeCommand } from "@/app/platform/ipc";
import { resolveStack, resolveStackSync, type ResolveStatus } from "@/log/source-map-resolver";
import { getTrail } from "@/log/render-trail";
import { ErrorBoundary as SolidErrorBoundary, createSignal, Show } from "solid-js";
import type { JSX } from "solid-js";

export interface BlockErrorBoundaryProps {
    /** The block this boundary protects. Logged in the host trace. */
    blockId: string;
    /** Optional view type ("agent", "term", "browser", ...) for the host trace. */
    viewType?: string;
    /**
     * Optional close handler. When provided, the fallback renders a "Close
     * pane" button that destroys the block. Skipped if missing (e.g. in
     * tests that don't have a layout).
     */
    onClose?: () => void;
    children: JSX.Element;
}

/**
 * Pure side effect: forward the catch to the host log, with the stack
 * rewritten through the source-map resolver and a snapshot of the
 * render trail attached (see `frontend/log/render-trail.ts`).
 *
 * Why the trail matters: SolidJS reconciler crashes throw from deep
 * inside `web.js` with no user-land frames — the effect that scheduled
 * the bad DOM op has already returned. The trail captures recent
 * reactive activity (reducer actions, render-effect entries, etc.) so
 * log readers can see "what was happening just before the throw."
 */
function logBoundaryCatch(blockId: string, viewType: string | undefined, err: Error): void {
    try {
        const errName = err?.name ?? "Error";
        const errMessage = err?.message ?? String(err);
        const rawStack = err?.stack ?? null;

        // Sync-first resolve: whichever frames are already cached get
        // rewritten now; anything pending falls into the async
        // follow-up below. Mirrors the pattern in
        // `frontend/log/error-forwarder.ts`.
        let stackForLog: string | null = rawStack;
        let stackResolved: ResolveStatus = "failed";
        if (rawStack) {
            try {
                const sync = resolveStackSync(rawStack);
                stackForLog = sync.resolved;
                stackResolved = sync.status;
            } catch {
                stackForLog = rawStack;
                stackResolved = "failed";
            }
        }

        const trailSnapshot = (() => {
            try {
                return getTrail();
            } catch {
                return [];
            }
        })();

        // Fire-and-forget — never let logging compound the rendering fault.
        invokeCommand("fe_log_structured", {
            level: "error",
            module: "block-error-boundary",
            message: `[block-error-boundary] ${errName}: ${errMessage} (block=${blockId.substring(0, 7)}, view=${viewType ?? "?"})`,
            data: {
                block_id: blockId,
                view_type: viewType ?? null,
                error_name: errName,
                error_message: errMessage,
                error_stack: stackForLog,
                error_stack_raw: rawStack,
                stack_resolved: stackResolved,
                render_trail: trailSnapshot,
            },
        }).catch(() => {});

        // If the synchronous resolve couldn't reach every frame, kick
        // off the async load and emit a follow-up entry once the
        // missing `.map` files are fetched. Crash investigations
        // typically need the fully-resolved stack, so the follow-up
        // is high-value even though it lands seconds later.
        if (stackResolved === "partial" && rawStack) {
            const stackToResolve = rawStack;
            void resolveStack(stackToResolve)
                .then((fullyResolved) => {
                    try {
                        invokeCommand("fe_log_structured", {
                            level: "warn",
                            module: "block-error-boundary",
                            message: `[block-error-boundary] (stack-resolved) ${errName}: ${errMessage} (block=${blockId.substring(0, 7)})`,
                            data: {
                                block_id: blockId,
                                view_type: viewType ?? null,
                                error_name: errName,
                                error_message: errMessage,
                                error_stack: fullyResolved.resolved,
                                error_stack_raw: stackToResolve,
                                stack_resolved: fullyResolved.status,
                            },
                        }).catch(() => {});
                    } catch {
                        // swallow
                    }
                })
                .catch(() => {
                    // swallow
                });
        }
    } catch {
        // swallow — logging must never break the fallback UI
    }
}

/** Fallback UI. Reads ONLY from the props that are passed in. */
function BlockErrorFallback(props: {
    blockId: string;
    viewType?: string;
    error: Error;
    reset: () => void;
    onClose?: () => void;
}): JSX.Element {
    const [stackOpen, setStackOpen] = createSignal(false);
    const errName = () => props.error?.name ?? "Error";
    const errMessage = () => props.error?.message ?? String(props.error);
    const errStack = () => props.error?.stack ?? "";
    const shortId = () => props.blockId.substring(0, 7);

    return (
        <div class="block-error-fallback" role="alert" data-testid="block-error-fallback">
            <div class="block-error-fallback-header">
                <i class="fa-sharp fa-solid fa-triangle-exclamation block-error-fallback-icon" aria-hidden="true" />
                <div class="block-error-fallback-title">This pane crashed</div>
            </div>
            <div class="block-error-fallback-body">
                <div class="block-error-fallback-message">
                    <strong>{errName()}:</strong> {errMessage()}
                </div>
                <div class="block-error-fallback-meta">
                    block <code>{shortId()}</code>
                    <Show when={props.viewType}>
                        {" · view "}
                        <code>{props.viewType}</code>
                    </Show>
                </div>
                <Show when={errStack()}>
                    <button
                        type="button"
                        class="block-error-fallback-stack-toggle"
                        onClick={() => setStackOpen((v) => !v)}
                        aria-expanded={stackOpen()}
                    >
                        {stackOpen() ? "Hide stack" : "Show stack"}
                    </button>
                    <Show when={stackOpen()}>
                        <pre class="block-error-fallback-stack">{errStack()}</pre>
                    </Show>
                </Show>
            </div>
            <div class="block-error-fallback-footer">
                <button
                    type="button"
                    class="modal-btn"
                    onClick={() => props.reset()}
                    data-testid="block-error-fallback-reload"
                >
                    Reload pane
                </button>
                <Show when={props.onClose}>
                    <button
                        type="button"
                        class="modal-btn"
                        onClick={() => props.onClose?.()}
                        data-testid="block-error-fallback-close"
                    >
                        Close pane
                    </button>
                </Show>
            </div>
        </div>
    );
}

/**
 * Wrap each block's body so a renderer exception inside one pane only blanks
 * THAT pane. Other panes in the same tab + the layout chrome stay alive.
 */
export function BlockErrorBoundary(props: BlockErrorBoundaryProps): JSX.Element {
    return (
        <SolidErrorBoundary
            fallback={(err: Error, reset: () => void) => {
                logBoundaryCatch(props.blockId, props.viewType, err);
                return (
                    <BlockErrorFallback
                        blockId={props.blockId}
                        viewType={props.viewType}
                        error={err}
                        reset={reset}
                        onClose={props.onClose}
                    />
                );
            }}
        >
            {props.children}
        </SolidErrorBoundary>
    );
}
