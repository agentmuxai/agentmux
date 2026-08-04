// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Singleton-modal coordination layer — PR 3 of
 * docs/specs/archive/SPEC_BUNDLE_MANAGEMENT_2026_05_22.md (§3 "Singleton — one
 * manager across the whole app").
 *
 * Problem this solves
 * -------------------
 * The unified modal has `pane` / `tab` / `window` scopes — each inerts a
 * DOM region within ONE renderer. A *singleton* modal needs something the
 * DOM-inert scopes cannot give: exactly one instance of a modal of kind
 * `K` across EVERY window of the AgentMux process, where each window is
 * its own CEF renderer. That is cross-window coordination, not DOM inert.
 *
 * Chosen mechanism — WPS-backed shared store (no Rust change)
 * -----------------------------------------------------------
 * Every AgentMux process runs exactly one `agentmux-srv`; every window's
 * renderer holds a WebSocket to it. The srv's WPS broker
 * (`agentmux-srv/src/backend/wps.rs`) is therefore a *process-wide* event
 * bus already shared by all windows. We ride it:
 *
 *   1. **Registry + broadcast.** A claim is a WPS event of type
 *      `EVENT_SINGLETON_CLAIM`, scoped `singleton:<kind>`, published with
 *      `persist: 1`. `persist: 1` means the broker keeps the *latest*
 *      claim and replays it to any window that subscribes later — so the
 *      "which window holds kind K" registry is simply the most-recent
 *      persisted event. No bespoke registry RPC, no Rust change.
 *   2. **Cross-window delivery.** `waveEventSubscribe` in every window
 *      receives every claim/release; non-holders react (render a banner).
 *   3. **Focus action.** The banner's button calls the existing
 *      `getApi().focusWindow(label)` (same primitive InstancePanel uses).
 *   4. **Crash release.** A holder that exits cannot publish its own
 *      release. The launcher already detects window exit and emits
 *      `window_closed` / `window_instance_released`; those events reach
 *      every *live* window via the launcher-event bridge. The first live
 *      window to observe its current holder's label close publishes a
 *      release on the dead holder's behalf — so no window is ever
 *      stranded pointing at a dead holder.
 *
 * Tradeoff vs. riding the launcher window registry directly: the claim
 * state is not durably persisted in the launcher's Rust-side registry,
 * so it lives only as long as the srv process. That is acceptable —
 * the singleton is meaningful only *within* one running AgentMux
 * process; if the process dies the claim is moot anyway. The win is
 * zero Rust/launcher changes: the whole layer is renderer-side over
 * infrastructure that already exists and is already battle-tested
 * (the same WPS persist/replay path that backs `tool_chunk` streaming).
 *
 * Publish transport
 * -----------------
 * The frontend `EventPublishCommand` RPC has no srv handler (the srv
 * only registers `eventsub` / `eventunsub` / `eventreadhistory`). The
 * auth-gated HTTP endpoint `POST /agentmux/wps/publish` *does* forward
 * arbitrary `{event, scopes, persist, data}` to the broker — that is the
 * publish path `agentmux-bashwrap` uses, and the one we use here.
 */

import { createSignal, type Accessor } from "solid-js";

import { getApi, openWindowEntriesAtom } from "@/store/global";
import { getWebServerEndpoint } from "@/util/endpoints";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { waveEventSubscribe } from "@/app/store/wps";
import { subscribeLauncherEvent } from "@/util/launcher-events";

/** WPS event name carrying singleton claim/release broadcasts. */
const EVENT_SINGLETON_CLAIM = "singleton:claim";

/**
 * Kind of singleton modal. Open enum (string) so future singletons add a
 * value without touching this module — each consumer defines its own
 * kind constant (e.g. `SINGLETON_KIND_BUNDLE_MANAGER` in
 * `bundle-manager-modal.tsx`).
 */
export type SingletonKind = string;

/**
 * Payload of an `EVENT_SINGLETON_CLAIM` WPS event. `data` on the wire.
 *  - `holder` = window label currently holding the singleton, or `null`
 *    when released.
 *  - `epoch` = monotonically-increasing-per-window claim counter; used
 *    only to break ties when two windows race a claim (higher epoch from
 *    the same logical moment is irrelevant — see `applyClaim`).
 */
interface ClaimPayload {
    kind: SingletonKind;
    holder: string | null;
    epoch: number;
}

/** Scope string for a kind — keeps each kind's claims isolated. */
function scopeFor(kind: SingletonKind): string {
    return `singleton:${kind}`;
}

// ── Per-kind reactive state ────────────────────────────────────────────

interface KindState {
    /** Reactive accessor → holder window label, or null. */
    holder: Accessor<string | null>;
    setHolder: (v: string | null) => void;
    /** Local epoch counter for claims this window publishes. */
    epoch: number;
    /** True once a WPS subscription + history-replay has been wired. */
    wired: boolean;
}

const kinds = new Map<SingletonKind, KindState>();

function kindState(kind: SingletonKind): KindState {
    let st = kinds.get(kind);
    if (!st) {
        const [holder, setHolder] = createSignal<string | null>(null);
        st = { holder, setHolder, epoch: 0, wired: false };
        kinds.set(kind, st);
        ensureWired(kind, st);
    }
    return st;
}

// ── This window's label ────────────────────────────────────────────────

let myLabel: string | null = null;
let myLabelPromise: Promise<string> | null = null;

/**
 * Resolve (and cache) this window's launcher label. `getWindowLabel`
 * reads it synchronously from the URL in practice, but the API is async
 * — so callers that need it before resolution get the cached value or a
 * pending promise.
 */
function resolveMyLabel(): Promise<string> {
    if (myLabel != null) return Promise.resolve(myLabel);
    if (!myLabelPromise) {
        const api = getApi();
        // `window.api` may not exist yet at very early boot. Calling
        // through a missing bridge throws synchronously — which would
        // crash app init. Bail without caching so a later call (the
        // post-init kick-off in `startSingletonCrashRelease`) retries.
        if (api == null) return Promise.resolve("main");
        myLabelPromise = api
            .getWindowLabel()
            .then((l) => {
                myLabel = l;
                return l;
            })
            .catch(() => {
                // Fall back to "main" — matches the cef-api default.
                myLabel = "main";
                return "main";
            });
    }
    return myLabelPromise;
}

/** Synchronous best-effort label. Null until `resolveMyLabel` settles. */
function myLabelSync(): string | null {
    return myLabel;
}

// ── Publish ────────────────────────────────────────────────────────────

/**
 * Publish a claim/release to the process-wide WPS broker via the
 * auth-gated HTTP endpoint. `persist: 1` so a window that subscribes
 * later replays the current holder. Fire-and-forget — the optimistic
 * local update already happened; a failed publish self-heals on the
 * next claim or on history-replay.
 */
async function publishClaim(payload: ClaimPayload): Promise<void> {
    const key = getApi()?.getAuthKey?.();
    const headers: Record<string, string> = { "Content-Type": "application/json" };
    if (key) headers["X-AuthKey"] = key;
    try {
        const resp = await fetch(getWebServerEndpoint() + "/agentmux/wps/publish", {
            method: "POST",
            headers,
            body: JSON.stringify({
                event: EVENT_SINGLETON_CLAIM,
                scopes: [scopeFor(payload.kind)],
                // persist: 1 → broker keeps only the latest claim and
                // replays it to late subscribers. Exactly the registry
                // semantics we want: "the current holder".
                persist: 1,
                data: payload,
            }),
        });
        if (!resp.ok) {
            console.error("[singleton] publishClaim failed: HTTP", resp.status);
        }
    } catch (e) {
        console.error("[singleton] publishClaim error:", e);
    }
}

// ── Apply an incoming claim ─────────────────────────────────────────────

/**
 * Apply a claim payload to local reactive state. Idempotent — applying
 * the same holder twice is a no-op (SolidJS signal equality short-
 * circuits the re-render). A `null` holder clears the registry.
 */
function applyClaim(payload: ClaimPayload): void {
    const st = kindState(payload.kind);
    if (st.holder() === payload.holder) return;
    st.setHolder(payload.holder);
}

function parsePayload(raw: unknown): ClaimPayload | null {
    if (raw == null || typeof raw !== "object") return null;
    const r = raw as Record<string, unknown>;
    if (typeof r.kind !== "string") return null;
    const holder = r.holder;
    if (holder !== null && typeof holder !== "string") return null;
    const epoch = typeof r.epoch === "number" ? r.epoch : 0;
    return { kind: r.kind, holder: holder as string | null, epoch };
}

// ── Wiring: WPS subscription + history replay + crash release ───────────

function ensureWired(kind: SingletonKind, st: KindState): void {
    if (st.wired) return;
    st.wired = true;

    // Resolve this window's label here too. A kind is first wired when a
    // component reads `singletonHolder`/`acquireSingleton` — which only
    // happens once the app shell has rendered, so `window.api` is ready.
    // This is the retry path: if the `startSingletonCrashRelease`
    // kick-off ran before the API bridge existed (and bailed), this
    // recovers the label so `acquireSingleton` is not a permanent no-op.
    void resolveMyLabel();

    // 1. Subscribe to live claim/release broadcasts for this kind.
    waveEventSubscribe({
        eventType: EVENT_SINGLETON_CLAIM,
        scope: scopeFor(kind),
        handler: (event: WaveEvent) => {
            const payload = parsePayload(event.data);
            if (payload && payload.kind === kind) applyClaim(payload);
        },
    });

    // 2. Replay the persisted latest claim so a window that opens AFTER
    //    a holder claimed still learns the current holder. Without this,
    //    a window started late would think the singleton is free.
    void RpcApi.EventReadHistoryCommand(TabRpcClient, {
        event: EVENT_SINGLETON_CLAIM,
        scope: scopeFor(kind),
        maxitems: 1,
    })
        .then((events) => {
            if (!events || events.length === 0) return;
            const payload = parsePayload(events[events.length - 1].data);
            if (payload && payload.kind === kind) applyClaim(payload);
        })
        .catch((e) => console.error("[singleton] history replay failed:", e));
}

/**
 * Crash-release wiring. Subscribes ONCE (process-wide, not per-kind) to
 * the launcher window-exit signal. When the window currently holding ANY
 * kind exits, this live window publishes a release on its behalf.
 *
 * Convergence: every live window runs this handler, so several may
 * publish the same release concurrently. That is safe — releases are
 * idempotent (a `null` holder applied twice is a no-op) and `persist: 1`
 * means the broker just keeps the last one. No coordination needed.
 *
 * Idempotent: guarded by `crashReleaseWired`.
 */
let crashReleaseWired = false;

export function startSingletonCrashRelease(): void {
    if (crashReleaseWired) return;
    crashReleaseWired = true;

    // Kick off this window's label resolution here. `app-init` calls
    // this once `window.api` exists, so resolving from here (not at
    // module-eval) keeps `getApi()` safe; `myLabelSync()` is then ready
    // well before any user-triggered acquire.
    void resolveMyLabel();

    subscribeLauncherEvent((evt) => {
        const isExit =
            evt.event === "window_closed" || evt.event === "window_instance_released";
        if (!isExit) return;
        const evtLabel = (evt as unknown as { label?: unknown }).label;
        const closedLabel = typeof evtLabel === "string" ? evtLabel : null;
        if (!closedLabel) return;

        // For every kind whose holder is the closed window, publish a
        // release. Skip the kinds this window itself holds — those get a
        // normal `releaseSingleton` on unmount; and if THIS window is the
        // one closing, it won't be running this handler much longer
        // anyway (the live siblings cover it).
        for (const [kind, st] of kinds) {
            if (st.holder() === closedLabel) {
                console.log(
                    "[singleton] crash-release:",
                    kind,
                    "holder",
                    closedLabel,
                    "exited",
                );
                // Optimistic local clear so this window's banner updates
                // immediately even before the broadcast round-trips.
                st.setHolder(null);
                void publishClaim({ kind, holder: null, epoch: ++st.epoch });
            }
        }
    });
}

// ── Public API ─────────────────────────────────────────────────────────

/**
 * Reactive accessor → the window label currently holding the singleton
 * modal of `kind`, or `null` if no window holds it.
 *
 * Use in a SolidJS component / memo: it tracks as a dependency, so a
 * banner re-renders when the holder changes (claim, release, or crash).
 */
export function singletonHolder(kind: SingletonKind): Accessor<string | null> {
    return kindState(kind).holder;
}

/**
 * True if `label` is a currently-open window per the launcher's window
 * registry. Used to reject a stale claim left behind by a crashed
 * holder — one that exited with no live sibling to publish its
 * crash-release.
 */
function isWindowLive(label: string): boolean {
    const entries = openWindowEntriesAtom();
    // Empty registry ⇒ not yet populated. Don't judge a holder dead on
    // incomplete data — that would wrongly steal a live singleton.
    if (entries.length === 0) return true;
    return entries.some((e) => e.label === label);
}

/**
 * Attempt to acquire the app-wide singleton for `kind` for THIS window.
 *
 * Returns `true` only when this window definitely holds it now — it was
 * free, held by a crashed/stale window, or already ours. Returns
 * `false` if another LIVE window holds it, or if this window's label
 * hasn't resolved yet; the caller then shows the focus banner instead
 * of opening the modal.
 *
 * Two windows racing a genuinely-free singleton is possible but rare
 * (both must call within the WPS round-trip); last-write-wins and both
 * converge. For the bundle manager the trigger is a deliberate
 * hamburger click, so the race is not a practical concern.
 */
export function acquireSingleton(kind: SingletonKind): boolean {
    const st = kindState(kind);
    const me = myLabelSync();
    // `true` must mean "holds it NOW" — only honourable synchronously,
    // so a window whose label hasn't resolved cannot acquire. In
    // practice the label is URL-derived and resolves at module load,
    // well before any user-triggered acquire; this guards a boot-instant
    // call only.
    if (me == null) return false;

    const current = st.holder();
    // Blocked only by a holder that is BOTH another window AND still
    // live. A holder that crashed with no live sibling to emit
    // crash-release leaves a stale persisted claim; a non-live holder is
    // treated as free so the singleton can never get permanently stuck.
    if (current != null && current !== me && isWindowLive(current)) {
        return false;
    }

    st.setHolder(me);
    void publishClaim({ kind, holder: me, epoch: ++st.epoch });
    return true;
}

/**
 * Release the singleton for `kind` IF this window holds it. No-op if a
 * different window holds it (or none does) — a window can only release
 * its own claim.
 */
export function releaseSingleton(kind: SingletonKind): void {
    const st = kindState(kind);
    const me = myLabelSync();
    // Only the holder may release. If we don't know our label yet we
    // also can't be the holder (acquire would have waited), so bail.
    if (me == null || st.holder() !== me) return;
    st.setHolder(null);
    void publishClaim({ kind, holder: null, epoch: ++st.epoch });
}

/**
 * True if THIS window currently holds the singleton for `kind`.
 * Reactive — safe to call inside a memo.
 */
export function holdsSingleton(kind: SingletonKind): boolean {
    // Guard on a resolved label — with `me` null (early boot) and no
    // holder, `null === null` would wrongly report this window as the
    // holder. Nobody holds it ⇒ false.
    const me = myLabelSync();
    return me != null && kindState(kind).holder() === me;
}

// ── Test-only helpers ──────────────────────────────────────────────────

/** Test-only: reset all kind state + wiring guards. Never call in prod. */
export function __resetSingletonForTests(): void {
    kinds.clear();
    crashReleaseWired = false;
    myLabel = null;
    myLabelPromise = null;
}

/** Test-only: force this window's label (bypasses the async resolve). */
export function __setMyLabelForTests(label: string | null): void {
    myLabel = label;
    myLabelPromise = label == null ? null : Promise.resolve(label);
}

/** Test-only: directly apply an inbound claim payload. */
export function __applyClaimForTests(payload: ClaimPayload): void {
    applyClaim(payload);
}
