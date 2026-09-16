// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentShellSubblock — Phase 0 spike.
 *
 * Mounts a real xterm.js + PTY terminal (Model A: a headless `term`
 * sub-block parented to the agent block) inside the composer details
 * drawer. The sub-block id is persisted on the agent block's meta
 * (`term:shellsubblockid`) so it's created once per pane and reused
 * across drawer open/close — only the xterm renderer is
 * mounted/disposed here (drawer close); the PTY itself is only killed
 * when the pane closes (see agent-view.tsx's pane-level onCleanup,
 * which calls DeleteSubBlockCommand).
 */

import { BrainSpinner } from "@/app/element/BrainSpinner";
import { atoms, staticTabId, WOS } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { waveEventSubscribe } from "@/app/store/wps";
import { WpsEvent } from "@/app/store/wps-events";
import { sendWSCommand } from "@/app/store/ws";
import { TermWrap } from "@/app/view/term/termwrap";
import { stringToBase64 } from "@/util/util";
import { createEffect, createMemo, createSignal, onCleanup, onMount, Show, type Accessor, type JSX } from "solid-js";

// Matches browser-view.tsx's LOADING_SPINNER_FADE_MS / BrainSpinner.scss's
// is-fading transition duration — keep in sync if either changes.
const SHELL_LOADING_SPINNER_FADE_MS = 200;

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
    /**
     * Fired when this shell's PROCESS ended cleanly — the human typed `exit`
     * (SPEC_AGENT_PANE_SHELL_EXIT_COLLAPSES_DRAWER_2026_09_15.md). Distinct
     * from `onTermDispose`, which also fires on ordinary unmount (drawer
     * close, pane dispose) and so cannot carry "the process ended" without
     * becoming ambiguous.
     *
     * Clean exits only (exit code 0), per that spec's §5 open question: a
     * crash or a backend restart produces the same `STATUS_DONE`, and
     * collapsing the drawer there would hide the very output the human needs
     * to read. A non-zero exit leaves the drawer open with its scrollback
     * intact.
     */
    onShellExited?: () => void;
}

const BASE_FONT_SIZE = 13;

/**
 * Waits for an already-in-flight WOS fetch for `oref` to settle (succeed or
 * fail), WITHOUT triggering a new one — see the onMount IIFE below for why a
 * second fetch must be avoided (reagentx P1 on #2522: `subBlockAtom`'s
 * `createMemo` already eagerly fetches this exact oref at component
 * construction). `getWaveObjectLoadingAtom` returns `null` while loading and
 * `false` once settled (regardless of whether the value ended up populated
 * or null) — see its doc comment in wos.ts. Bounded by `timeoutMs` since a
 * genuine network failure can leave the loading atom stuck at "loading"
 * forever (wos.ts's own comment on GetObject rejections other than a
 * definitive "not found").
 */
async function waitForWaveObjectSettled(oref: string, timeoutMs = 2000): Promise<void> {
    const loadingAtom = WOS.getWaveObjectLoadingAtom(oref);
    const start = Date.now();
    while (loadingAtom() === null) {
        if (Date.now() - start >= timeoutMs) return;
        await new Promise<void>((resolve) => setTimeout(resolve, 16));
    }
}

export const AgentShellSubblock = (props: AgentShellSubblockProps): JSX.Element => {
    let containerRef: HTMLDivElement | undefined;
    let termWrap: TermWrap | undefined;
    let resizeObserver: ResizeObserver | undefined;
    let disposed = false;
    // Bumped at the start of every `attachShell` call — an in-flight call
    // whose own generation no longer matches this by the time one of its
    // `await`s resolves knows a NEWER attach has since started (another
    // externally-driven repoint arriving before this one finished) and
    // must abandon itself rather than commit `termWrap`/`resizeObserver`
    // out from under the newer, correct attachment (ReAgent P1 on #3257).
    let attachGeneration = 0;

    const [subBlockId, setSubBlockId] = createSignal<string | undefined>(props.existingSubBlockId);
    const [error, setError] = createSignal<string | null>(null);
    // True once we've resolved (or determined we don't need) the persisted
    // term:zoom for this sub-block, and are safe to construct TermWrap with
    // the FINAL font size — see SPEC_AGENT_SHELL_ZOOM_SEED_RACE_2026-08-10.md.
    // Gates both terminal construction and the loading overlay below.
    const [zoomSeeded, setZoomSeeded] = createSignal(false);
    // True once TermWrap.init() has resolved. A reactive replacement for
    // reading termWrap.loaded (a plain class field) directly inside
    // createEffect below — the plain field doesn't subscribe the effect to
    // its later transition, so a font-size correction landing in the narrow
    // window between TermWrap construction and init() resolving could
    // previously be silently dropped forever (same spec, §2.2.6).
    const [wrapLoaded, setWrapLoaded] = createSignal(false);

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

    // Agent-lock gating (SPEC_AGENT_INTERACTIVE_PTY_SHELL_API_2026_09_10.md):
    // while an agent is actively driving THIS shell via PtyShellInput/
    // PtyShellResize, the human's own keystrokes are dropped instead of
    // forwarded — the two typing into the same PTY at once would interleave
    // into garbage. `term:agentlockuntil` is an absolute expiry timestamp
    // (epoch ms) the backend stamps on every successful agent write; gating
    // on it here needs no server push to UN-lock — a quiet agent just lets
    // the timestamp lapse, and `nowTick` (below) re-evaluates this memo on
    // its own schedule so the UI actually notices the moment it does.
    // Deliberately a lease, not an explicit lock/unlock the agent could
    // fail to release (crash, error) — see the backend const's own doc
    // comment (`AGENT_LOCK_WINDOW_MS`) for why.
    const [nowTick, setNowTick] = createSignal(Date.now());
    const tickInterval = setInterval(() => setNowTick(Date.now()), 500);
    onCleanup(() => clearInterval(tickInterval));

    const agentLockedUntil = createMemo(() => {
        const v = subBlockAtom()?.()?.meta?.["term:agentlockuntil"];
        return typeof v === "number" ? v : 0;
    });
    const agentLocked = createMemo(() => agentLockedUntil() > nowTick());

    // Process-exit detection (SPEC_AGENT_PANE_SHELL_EXIT_COLLAPSES_DRAWER_2026_09_15.md).
    //
    // Subscribes to `controllerstatus` for THIS SUB-BLOCK's id — not the
    // parent agent block's, which agent-view.tsx separately subscribes to for
    // turn tracking. Without this the drawer never learns its shell died: it
    // keeps rendering a dead terminal that silently accepts no input, and the
    // human who just typed `exit` has to close the drawer by hand.
    //
    // Backend close-on-exit cannot do this job. It is deliberately scoped to
    // top-level panes (`!is_sub_block`, shell/lifecycle.rs) because its close
    // action deletes the block and prunes the tab layout — for a drawer shell
    // that leaves the parent's `term:shellsubblockid` dangling and the
    // attach-or-create path respawns a replacement, which is the respawn loop
    // SPEC_TERM_EXIT_RESPAWN_LOOP_2026_09_15.md §11 fixed. So the collapse is
    // driven here, from the event the backend already publishes.
    //
    // Fires at most once per mount: `STATUS_DONE` can be published more than
    // once for one block (a status re-publish, a resync), and the parent's
    // handler tears down real state — deleting a sub-block twice races the
    // second delete against a fresh shell the human may have opened since.
    //
    // Subscribed from an effect, not `onMount`: a freshly created shell has
    // no id at mount (the async IIFE below assigns it), so a one-shot mount
    // subscription would bind to an empty scope and never fire — the common
    // case of opening the drawer for the first time. The effect re-subscribes
    // when the id arrives, and its `onCleanup` unsubscribes the previous one.
    // How this shell's process ended, if we have been told at all — set from
    // the `controllerstatus` subscription below (live OR replayed). `undefined`
    // means "no exit observed", which is NOT the same as "exited cleanly" and
    // must never be treated as permission to delete anything.
    let lastObservedExitCode: number | undefined;

    createEffect(() => {
        const id = subBlockId();
        if (!id) return;
        // Per-subscription, not per-mount: a fresh shell created after this
        // one exits gets its own scope, and must arm independently.
        let sawRunning = false;
        let exitNotified = false;
        const unsub = waveEventSubscribe({
            eventType: WpsEvent.ControllerStatus,
            scope: WOS.makeORef("block", id),
            handler: (event) => {
                const data = event?.data as { shellprocstatus?: unknown; shellprocexitcode?: unknown } | undefined;
                if (!data) return;
                if (data.shellprocstatus === "running") {
                    sawRunning = true;
                    return;
                }
                if (data.shellprocstatus !== "done") return;
                // Record HOW it ended before deciding whether to act on it.
                // The attach path below consults this to tell an exit it may
                // safely clean up (clean) from one whose scrollback is the
                // only record of what went wrong (crash) — see its own
                // comment. Set for replayed exits too, which is the whole
                // point: a crash that happened while the drawer was closed is
                // knowable ONLY from the replay.
                lastObservedExitCode = typeof data.shellprocexitcode === "number" ? data.shellprocexitcode : 0;
                // ONLY act on an exit we watched happen.
                //
                // ReAgent P0 on PR #3253: `controllerstatus` is published with
                // `persist: 1` precisely so a subscriber is replayed the
                // CURRENT status on first subscribe to a scope
                // (blockcontroller/mod.rs, wps.rs's `replay_to_route`). So a
                // shell that exited while the drawer was closed — an agent
                // finishing work in the shared shell, the supported case in
                // §3.3 of this feature's spec — delivers its old `done`
                // synchronously the moment this effect subscribes. Acting on
                // it would collapse the drawer the human just opened *to read
                // that output*, and the teardown deletes the sub-block, taking
                // its persisted `term` file with it. Not hidden — gone.
                //
                // A replayed exit is distinguishable from a live one by what
                // came before it: a live exit is always preceded by the
                // `running` status this shell published while it was alive
                // (replayed too, for a shell that IS alive). No `running`
                // means the shell was already over before we got here, which
                // is the attach path's problem — it resyncs with `norespawn`
                // and falls through to creating a genuinely fresh shell.
                if (!sawRunning) return;
                // `shellprocexitcode` is `#[serde(default)]` on the wire, so a
                // clean exit can arrive as 0 or be omitted entirely — both
                // mean "exited 0". Anything else is a crash/failure and keeps
                // the drawer open so its output stays readable.
                const code = typeof data.shellprocexitcode === "number" ? data.shellprocexitcode : 0;
                if (code !== 0) return;
                if (exitNotified) return;
                exitNotified = true;
                props.onShellExited?.();
            },
        });
        onCleanup(() => unsub());
    });

    const termFontSize = createMemo(() => {
        const paneZoom = props.agentPaneZoom() || 1;
        return Math.max(4, Math.min(64, Math.round((BASE_FONT_SIZE * termZoom()) / paneZoom)));
    });

    // Apply zoom-driven font-size changes to the live terminal in place —
    // mirrors term.tsx:234-241. Only for LIVE updates (Ctrl+Wheel while the
    // shell is already open, or a meta push from elsewhere); the initial
    // mount's font size is seeded correctly before TermWrap is even
    // constructed (see the onMount IIFE below), so this effect's first
    // real-work firing is normally a no-op re-application of the same value.
    createEffect(() => {
        const fs = termFontSize();
        // Read unconditionally (not inside the `if`) so SolidJS subscribes
        // to this signal on the effect's very first run, when termWrap is
        // still undefined and `termWrap?.terminal && ...` would otherwise
        // short-circuit before wrapLoaded() is ever read — which silently
        // drops the subscription and reintroduces the exact bug this signal
        // was added to fix (reagentx P2 on #2522: setWrapLoaded(true) later
        // wouldn't re-trigger this effect at all, since it was never
        // actually subscribed to wrapLoaded in the first place).
        const loaded = wrapLoaded();
        if (termWrap?.terminal && loaded) {
            termWrap.terminal.options.fontSize = fs;
            termWrap.handleResize();
        }
    });

    // Loading-brain overlay, mirroring browser-view.tsx's pattern
    // (SPEC_AGENT_SHELL_ZOOM_SEED_RACE_2026-08-10.md) — masks the drawer
    // from mount until the zoom seed fetch resolves and the terminal is
    // constructed with its FINAL font size, so nothing ever paints at the
    // wrong size in the first place. `zoomSeeded()` is the source of truth;
    // these two signals exist only to hold BrainSpinner mounted for the CSS
    // fade-out duration after seeding finishes (its own contract: caller
    // owns unmounting after the transition ends).
    const [spinnerMounted, setSpinnerMounted] = createSignal(true);
    const [spinnerFading, setSpinnerFading] = createSignal(false);
    let spinnerFadeTimeout: ReturnType<typeof setTimeout> | null = null;
    createEffect(() => {
        if (!zoomSeeded()) {
            if (spinnerFadeTimeout) {
                clearTimeout(spinnerFadeTimeout);
                spinnerFadeTimeout = null;
            }
            setSpinnerFading(false);
            setSpinnerMounted(true);
            return;
        }
        if (!spinnerMounted()) return;
        // prefersReducedMotion: BrainSpinner shows/hides instantly (no CSS
        // transition) in that mode, so holding the node mounted for the
        // normal fade duration would just be a pointless delay — unmount now.
        if (atoms.prefersReducedMotionAtom()) {
            setSpinnerMounted(false);
            return;
        }
        setSpinnerFading(true);
        spinnerFadeTimeout = setTimeout(() => {
            spinnerFadeTimeout = null;
            setSpinnerFading(false);
            setSpinnerMounted(false);
        }, SHELL_LOADING_SPINNER_FADE_MS);
    });
    onCleanup(() => {
        if (spinnerFadeTimeout) clearTimeout(spinnerFadeTimeout);
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
            // Ctrl+Shift+Scroll is AppAllPanesZoomHandler's all-panes gesture
            // (app.tsx) — let it bubble there instead of zooming just this
            // sub-block. See SPEC_CTRL_SHIFT_SCROLL_ZOOM_ALL_PANES_2026_09_07.md.
            if (!ev.ctrlKey || ev.shiftKey) return;
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

        void attachShell(subBlockId());
    });

    // Re-attach when the parent's `term:shellsubblockid` meta repoints at a
    // DIFFERENT sub-block while this component stays mounted (the drawer
    // was never closed) — e.g. the previous shell's process exited and the
    // backend's "attach or create" logic created a replacement
    // (SPEC_TERM_EXIT_RESPAWN_LOOP_2026_09_15.md §11's respawn path).
    // `onMount`'s own `attachShell` call only ever runs once, for whatever
    // id was current AT MOUNT TIME — without this, the drawer stays bound
    // to the now-dead sub-block forever, rendering its last frame with no
    // live process behind it. Found live 2026-09-16 investigating a
    // corrupted vim pane
    // (docs/reports/REPORT_VIM_TERMINAL_PANE_HANG_INVESTIGATION_2026_09_16.md).
    let lastSeenExternalId = props.existingSubBlockId;
    createEffect(() => {
        const nextId = props.existingSubBlockId;
        if (nextId === lastSeenExternalId) return;
        lastSeenExternalId = nextId;
        // Undefined means the pointer was cleared, not repointed — nothing
        // to attach to. Equal to what's already attached means this is the
        // prop catching up to our OWN just-created id (onSubBlockCreated →
        // parent persists meta → re-render), not an external change;
        // attachShell already handled that id when we created it.
        if (!nextId || nextId === subBlockId()) return;
        termWrap?.dispose();
        termWrap = undefined;
        resizeObserver?.disconnect();
        resizeObserver = undefined;
        setWrapLoaded(false);
        setZoomSeeded(false);
        setError(null);
        void attachShell(nextId);
    });

    async function attachShell(candidateId: string | undefined) {
        const myGeneration = ++attachGeneration;
        const isStale = () => disposed || myGeneration !== attachGeneration;
        try {
            let id = candidateId;
            let isExistingBlock = false;
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
                //
                // `norespawn` covers the OTHER way this id can be dead: the
                // shell exited while the drawer was closed (the human
                // `exit`ed and reopened, or an agent sharing this shell
                // exited it), so the live exit subscription above — which
                // only exists while mounted — never saw it. Without the
                // flag, resync's default is to silently REVIVE a
                // STATUS_DONE controller in place, appending a fresh
                // startup banner to this block's append-only `term` file
                // on every reopen: the respawn-loop symptom
                // SPEC_TERM_EXIT_RESPAWN_LOOP_2026_09_15.md fixed on the
                // PtyShellCreate side (§7). With it, an exited shell
                // surfaces as an error and takes the same fall-through as
                // a vanished one — a genuinely fresh shell, clean
                // scrollback — and the dead block is deleted rather than
                // left orphaned. Codex P2 on PR #3253.
                try {
                    await RpcApi.ControllerResyncCommand(TabRpcClient, {
                        tabid: staticTabId(),
                        blockid: id,
                        forcerestart: false,
                        norespawn: true,
                    });
                    isExistingBlock = true;
                } catch (e) {
                    console.warn(
                        "AgentShellSubblock: existing sub-block is stale or already exited, creating a fresh one:",
                        e
                    );
                    // Delete the dead block ONLY on a positively-known
                    // CLEAN exit.
                    //
                    // ReAgent P0 (round 2) on PR #3253: deleting on any
                    // resync failure destroys the block's persisted `term`
                    // file — and for a shell that CRASHED while the drawer
                    // was closed, that file is the only record of what went
                    // wrong. The live-exit listener above already refuses to
                    // collapse on a non-zero exit for exactly this reason
                    // (spec §5); doing it here anyway would reproduce the
                    // same "not hidden — gone" data loss through the attach
                    // path instead of the collapse path.
                    //
                    // `undefined` (no exit observed — a genuinely vanished
                    // block, or a status we were never told) is NOT treated
                    // as clean: skipping the delete costs at worst a dead
                    // block lingering until the pane closes, while getting
                    // it wrong costs the user their diagnostics. A vanished
                    // block needs no delete anyway — it is already gone.
                    if (lastObservedExitCode === 0) {
                        void RpcApi.DeleteSubBlockCommand(TabRpcClient, { blockid: id }).catch(() => {});
                    } else {
                        console.warn(
                            `AgentShellSubblock: leaving sub-block ${id} in place (exit code ` +
                                `${lastObservedExitCode ?? "unknown"}) — its scrollback may be the ` +
                                `only record of why the shell ended`
                        );
                    }
                    id = undefined;
                }
                // Bail BEFORE touching any signal: a stale attempt (a newer
                // repoint already started while this resync was in flight)
                // must not even briefly reassign subBlockId back toward its
                // own dead id — it would corrupt subBlockAtom/termZoom/
                // agentLocked for the currently-live attachment even though
                // this attempt goes on to correctly bail before constructing
                // a TermWrap (ReAgent P1 on #3257).
                if (isStale()) return;
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
                if (isStale()) {
                    // A newer attach already started while this create call
                    // was in flight. This id was never recorded anywhere —
                    // setSubBlockId/onSubBlockCreated haven't run for it, so
                    // it never reaches term:shellsubblockid — meaning
                    // nothing else, including agent-view.tsx's own
                    // pane-close cleanup (which only knows about whatever
                    // that meta currently points at), can ever find it to
                    // clean up. Delete it ourselves or its backend PTY
                    // process leaks for the rest of the app's life
                    // (ReAgent P1 on #3257, round 2).
                    void RpcApi.DeleteSubBlockCommand(TabRpcClient, { blockid: id });
                    return;
                }
                // Set BEFORE onSubBlockCreated, not after: the parent can
                // (and in the real app does) synchronously feed this id
                // straight back as a new `existingSubBlockId` prop value —
                // that re-triggers the re-attach effect below, whose
                // "is this just our own echo?" guard compares the new prop
                // against subBlockId(). If subBlockId() were still stale at
                // that moment, the echo would look like an EXTERNAL repoint
                // and trigger a redundant, self-inflicted re-attach.
                setSubBlockId(id);
                props.onSubBlockCreated(id);
            } else {
                // Reuse-existing-id path: `id` was resolved via resync
                // above, not just-created, so there's no risk of racing our
                // own echo — but this component's OWN reactive state
                // (subBlockAtom/termZoom/agentLocked, and
                // handleCtrlWheel's zoom-write) all key off this signal,
                // not the local `id` variable. Leaving it stuck on the OLD
                // id after a reused-id re-attach silently breaks all of
                // them against the wrong block (ReAgent P1 on #3257).
                setSubBlockId(id);
            }

            // Seed the persisted zoom BEFORE constructing TermWrap, so the
            // very first paint already uses the correct font size instead
            // of the BASE_FONT_SIZE default followed by a visible
            // correction jerk — see
            // docs/specs/SPEC_AGENT_SHELL_ZOOM_SEED_RACE_2026-08-10.md. A
            // freshly created sub-block has no persisted term:zoom yet
            // (the already-correct default of 1.0 applies), so only the
            // reused-existing-block path needs to wait for anything.
            //
            // Deliberately does NOT call WOS.reloadWaveObject here: the
            // `subBlockAtom` memo above already triggered a fetch for
            // this exact oref as a side effect of being constructed
            // (WOS.getWaveObjectAtom → getWaveObjectValue eagerly fetches
            // on first read, and that memo runs synchronously at
            // component construction, before this async IIFE even
            // starts). Calling reloadWaveObject here would force a
            // SECOND, redundant GetObject round-trip for the same object
            // on every reused-sub-block drawer open (reagentx P1 on
            // #2522). Instead, just wait for that already-in-flight
            // fetch to settle, bounded by a timeout so a genuine network
            // failure (which can leave the loading atom stuck, per
            // wos.ts's own comment on GetObject rejections) can't hang
            // shell startup indefinitely — falls back to whatever
            // termFontSize() currently computes (default zoom) if it
            // times out; the live-update effect below corrects it later
            // if a subsequent fetch/push succeeds.
            if (isExistingBlock) {
                await waitForWaveObjectSettled(WOS.makeORef("block", id));
            }
            if (isStale()) return;
            setZoomSeeded(true);

            if (isStale() || !containerRef) return;
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
                        // Dropped, not queued: while the agent holds the
                        // lock, this shell's PTY is being actively driven
                        // by PtyShellInput — forwarding the human's
                        // keystrokes too would interleave both into the
                        // same input stream. See `agentLocked` above.
                        if (agentLocked()) return;
                        sendWSCommand({
                            wscommand: "blockinput",
                            blockid: id,
                            inputdata64: stringToBase64(data),
                        } as BlockInputWSCommand);
                    },
                }
            );
            await wrap.init();
            if (isStale()) {
                // Abandoned: either unmounted mid-init, or a NEWER attach
                // (another repoint) has since started and will commit its
                // own TermWrap. Dispose this one rather than leaving it
                // (and its PTY/WS subscription) leaked and orphaned, or —
                // worse — committing it to `termWrap` out from under the
                // newer, correct attachment (ReAgent P1 on #3257).
                wrap.dispose();
                return;
            }
            termWrap = wrap;
            setWrapLoaded(true);
            props.onTermReady?.((text: string) => {
                // Leading \r\n forces this line to start fresh regardless of
                // where the cursor was left by concurrently-arriving live PTY
                // output — see the onTermReady doc comment above.
                termWrap?.terminal.write(`\r\n${text}\r\n`);
            });

            // Reflow the PTY grid whenever the container is resized — drag-
            // resizing the details drawer (ResizableDetailsDrawer), the pane
            // itself, or the window. Without this the container can change
            // size (e.g. via the drawer's drag handle) with the terminal
            // never re-fitting to it. Mirrors term.tsx's rszObs pattern.
            // Plain DOM API, not a Solid primitive, so it's safe to set up
            // here post-await; teardown is registered synchronously below
            // via the `resizeObserver` closure var, not a second onCleanup.
            if (containerRef) {
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
            // A stale attempt's failure isn't this pane's problem any more
            // — a newer attach has already superseded it (or the component
            // unmounted), so surfacing its error would show a stale/wrong
            // message over whatever the newer attempt is doing.
            if (!isStale()) {
                setError(e instanceof Error ? e.message : String(e));
                // Clear the loading overlay even on failure — otherwise a
                // rejection before setZoomSeeded(true) (e.g. resync/create
                // both failing) leaves the BrainSpinner overlay covering
                // the error message forever.
                setZoomSeeded(true);
            }
        }
    }

    onCleanup(() => {
        disposed = true;
        termWrap?.dispose();
        resizeObserver?.disconnect();
        props.onTermDispose?.();
    });

    return (
        <div class="agent-shell-subblock" ref={containerRef}>
            {error() && <div class="agent-shell-subblock-error">Shell failed to start: {error()}</div>}
            <Show when={spinnerMounted()}>
                <div class="agent-shell-loading-overlay" classList={{ "is-fading": spinnerFading() }}>
                    <BrainSpinner />
                </div>
            </Show>
            <Show when={agentLocked()}>
                <div
                    class="agent-shell-agentlock-badge"
                    title="The agent is actively typing into this shell — your own input is paused until it stops."
                >
                    Agent is using this shell
                </div>
            </Show>
        </div>
    );
};
