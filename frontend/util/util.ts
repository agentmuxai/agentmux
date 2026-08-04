// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0s

import base64 from "base64-js";
import clsx, { type ClassValue } from "clsx";
import { createSignal } from "solid-js";
import { twMerge } from "tailwind-merge";
import { throttle } from "throttle-debounce";
const prevValueCache = new WeakMap<any, any>(); // stores a previous value for a deep equal comparison (used with the deepCompareReturnPrev function)

function isBlank(str: string): boolean {
    return str == null || str == "";
}

function stringToBase64(input: string): string {
    const stringBytes = new TextEncoder().encode(input);
    return base64.fromByteArray(stringBytes);
}

function base64ToArray(b64: string): Uint8Array<ArrayBuffer> {
    const rawStr = atob(b64);
    const rtnArr = new Uint8Array(new ArrayBuffer(rawStr.length));
    for (let i = 0; i < rawStr.length; i++) {
        rtnArr[i] = rawStr.charCodeAt(i);
    }
    return rtnArr;
}

function boundNumber(num: number, min: number, max: number): number {
    if (num == null || typeof num != "number" || isNaN(num)) {
        return null;
    }
    return Math.min(Math.max(num, min), max);
}

// key must be a suitable weakmap key.  pass the new value
// it will return the prevValue (for object equality) if the new value is deep equal to the prev value
function deepCompareReturnPrev(key: any, newValue: any): any {
    if (key == null) {
        return newValue;
    }
    const previousValue = prevValueCache.get(key);
    if (previousValue !== undefined && JSON.stringify(newValue) === JSON.stringify(previousValue)) {
        return previousValue;
    }
    prevValueCache.set(key, newValue);
    return newValue;
}

function makeIconClass(icon: string, fw: boolean, opts?: { spin?: boolean; defaultIcon?: string }): string {
    if (isBlank(icon)) {
        if (opts?.defaultIcon != null) {
            return makeIconClass(opts.defaultIcon, fw, { spin: opts?.spin });
        }
        return null;
    }
    if (icon.match(/^(solid@)?[a-z0-9-]+$/)) {
        // strip off "solid@" prefix if it exists
        icon = icon.replace(/^solid@/, "");
        return clsx(`fa fa-solid fa-${icon}`, fw ? "fa-fw" : null, opts?.spin ? "fa-spin" : null);
    }
    if (icon.match(/^regular@[a-z0-9-]+$/)) {
        // strip off the "regular@" prefix if it exists
        icon = icon.replace(/^regular@/, "");
        return clsx(`fa fa-sharp fa-regular fa-${icon}`, fw ? "fa-fw" : null, opts?.spin ? "fa-spin" : null);
    }
    if (icon.match(/^brands@[a-z0-9-]+$/)) {
        // strip off the "brands@" prefix if it exists
        icon = icon.replace(/^brands@/, "");
        return clsx(`fa fa-brands fa-${icon}`, fw ? "fa-fw" : null, opts?.spin ? "fa-spin" : null);
    }
    if (icon.match(/^custom@[a-z0-9-]+$/)) {
        // strip off the "custom@" prefix if it exists
        icon = icon.replace(/^custom@/, "");
        return clsx(`fa fa-kit fa-${icon}`, fw ? "fa-fw" : null, opts?.spin ? "fa-spin" : null);
    }
    if (opts?.defaultIcon != null) {
        return makeIconClass(opts.defaultIcon, fw, { spin: opts?.spin });
    }
    return null;
}

/**
 * A wrapper function for running a promise and catching any errors
 * @param f The promise to run
 */
function fireAndForget(f: () => Promise<any>) {
    f()?.catch((e) => {
        console.log("fireAndForget error", e);
    });
}

const promiseWeakMap = new WeakMap<Promise<any>, ResolvedValue<any>>();

type ResolvedValue<T> = {
    pending: boolean;
    error: any;
    value: T;
};

// ---------------------------------------------------------------------------
// SignalAtom — a SolidJS signal that is also callable as an accessor.
// Used to replace Jotai's WritableAtom pattern throughout the layout system.
// Call it to read, use ._set() to write.
// ---------------------------------------------------------------------------

export type SignalAtom<T> = {
    (): T;
    _set(v: T | ((prev: T) => T)): void;
};

export function createSignalAtom<T>(initial: T): SignalAtom<T> {
    const [get, set] = createSignal<T>(initial);
    const atom = () => get() as T;
    (atom as any)._set = set;
    return atom as unknown as SignalAtom<T>;
}

/** SolidJS-compatible useAtomValueSafe: safely call a signal accessor. Returns null when accessor is null. */
function useAtomValueSafe<T>(accessor: (() => T) | null | undefined): T {
    if (accessor == null) return null as T;
    return (accessor as () => T)();
}

/**
 * Simple wrapper function that lazily evaluates the provided function and caches its result for future calls.
 * @param callback The function to lazily run.
 * @returns The result of the function.
 */
const lazy = <T extends (...args: any[]) => any>(callback: T) => {
    let res: ReturnType<T>;
    let processed = false;
    return (...args: Parameters<T>): ReturnType<T> => {
        if (processed) return res;
        res = callback(...args);
        processed = true;
        return res;
    };
};

function atomWithThrottle<T>(initialValue: T, delayMilliseconds = 500): AtomWithThrottle<T> {
    const [currentValue, setCurrentValue] = createSignal<T>(initialValue);
    const [throttledValue, setThrottledValueDirect] = createSignal<T>(initialValue);

    const throttleUpdate = throttle(delayMilliseconds, () => {
        setThrottledValueDirect(() => currentValue() as T);
    });

    // throttledValueAtom is both readable and writable (SignalAtom)
    const throttledAtom = () => throttledValue() as T;
    (throttledAtom as any)._set = (update: T | ((prev: T) => T)) => {
        const prevValue = currentValue();
        const nextValue = typeof update === "function" ? (update as (prev: T) => T)(prevValue as T) : update;
        setCurrentValue(() => nextValue as T);
        throttleUpdate();
    };

    return {
        currentValueAtom: currentValue as () => T,
        throttledValueAtom: throttledAtom as unknown as SignalAtom<T>,
    };
}

function getPrefixedSettings(settings: SettingsType, prefix: string): SettingsType {
    const rtn: SettingsType = {};
    if (settings == null || isBlank(prefix)) {
        return rtn;
    }
    for (const key in settings) {
        if (key == prefix || key.startsWith(prefix + ":")) {
            rtn[key] = settings[key];
        }
    }
    return rtn;
}

function countGraphemes(str: string): number {
    if (str == null) {
        return 0;
    }
    // this exists (need to hack TS to get it to not show an error)
    const seg = new (Intl as any).Segmenter(undefined, { granularity: "grapheme" });
    return Array.from(seg.segment(str)).length;
}

function sleep(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms));
}

function cn(...inputs: ClassValue[]) {
    return twMerge(clsx(inputs));
}

export {
    atomWithThrottle,
    base64ToArray,
    boundNumber,
    cn,
    countGraphemes,
    deepCompareReturnPrev,
    fireAndForget,
    getPrefixedSettings,
    isBlank,
    lazy,
    makeIconClass,
    sleep,
    stringToBase64,
    useAtomValueSafe,
};
