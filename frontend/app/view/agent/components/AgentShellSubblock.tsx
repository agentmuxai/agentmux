// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentShellSubblock — Phase 0 spike for
 * docs/specs/SPEC_AGENT_SHELL_XTERM_TERMINAL_2026_07_03.md.
 *
 * Mounts a real xterm.js + PTY terminal (Model A: a headless `term`
 * sub-block parented to the agent block, resolved in that spec's §4)
 * inside the composer details drawer. The sub-block id is persisted on
 * the agent block's meta (`term:shellsubblockid`) so it's created once
 * per pane and reused across drawer open/close — only the xterm
 * renderer is mounted/disposed here (drawer close); the PTY itself is
 * only killed when the pane closes (see agent-view.tsx's pane-level
 * onCleanup, which calls DeleteSubBlockCommand).
 */

import { createEffect, createMemo, createSignal, onCleanup, onMount, type Accessor, type JSX } from "solid-js";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { sendWSCommand } from "@/app/store/ws";
import { WOS, staticTabId } from "@/app/store/global";
import { stringToBase64 } from "@/util/util";
import { TermWrap } from "@/app/view/term/termwrap";

interface AgentShellSubblockProps {
    parentBlockId: string;
    cwd: string;
    existingSubBlockId: string | undefined;
    onSubBlockCreated: (subBlockId: string) => void;
    /**
     * The agent pane's OWN zoom factor (agent-view.tsx's `zoomFactor()`,
     * applied as CSS `zoom` on the `.agent-view` root — an ancestor of this
     * component). CSS `zoom` cascades to descendants, so without correction
     * the terminal's rendered glyph size would silently ride along with
     * whatever the outer pane is zoomed to, on top of this component's own
     * independent `term:zoom`. Dividing it out of the raw pixel fontSize we
     * feed xterm cancels that cascade — the two zooms become fully
     * independent controls (see `termFontSize` below).
     */
    agentPaneZoom: Accessor<number>;
    /**
     * Fired once the terminal has finished `init()`, handing the parent a
     * closure that writes pre-formatted (already ANSI-colored, no trailing
     * newline) text directly into the terminal's local render buffer via
     * `Terminal.write`. This never touches the PTY (that's
     * `sendDataHandler`/`blockinput` above, a separate path), so writes here
     * can't be interpreted as shell input. Used to redirect the agent pane's
     * activity-log lines into the shell instead of a separate log panel —
     * see agent-view.tsx's `log` wrapper.
     *
     * Unlike `AgentInstallModal.tsx`'s synthetic terminal (which has exactly
     * one writer — no PTY, no live stream), THIS `Terminal` instance also has
     * a second, independent writer: TermWrap's own `doTerminalWrite`, driven
     * by live PTY output arriving over the WS file-subject. `Terminal.write`
     * is safe to call from multiple sites — xterm.js internally queues and
     * processes writes strictly in call order (single `_innerWrite` in
     * flight at a time), so two overlapping calls never interleave at the
     * byte/escape-sequence level. But nothing coordinates *placement*: a log
     * line can still be queued in between two chunks of live PTY output,
     * landing mid-line (e.g. inside a user's in-progress prompt, or a TUI's
     * in-place redraw) — the closure below forces a leading `\r\n` so the
     * log line always starts its own fresh line regardless of where the
     * cursor happened to be. It intentionally calls `terminal.write`
     * directly rather than routing through TermWrap's `doTerminalWrite`:
     * that helper advances `ptyOffset`/`dataBytesProcessed`, which must only
     * ever track bytes that actually came from the "term" PTY file — this
     * synthetic text isn't part of that file, and inflating those counters
     * would desync the reconnect-offset accounting in
     * SPEC_TERMINAL_SCROLLBACK_PERSISTENCE_2026_07_23.md.
     */
    onTermReady?: (write: (text: string) => void) => void;
    /** Fired on unmount (drawer close) — pairs with `onTermReady` so the
     *  parent can drop its write closure rather than risk calling `.write`
     *  on a disposed `Terminal` (`TermWrap.dispose()` doesn't null it out). */
    onTermDispose?: () => void;
}

const BASE_FONT_SIZE = 13;

export const AgentShellSubblock = (props: AgentShellSubblockProps): JSX.Element => {
    let containerRef: HTMLDivElement | undefined;
    let termWrap: TermWrap | undefined;
    let resizeObserver: ResizeObserver | undefined;
    let disposed = false;

    const [subBlockId, setSubBlockId] = createSignal<string | undefined>(props.existingSubBlockId);
    const [error, setError] = createSignal<string | null>(null);

    // Reactive accessor for the sub-block's OWN meta — the same wave-object
    // atom mechanism TermViewModel uses (termViewModel.ts:86,237-246), just
    // targeting this sub-block's id instead of a top-level Terminal pane's.
    // This is what makes zoom a property of the terminal, not the agent pane.
    const subBlockAtom = createMemo(() => {
        const id = subBlockId();
        return id ? WOS.getWaveObjectAtom<Block>(`block:${id}`) : null;
    });

    const termZoom = createMemo(() => {
        const z = subBlockAtom()?.()?.meta?.["term:zoom"];
        if (z == null || typeof z !== "number" || isNaN(z)) return 1.0;
        return Math.max(0.5, Math.min(2.0, z));
    });
    const termFontSize = createMemo(() => {
        const paneZoom = props.agentPaneZoom() || 1;
        return Math.max(4, Math.min(64, Math.round((BASE_FONT_SIZE * termZoom()) / paneZoom)));
    });

    // Apply zoom-driven font-size changes to the live terminal in place —
    // mirrors term.tsx:234-241.
    createEffect(() => {
        const fs = termFontSize();
        if (termWrap?.terminal && termWrap.loaded) {
            termWrap.terminal.options.fontSize = fs;
            termWrap.handleResize();
        }
    });

    onMount(() => {
        // Ctrl+Wheel zoom, scoped to THIS terminal only — capture phase so it
        // intercepts before xterm's own bubble-phase wheel listener AND before
        // it can bubble up to app.tsx's document-level Ctrl+Wheel handler,
        // which would otherwise resolve `target.closest("[data-blockid]")` to
        // the AGENT pane's block (the nearest ancestor with that attribute,
        // since this sub-block is headless and never gets one) and zoom the
        // whole pane instead of just this shell. Mirrors term.tsx:212-231,
        // writing to the sub-block's OWN meta rather than the agent's.
        const handleCtrlWheel = (ev: WheelEvent) => {
            if (!ev.ctrlKey) return;
            const id = subBlockId();
            if (!id) return;
            ev.preventDefault();
            ev.stopPropagation();
            const STEP = 0.1;
            const delta = ev.deltaY > 0 ? -STEP : STEP;
            const next = Math.max(0.5, Math.min(2.0, Math.round((termZoom() + delta) * 100) / 100));
            void RpcApi.SetMetaCommand(TabRpcClient, {
                oref: WOS.makeORef("block", id),
                meta: { "term:zoom": next === 1.0 ? null : next } as any,
            });
        };
        containerRef?.addEventListener("wheel", handleCtrlWheel, { passive: false, capture: true });
        onCleanup(() => containerRef?.removeEventListener("wheel", handleCtrlWheel, { capture: true }));

        void (async () => {
            try {
                let id = subBlockId();
                if (id) {
                    // Reusing a sub-block id persisted on the parent's meta from a
                    // prior mount — but a sub-block, unlike its parent agent block,
                    // does not survive a full app restart (it's gone from the object
                    // store entirely, not just missing its in-memory controller).
                    // Verify it's still real before trusting it: resync throws
                    // "block <id> not found" for a stale reference. Reconnecting to
                    // a dead id otherwise renders whatever history is left (once
                    // persisted) with no live process behind it — the terminal
                    // looks normal but silently accepts no input. Confirmed live
                    // via CDP against a session that had been through several dev
                    // rebuild restarts.
                    try {
                        await RpcApi.ControllerResyncCommand(TabRpcClient, {
                            tabid: staticTabId(),
                            blockid: id,
                            forcerestart: false,
                        });
                    } catch (e) {
                        console.warn(
                            "AgentShellSubblock: existing sub-block is stale, creating a fresh one:",
                            e
                        );
                        id = undefined;
                    }
                }
                if (!id) {
                    const oref = await RpcApi.CreateSubBlockCommand(TabRpcClient, {
                        parentblockid: props.parentBlockId,
                        blockdef: {
                            meta: {
                                view: "term",
                                controller: "shell",
                                "cmd:cwd": props.cwd,
                            },
                        },
                    });
                    // ORef wire format is always "<otype>:<oid>" (wos.ts makeORef) —
                    // oid is a UUID, never contains a colon, so a single split is safe.
                    id = oref.slice(oref.indexOf(":") + 1);
                    setSubBlockId(id);
                    props.onSubBlockCreated(id);
                }
                if (disposed || !containerRef) return;
                const wrap = new TermWrap(
                    id,
                    containerRef,
                    {
                        fontSize: termFontSize(),
                        fontFamily: "Hack",
                        allowTransparency: false,
                        scrollback: 2000,
                        allowProposedApi: true,
                    },
                    {
                        useWebGl: true,
                        // Bare sendDataHandler mirroring TermViewModel's fast path
                        // (termViewModel.ts:370-379) — blockinput, not the
                        // controllerinput RPC, so consecutive keystrokes stay in
                        // TCP order. No chunked-paste handling for this spike.
                        sendDataHandler: (data: string) => {
                            sendWSCommand({
                                wscommand: "blockinput",
                                blockid: id,
                                inputdata64: stringToBase64(data),
                            } as BlockInputWSCommand);
                        },
                    }
                );
                termWrap = wrap;
                await wrap.init();
                if (!disposed) {
                    props.onTermReady?.((text: string) => {
                        // Leading \r\n forces this line to start fresh regardless of
                        // where the cursor was left by concurrently-arriving live PTY
                        // output — see the onTermReady doc comment above.
                        termWrap?.terminal.write(`\r\n${text}\r\n`);
                    });
                }

                // Reflow the PTY grid whenever the container is resized — drag-
                // resizing the details drawer (ResizableDetailsDrawer), the pane
                // itself, or the window. Without this the container can change
                // size (e.g. via the drawer's drag handle) with the terminal
                // never re-fitting to it. Mirrors term.tsx's rszObs pattern.
                // Plain DOM API, not a Solid primitive, so it's safe to set up
                // here post-await; teardown is registered synchronously below
                // via the `resizeObserver` closure var, not a second onCleanup.
                if (!disposed && containerRef) {
                    resizeObserver = new ResizeObserver(() => {
                        termWrap?.handleResize_debounced();
                    });
                    resizeObserver.observe(containerRef);
                }
            } catch (e) {
                // Without this, a rejection here (e.g. createsubblock failing)
                // was an unhandled promise rejection and the drawer silently
                // never rendered a terminal — no user-facing error at all.
                console.error("AgentShellSubblock: failed to start shell:", e);
                if (!disposed) setError(e instanceof Error ? e.message : String(e));
            }
        })();
    });

    onCleanup(() => {
        disposed = true;
        termWrap?.dispose();
        resizeObserver?.disconnect();
        props.onTermDispose?.();
    });

    return (
        <div class="agent-shell-subblock" ref={containerRef}>
            {error() && <div class="agent-shell-subblock-error">Shell failed to start: {error()}</div>}
        </div>
    );
};
