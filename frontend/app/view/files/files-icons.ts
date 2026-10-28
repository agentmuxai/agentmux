// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The system's own file icons for the Hangar (Files pane): the picture the
 * operating system shows for a file type, fetched from srv as a small PNG and
 * kept as an object URL. One request per file *type*, so a folder of 5,000
 * `.ts` files asks once; a few at a time; abandoned when every row that asked
 * has scrolled away; a failure isn't retried; and a 501 from srv (no system
 * source on this computer yet) turns the whole thing off for the session so
 * the built-in icons stay.
 * docs/specs/SPEC_HANGAR_SYSTEM_FILE_ICONS_2026_10_27.md §4.
 */
import { authHeaders } from "@/app/store/auth-headers";
import { getWebServerEndpoint } from "@/util/endpoints";
import { fetch } from "@/util/fetchutil";
import { extensionOf } from "./files-sort";

/** The sizes srv offers, in pixels (crates/srv/src/backend/file_icons/request.rs). */
export const ICON_PX = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256] as const;
/** Icons kept as object URLs; past this the least recently used is released. */
const MAX_CACHED = 200;
/** Requests in flight at once. */
const MAX_ACTIVE = 6;
/** What srv accepts as an extension (`[A-Za-z0-9_+-]`, up to 24). */
const EXT_RE = /^[a-z0-9_+-]{1,24}$/;

export interface IconTarget {
    kind: "file" | "dir";
    /** Lower case, without the dot; "" for a folder or a file with no usable extension. */
    ext: string;
}

/** What to ask srv for, for an entry: its type, never its path. */
export function iconTarget(entry: { name: string; is_dir: boolean }): IconTarget {
    if (entry.is_dir) return { kind: "dir", ext: "" };
    const ext = extensionOf(entry.name);
    return { kind: "file", ext: EXT_RE.test(ext) ? ext : "" };
}

/** The offered size that covers `cssPx` at `scale` (display scale times the pane's zoom): never scaled up. */
export function snapPx(cssPx: number, scale: number): number {
    const want = Math.ceil(cssPx * (scale > 0 ? scale : 1));
    return ICON_PX.find((p) => p >= want) ?? ICON_PX[ICON_PX.length - 1];
}

const keyOf = (t: IconTarget, px: number) => `${t.kind}|${t.ext}|${px}`;

const cache = new Map<string, string>();
interface Job {
    promise: Promise<string | null>;
    controller: AbortController;
    interest: number;
}
const inflight = new Map<string, Job>();
/** Types that couldn't be made: not asked for again this session. */
const failed = new Set<string>();
let unavailable = false;
let active = 0;
const waiting: (() => void)[] = [];

/** False once srv has said this computer has no system icon source. */
export function systemIconsAvailable(): boolean {
    return !unavailable;
}

/** A cached icon, without asking for one. */
export function cachedIcon(t: IconTarget, px: number): string | undefined {
    const key = keyOf(t, px);
    const url = cache.get(key);
    if (url) {
        cache.delete(key);
        cache.set(key, url);
    }
    return url;
}

/** Run `work` once fewer than MAX_ACTIVE are running; a finishing job hands its slot straight to the next waiter. */
async function slot<T>(work: () => Promise<T>): Promise<T> {
    if (active >= MAX_ACTIVE) await new Promise<void>((r) => waiting.push(r));
    else active++;
    try {
        return await work();
    } finally {
        const next = waiting.shift();
        if (next) next();
        else active--;
    }
}

function url(t: IconTarget, px: number): string {
    return `${getWebServerEndpoint()}/agentmux/file-icon?kind=${t.kind}&ext=${encodeURIComponent(t.ext)}&px=${px}`;
}

/**
 * The icon of `t` at `px` as an object URL, or null when there isn't one (srv
 * has no system source here, the type has no icon, or it failed before). Rows
 * that scroll away pass an aborted `signal`; the request stops once none is left.
 */
export function systemIcon(t: IconTarget, px: number, signal?: AbortSignal): Promise<string | null> {
    const key = keyOf(t, px);
    const hit = cachedIcon(t, px);
    if (hit) return Promise.resolve(hit);
    if (unavailable || failed.has(key)) return Promise.resolve(null);
    let job = inflight.get(key);
    if (!job) {
        const controller = new AbortController();
        job = { controller, interest: 0, promise: make(t, px, key, controller.signal) };
        inflight.set(key, job);
    }
    const j = job;
    j.interest++;
    signal?.addEventListener(
        "abort",
        () => {
            if (--j.interest <= 0 && inflight.get(key) === j) {
                inflight.delete(key);
                j.controller.abort();
            }
        },
        { once: true }
    );
    return j.promise;
}

function make(t: IconTarget, px: number, key: string, signal: AbortSignal): Promise<string | null> {
    return slot(async () => {
        if (signal.aborted) return null;
        try {
            const resp = await fetch(url(t, px), { headers: authHeaders(), signal });
            if (!resp.ok) {
                void resp.body?.cancel().catch(() => {});
                if (resp.status === 501) unavailable = true;
                else if (resp.status !== 503 && resp.status !== 504) failed.add(key);
                return null;
            }
            const blob = await resp.blob();
            if (signal.aborted) return null;
            const object = URL.createObjectURL(new Blob([blob], { type: "image/png" }));
            cache.set(key, object);
            while (cache.size > MAX_CACHED) {
                const [oldKey, oldUrl] = cache.entries().next().value!;
                cache.delete(oldKey);
                URL.revokeObjectURL(oldUrl);
            }
            return object;
        } catch {
            if (!signal.aborted) failed.add(key);
            return null;
        } finally {
            if (inflight.get(key)?.controller.signal === signal) inflight.delete(key);
        }
    });
}

/** Forget everything (tests). */
export function resetIconsForTest(): void {
    for (const u of cache.values()) URL.revokeObjectURL(u);
    cache.clear();
    inflight.clear();
    failed.clear();
    unavailable = false;
    active = 0;
    waiting.length = 0;
}
