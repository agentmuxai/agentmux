// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0
//
// WaveObjectStore — migrated to SolidJS signals.

import { waveEventSubscribe } from "@/app/store/wps";
import { getWebServerEndpoint } from "@/util/endpoints";
import { fetch } from "@/util/fetchutil";
import { type SignalAtom, fireAndForget } from "@/util/util";
import { createSignal, onCleanup } from "solid-js";
import { ObjectService } from "./services";
import { getApi } from "./global";

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

type WaveObjectDataItemType<T extends WaveObj> = {
    value: T;
    loading: boolean;
};

// Each cached WaveObject holds a SolidJS signal instead of a Jotai atom.
type WaveObjectValue<T extends WaveObj> = {
    pendingPromise: Promise<T> | null;
    // signal getter & setter pair
    getData: () => WaveObjectDataItemType<T>;
    setData: (v: WaveObjectDataItemType<T>) => void;
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

function isValidWaveObj(val: WaveObj): boolean {
    if (val == null) return false;
    return !(isBlank(val.otype) || isBlank(val.oid) || isBlankNum(val.version));
}

function makeORef(otype: string, oid: string): string {
    if (isBlank(otype) || isBlank(oid)) return null;
    return `${otype}:${oid}`;
}

function GetObject<T>(oref: string): Promise<T> {
    return callBackendService("object", "GetObject", [oref], true);
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

function wpsSubscribeToObject(oref: string): () => void {
    return waveEventSubscribe({
        eventType: "waveobj:update",
        scope: oref,
        handler: (event) => {
            updateWaveObject(event.data);
        },
    });
}

function callBackendService(service: string, method: string, args: any[], noUIContext?: boolean): Promise<any> {
    const startTs = Date.now();
    let uiContext: UIContext = null;
    if (!noUIContext && globalThis.window != null && (window as any).globalAtoms) {
        // During migration, globalAtoms may expose a signal accessor
        const ga = (window as any).globalAtoms as GlobalAtomsType;
        uiContext = typeof ga?.uiContext === "function" ? (ga.uiContext as any)() : null;
    }
    const waveCall: WebCallType = {
        service,
        method,
        args,
        uicontext: uiContext,
    };
    const methodName = `${service}.${method}`;
    const usp = new URLSearchParams();
    usp.set("service", service);
    usp.set("method", method);

    if (globalThis.window != null) {
        const authKey = getApi()?.getAuthKey?.();
        if (authKey) usp.set("authkey", authKey);
    }

    const url = getWebServerEndpoint() + "/wave/service?" + usp.toString();
    const fetchPromise = fetch(url, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(waveCall),
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
            if (respData.updates != null) updateWaveObjects(respData.updates);
            if (respData.error != null) throw new Error(`call ${methodName} error: ${respData.error}`);
            const durationStr = Date.now() - startTs + "ms";
            debugLogBackendCall(methodName, durationStr, args);
            return respData.data;
        });
}

// ---------------------------------------------------------------------------
// WaveObject cache — signals replace Jotai atoms
// ---------------------------------------------------------------------------

const waveObjectValueCache = new Map<string, WaveObjectValue<any>>();
const defaultHoldTime = 5000;

function createWaveValueObject<T extends WaveObj>(oref: string, shouldFetch: boolean): WaveObjectValue<T> {
    const [getData, setData] = createSignal<WaveObjectDataItemType<T>>({ value: null, loading: true });
    const wov: WaveObjectValue<T> = { pendingPromise: null, getData, setData, refCount: 0, holdTime: Date.now() + 5000 };
    if (!shouldFetch) return wov;

    const startTs = Date.now();
    const localPromise = GetObject<T>(oref);
    wov.pendingPromise = localPromise;
    localPromise.then((val) => {
        if (wov.pendingPromise !== localPromise) return;
        const [otype, oid] = splitORef(oref);
        if (val != null) {
            if ((val as any)["otype"] != otype) throw new Error("GetObject returned wrong type");
            if ((val as any)["oid"] != oid) throw new Error("GetObject returned wrong id");
        }
        wov.pendingPromise = null;
        wov.setData({ value: val, loading: false });
        console.log("WaveObj resolved", oref, Date.now() - startTs + "ms");
    });
    return wov;
}

function getWaveObjectValue<T extends WaveObj>(oref: string, createIfMissing = true): WaveObjectValue<T> {
    let wov = waveObjectValueCache.get(oref);
    if (wov === undefined && createIfMissing) {
        wov = createWaveValueObject(oref, true);
        waveObjectValueCache.set(oref, wov);
    }
    return wov;
}

function clearWaveObjectCache() {
    waveObjectValueCache.clear();
}

function reloadWaveObject<T extends WaveObj>(oref: string): Promise<T> {
    let wov = waveObjectValueCache.get(oref);
    if (wov === undefined) {
        wov = getWaveObjectValue<T>(oref, true);
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
 * Returns a SignalAtom for a WaveObject — callable as a getter, writable via ._set().
 * Calling the atom reads the current value (reactive in SolidJS components).
 * Setting via ._set() updates the local cache and optionally pushes to the server.
 */
function getWaveObjectAtom<T extends WaveObj>(oref: string): SignalAtom<T> {
    const wov = getWaveObjectValue<T>(oref);
    const atom = () => wov.getData().value;
    (atom as any)._set = (value: T | ((prev: T) => T)) => {
        const nextValue =
            typeof value === "function" ? (value as (prev: T) => T)(wov.getData().value) : value;
        setObjectValue(nextValue, false);
    };
    return atom as unknown as SignalAtom<T>;
}

/** Returns a signal accessor for the loading state. */
function getWaveObjectLoadingAtom(oref: string): () => boolean {
    const wov = getWaveObjectValue(oref);
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
function useWaveObjectValue<T extends WaveObj>(oref: string): [() => T, () => boolean] {
    const wov = getWaveObjectValue<T>(oref);
    wov.refCount++;
    onCleanup(() => {
        wov.refCount--;
    });
    return [
        () => wov.getData().value,
        () => wov.getData().loading,
    ];
}

function loadAndPinWaveObject<T extends WaveObj>(oref: string): Promise<T> {
    const wov = getWaveObjectValue<T>(oref);
    wov.refCount++;
    if (wov.pendingPromise == null) {
        const dataValue = wov.getData();
        return Promise.resolve(dataValue.value);
    }
    return wov.pendingPromise;
}

function updateWaveObject(update: WaveObjUpdate) {
    if (update == null) return;
    const oref = makeORef(update.otype, update.oid);
    const wov = getWaveObjectValue(oref);
    if (update.updatetype == "delete") {
        console.log("WaveObj deleted", oref);
        wov.setData({ value: null, loading: false });
    } else {
        if (!isValidWaveObj(update.obj)) {
            console.log("invalid wave object update", update);
            return;
        }
        const curValue = wov.getData();
        if (curValue.value != null && curValue.value.version >= update.obj.version) {
            return;
        }
        console.log("WaveObj updated", oref);
        wov.setData({ value: update.obj, loading: false });
    }
    wov.holdTime = Date.now() + defaultHoldTime;
}

function updateWaveObjects(vals: WaveObjUpdate[]) {
    for (const val of vals) {
        updateWaveObject(val);
    }
}

function cleanWaveObjectCache() {
    const now = Date.now();
    for (const [oref, wov] of waveObjectValueCache) {
        if (wov.refCount == 0 && wov.holdTime < now) {
            waveObjectValueCache.delete(oref);
        }
    }
}

/** Non-reactive read — returns the current value without tracking. */
function getObjectValue<T extends WaveObj>(oref: string): T {
    const wov = getWaveObjectValue<T>(oref);
    return wov.getData().value;
}

function setObjectValue<T extends WaveObj>(value: T, pushToServer?: boolean) {
    const oref = makeORef(value.otype, value.oid);
    const wov = getWaveObjectValue(oref, false);
    if (wov === undefined) return;
    wov.setData({ value, loading: false });
    if (pushToServer) {
        fireAndForget(() => ObjectService.UpdateObject(value, false));
    }
}

export {
    callBackendService,
    cleanWaveObjectCache,
    clearWaveObjectCache,
    getObjectValue,
    getWaveObjectAtom,
    getWaveObjectLoadingAtom,
    loadAndPinWaveObject,
    makeORef,
    reloadWaveObject,
    setObjectValue,
    splitORef,
    updateWaveObject,
    updateWaveObjects,
    useWaveObjectValue,
    wpsSubscribeToObject,
};
