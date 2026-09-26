// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// MuxObjectStore — migrated to SolidJS signals.

import { muxEventSubscribe } from "@/app/store/mps";
import { WpsEvent } from "@/app/store/mps-events";
import { getWebServerEndpoint, isWebServerEndpointSet } from "@/util/endpoints";
import { fetch } from "@/util/fetchutil";
import { retryTransient } from "@/util/transient-network";
import { type SignalAtom, fireAndForget } from "@/util/util";
import { batch, createSignal, getOwner, onCleanup } from "solid-js";
import { ObjectService } from "./services";
import { getApi } from "./app-api";

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

type MuxObjectDataItemType<T extends MuxObj> = {
    value: T;
    loading: boolean;
};

// Each cached MuxObject holds a SolidJS signal instead of a Jotai atom.
type MuxObjectValue<T extends MuxObj> = {
    pendingPromise: Promise<T> | null;
    // The last fetch failed without a definitive answer (network error, no
    // endpoint yet). The value is unknown, not null: the next explicit load
    // re-fetches into this same entry. See #3868.
    fetchFailed: boolean;
    // signal getter & setter pair
    getData: () => MuxObjectDataItemType<T>;
    setData: (v: MuxObjectDataItemType<T>) => void;
    refCount: number;
    holdTime: number;
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function splitORef(oref: string): [string, string] {
    const parts = oref.split(":");
    if (parts.length != 2) {
        throw new Error("invalid oref");
    }
    return [parts[0], parts[1]];
}

function isBlank(str: string): boolean {
    return str == null || str == "";
}

function isBlankNum(num: number): boolean {
    return num == null || isNaN(num) || num == 0;
}

function isValidMuxObj(val: MuxObj): boolean {
    if (val == null) return false;
    return !(isBlank(val.otype) || isBlank(val.oid) || isBlankNum(val.version));
}

function makeORef(otype: string, oid: string): string {
    if (isBlank(otype) || isBlank(oid)) return null;
    return `${otype}:${oid}`;
}

/** Rejection for a backend call made before the CEF bootstrap has set the
 *  backend address. Nothing is sent; callers may retry once it is set. */
class BackendEndpointNotSetError extends Error {
    constructor(methodName: string) {
        super(`${methodName}: backend endpoint not set yet`);
        this.name = "BackendEndpointNotSetError";
    }
}

// A read, so safe to retry when the request never got a response (#3868).
function GetObject<T>(oref: string): Promise<T> {
    return retryTransient(() => callBackendService("object", "GetObject", [oref], true));
}

function debugLogBackendCall(methodName: string, durationStr: string, args: any[]) {
    durationStr = "| " + durationStr;
    if (methodName == "object.UpdateObject" && args.length > 0) {
        console.log("[service] object.UpdateObject", args[0].otype, args[0].oid, durationStr, args[0]);
        return;
    }
    if (methodName == "object.GetObject" && args.length > 0) {
        console.log("[service] object.GetObject", args[0], durationStr);
        return;
    }
    if (methodName == "file.StatFile" && args.length >= 2) {
        console.log("[service] file.StatFile", args[1], durationStr);
        return;
    }
    console.log("[service]", methodName, durationStr);
}

function mpsSubscribeToObject(oref: string): () => void {
    return muxEventSubscribe({
        eventType: WpsEvent.MuxObjUpdate,
        scope: oref,
        handler: (event) => {
            updateMuxObject(event.data);
        },
    });
}

function callBackendService(service: string, method: string, args: any[], noUIContext?: boolean): Promise<any> {
    // Module-init reads (window-identity memos, activity polls) can run before
    // the bootstrap sets the endpoint. Don't send them to "http://null".
    if (!isWebServerEndpointSet()) {
        return Promise.reject(new BackendEndpointNotSetError(`${service}.${method}`));
    }
    const startTs = Date.now();
    let uiContext: UIContext = null;
    if (!noUIContext && globalThis.window != null && window.globalAtoms) {
        // During migration, globalAtoms may expose a signal accessor
        const ga = window.globalAtoms as GlobalAtomsType;
        uiContext = typeof ga?.uiContext === "function" ? (ga.uiContext as any)() : null;
    }
    const muxCall: WebCallType = {
        service,
        method,
        args,
        uicontext: uiContext,
    };
    const methodName = `${service}.${method}`;
    const usp = new URLSearchParams();
    usp.set("service", service);
    usp.set("method", method);

    // Audit C3: the `?authkey=` query-string fallback was removed on
    // every HTTP route. Use the X-AuthKey header instead.
    const headers: Record<string, string> = { "Content-Type": "application/json" };
    if (globalThis.window != null) {
        const authKey = getApi()?.getAuthKey?.();
        if (authKey) headers["X-AuthKey"] = authKey;
    }

    const url = getWebServerEndpoint() + "/agentmux/service?" + usp.toString();
    const fetchPromise = fetch(url, {
        method: "POST",
        headers,
        body: JSON.stringify(muxCall),
    });
    return fetchPromise
        .then((resp) => {
            if (!resp.ok) {
                throw new Error(`call ${methodName} failed: ${resp.status} ${resp.statusText}`);
            }
            return resp.json();
        })
        .then((respData: WebReturnType) => {
            if (respData == null) return null;
            if (respData.updates != null) updateMuxObjects(respData.updates);
            if (respData.error != null) throw new Error(`call ${methodName} error: ${respData.error}`);
            const durationStr = Date.now() - startTs + "ms";
            debugLogBackendCall(methodName, durationStr, args);
            return respData.data;
        });
}

// ---------------------------------------------------------------------------
// MuxObject cache — signals replace Jotai atoms
// ---------------------------------------------------------------------------

const muxObjectValueCache = new Map<string, MuxObjectValue<any>>();
const defaultHoldTime = 5000;

function createMuxValueObject<T extends MuxObj>(oref: string, shouldFetch: boolean): MuxObjectValue<T> {
    const [getData, setData] = createSignal<MuxObjectDataItemType<T>>({ value: null, loading: true });
    const wov: MuxObjectValue<T> = {
        pendingPromise: null,
        fetchFailed: false,
        getData,
        setData,
        refCount: 0,
        holdTime: Date.now() + 5000,
    };
    if (!shouldFetch) return wov;
    startFetch(wov, oref);
    return wov;
}

/** Fetch `oref` into `wov` — the same entry, so existing subscribers of its
 *  signal see the result. Returns the fetch promise (rejects on failure). */
function startFetch<T extends MuxObj>(wov: MuxObjectValue<T>, oref: string): Promise<T> {
    const startTs = Date.now();
    const localPromise = GetObject<T>(oref);
    wov.pendingPromise = localPromise;
    wov.fetchFailed = false;
    localPromise.then((val) => {
        if (wov.pendingPromise !== localPromise) return;
        const [otype, oid] = splitORef(oref);
        if (val != null) {
            if ((val as any)["otype"] != otype) throw new Error("GetObject returned wrong type");
            if ((val as any)["oid"] != oid) throw new Error("GetObject returned wrong id");
        }
        wov.pendingPromise = null;
        wov.setData({ value: val, loading: false });
        console.log("MuxObj resolved", oref, Date.now() - startTs + "ms");
    }).catch((err) => {
        if (wov.pendingPromise !== localPromise) return;
        wov.pendingPromise = null;
        // A backend "not found" rejection (get_object_by_oref's
        // `Err(format!("not found: {}", oref_str))`, object_helpers.rs) is a
        // DEFINITIVE answer, not a transient failure — resolve loading so
        // callers checking getMuxObjectLoadingAtom (e.g. swarm-model.ts's
        // hasRenderableBlock) can actually distinguish "confirmed absent"
        // from "still fetching" instead of this oref staying stuck at
        // loading:true forever (reagentx P1 on #2438, second pass).
        if (err instanceof Error && err.message.includes("not found: " + oref)) {
            wov.setData({ value: null, loading: false });
            return;
        }
        // Any OTHER error (network failure after retries, endpoint not set
        // yet) says nothing about the object. Leave loading as-is and mark
        // the entry so loadAndPinMuxObject re-fetches instead of returning
        // this null as if it were the answer — that null is what aborted
        // initMux with "did not load" in #3868.
        wov.fetchFailed = true;
    });
    return localPromise;
}

function getMuxObjectValue<T extends MuxObj>(oref: string, createIfMissing = true): MuxObjectValue<T> {
    let wov = muxObjectValueCache.get(oref);
    if (wov === undefined && createIfMissing) {
        wov = createMuxValueObject(oref, true);
        muxObjectValueCache.set(oref, wov);
    }
    return wov;
}

function reloadMuxObject<T extends MuxObj>(oref: string): Promise<T> {
    let wov = muxObjectValueCache.get(oref);
    if (wov === undefined) {
        wov = getMuxObjectValue<T>(oref, true);
        return wov.pendingPromise!;
    }
    const prtn = GetObject<T>(oref);
    prtn.then((val) => {
        wov!.setData({ value: val, loading: false });
    });
    return prtn;
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/**
 * Returns a SignalAtom for a MuxObject — callable as a getter, writable via ._set().
 * Calling the atom reads the current value (reactive in SolidJS components).
 * Setting via ._set() updates the local cache and optionally pushes to the server.
 */
function getMuxObjectAtom<T extends MuxObj>(oref: string): SignalAtom<T> {
    // Resolve the cache entry on every read rather than capturing it once.
    //
    // `cleanMuxObjectCache` (below) evicts any entry with `refCount === 0` once
    // its holdTime lapses, and this function — unlike `useMuxObjectValue` —
    // deliberately takes no refCount, because it has no lifecycle hook to give
    // one back. So an atom that captured its `wov` kept reading a signal that
    // nothing writes to after the first eviction: `updateMuxObject` re-resolves
    // the oref, gets a NEW entry, and writes there. The object looked updated
    // from the store's side (it even logs "MuxObj updated") while every
    // component holding the stale closure was frozen on the pre-eviction value,
    // permanently and silently.
    //
    // This bit surfaced as "Ctrl+Wheel zoom does nothing in the agent Shell
    // drawer" — its sub-block is headless, so nothing else holds a refCount on
    // it, and it was always evicted within a GC tick or two. A top-level
    // terminal pane's block is held by layout code through `useMuxObjectValue`,
    // which is why the same gesture kept working there. Reopening the drawer
    // "fixed" it only because that rebuilt the atom against the fresh entry.
    //
    // Late-binding keeps the read reactive (getData() still tracks the current
    // signal) and lets an evicted entry transparently re-fetch, at the cost of
    // one Map lookup per read.
    // Pin the entry for the caller's reactive lifetime when there is one.
    //
    // Late-binding the read (below) is not enough on its own: a SolidJS memo or
    // effect only re-runs when a signal IT IS SUBSCRIBED TO changes. Once the
    // entry is evicted and `updateMuxObject` re-creates it, the new entry has a
    // BRAND NEW signal that no existing dependent is subscribed to, so nothing
    // ever re-triggers them — they never re-read, and late-binding never gets a
    // chance to help. Keeping the entry alive for as long as a reactive owner
    // depends on it preserves signal identity, which is what reactivity needs.
    //
    // Guarded on `getOwner()`: outside a reactive root there is no cleanup hook
    // to hand the refCount back, and an unconditional increment would pin every
    // oref forever — turning a staleness bug into an unbounded cache leak.
    const owner = getOwner();
    if (owner != null) {
        const pinned = getMuxObjectValue<T>(oref);
        pinned.refCount++;
        onCleanup(() => {
            pinned.refCount--;
        });
    }
    const atom = () => getMuxObjectValue<T>(oref).getData().value;
    (atom as any)._set = (value: T | ((prev: T) => T)) => {
        const nextValue =
            typeof value === "function"
                ? (value as (prev: T) => T)(getMuxObjectValue<T>(oref).getData().value)
                : value;
        setObjectValue(nextValue, false);
    };
    return atom as unknown as SignalAtom<T>;
}

/** Returns a signal accessor for the loading state. */
function getMuxObjectLoadingAtom(oref: string): () => boolean {
    const wov = getMuxObjectValue(oref);
    return () => {
        const d = wov.getData();
        return d.loading ? null : d.loading;
    };
}

/**
 * SolidJS hook: returns [valueAccessor, loadingAccessor].
 * Must be called inside a component or reactive root.
 * Manages refCount and cleanup automatically.
 */
function useMuxObjectValue<T extends MuxObj>(oref: string): [() => T, () => boolean] {
    const wov = getMuxObjectValue<T>(oref);
    wov.refCount++;
    onCleanup(() => {
        wov.refCount--;
    });
    return [
        () => wov.getData().value,
        () => wov.getData().loading,
    ];
}

function loadAndPinMuxObject<T extends MuxObj>(oref: string): Promise<T> {
    const wov = getMuxObjectValue<T>(oref);
    wov.refCount++;
    if (wov.pendingPromise == null) {
        if (wov.fetchFailed) return startFetch(wov, oref);
        const dataValue = wov.getData();
        return Promise.resolve(dataValue.value);
    }
    return wov.pendingPromise;
}

function updateMuxObject(update: MuxObjUpdate) {
    if (update == null) return;
    const oref = makeORef(update.otype, update.oid);
    const wov = getMuxObjectValue(oref);
    if (update.updatetype == "delete") {
        console.log("MuxObj deleted", oref);
        wov.setData({ value: null, loading: false });
    } else {
        if (!isValidMuxObj(update.obj)) {
            console.log("invalid wave object update", update);
            return;
        }
        const curValue = wov.getData();
        if (curValue.value != null && curValue.value.version >= update.obj.version) {
            return;
        }
        console.log("MuxObj updated", oref);
        wov.setData({ value: update.obj, loading: false });
    }
    wov.fetchFailed = false;
    wov.holdTime = Date.now() + defaultHoldTime;
}

function updateMuxObjects(vals: MuxObjUpdate[]) {
    // batch() so a single RPC response carrying multiple related updates
    // (e.g. CloseTab's [delete Tab, update Workspace] pair) applies as one
    // atomic reactive flush. Without it, each setData() below propagates
    // synchronously and independently: the deleted tab's own signal went
    // null (blanking its still-mounted <Tab>'s name) a full reactive
    // update BEFORE the workspace update removed it from the tab strip's
    // <For> list, so the tab visibly flashed blank in place before
    // disappearing. See docs/specs/SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH_2026_08_25.md §6.
    batch(() => {
        for (const val of vals) {
            updateMuxObject(val);
        }
    });
}

function cleanMuxObjectCache() {
    const now = Date.now();
    for (const [oref, wov] of muxObjectValueCache) {
        if (wov.refCount == 0 && wov.holdTime < now) {
            muxObjectValueCache.delete(oref);
        }
    }
}

// Periodically clean up stale MuxObject cache entries
setInterval(cleanMuxObjectCache, 30000);

/**
 * Diagnostic-only snapshot of `muxObjectValueCache` — added while
 * investigating a 2026-09-20 latency/memory report. `pinnedEntries` is the
 * count with `refCount > 0`: these survive `cleanMuxObjectCache` regardless
 * of `holdTime`, since #3454 made `getMuxObjectAtom` pin an entry for its
 * calling reactive owner's entire lifetime (see that function's own doc
 * comment). A `pinnedEntries` count that climbs without bound and never
 * comes back down as panes go dormant/close would point at exactly that
 * mechanism; see `frontend/app/diag/atom-cache-diagnostic.ts`, which reports
 * this alongside `block-atom-cache.ts`'s and the block registry's own stats.
 */
function getMuxObjectCacheStats(): { totalEntries: number; pinnedEntries: number; totalRefCount: number } {
    let pinnedEntries = 0;
    let totalRefCount = 0;
    for (const wov of muxObjectValueCache.values()) {
        if (wov.refCount > 0) pinnedEntries++;
        totalRefCount += wov.refCount;
    }
    return { totalEntries: muxObjectValueCache.size, pinnedEntries, totalRefCount };
}

/** Non-reactive read — returns the current value without tracking. */
function getObjectValue<T extends MuxObj>(oref: string): T {
    const wov = getMuxObjectValue<T>(oref);
    return wov.getData().value;
}

function setObjectValue<T extends MuxObj>(value: T, pushToServer?: boolean) {
    const oref = makeORef(value.otype, value.oid);
    const wov = getMuxObjectValue(oref, false);
    if (wov === undefined) return;
    wov.setData({ value, loading: false });
    if (pushToServer) {
        fireAndForget(() => ObjectService.UpdateObject(value, false));
    }
}

export {
    callBackendService,
    getObjectValue,
    getMuxObjectAtom,
    getMuxObjectCacheStats,
    getMuxObjectLoadingAtom,
    loadAndPinMuxObject,
    makeORef,
    reloadMuxObject,
    setObjectValue,
    updateMuxObject,
    updateMuxObjects,
    useMuxObjectValue,
    mpsSubscribeToObject,
};
