// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The app's parser of deep links (docs/specs/SPEC_DEEP_LINKS_ONE_FRAMEWORK_2026_11_20.md §3.2, R6 to R9, G7, G8).
 * srv has the same parser in Rust (crates/common/src/deeplinks/parse.rs); both are written from the one grammar and
 * the one registry, and both run crates/common/deeplinks/conformance.json. A change here is a change there.
 *
 * The order of the checks is part of the contract, so both sides give the same reason for the same link:
 * 1. length, then the scheme;
 * 2. the shape: view, segments, query and anchor, no empty part;
 * 3. each part, left to right: its characters, its percent-encoding (decoded once), control characters, its one
 *    written form;
 * 4. the view (an alias is rewritten first), its build, the place, the parameters, the anchor.
 */

import { DEEPLINK_REGISTRY } from "./registry.generated";
import type { DeepLinkParam, DeepLinkPlace, DeepLinkRegistry, DeepLinkView } from "./registry-types";

export type DeepLinkErrorCode =
    | "too_long"
    | "not_agentmux"
    | "bad_syntax"
    | "bad_encoding"
    | "double_encoded"
    | "control_char"
    | "not_canonical"
    | "unknown_view"
    | "unknown_place"
    | "unknown_param"
    | "unknown_anchor"
    | "duplicate_param"
    | "empty_param"
    | "bad_value"
    | "bad_path"
    | "missing_place"
    | "not_yet";

export type DeepLinkValue = string | number;

/** A link read: its view, the place it matched, the values of the place's vars, its parameters and anchor. */
export interface ParsedDeepLink {
    /** The registry view (`ext` for a widget). */
    view: string;
    /** The place's id, or null for the view as it is. */
    place: string | null;
    /** Each var of the place by name; for a widget, `package` too. */
    values: Record<string, DeepLinkValue>;
    params: Record<string, DeepLinkValue>;
    anchor: string | null;
    /** The link's one written form (R7): what copying it gives. */
    canonical: string;
}

export type DeepLinkParse = { ok: true; link: ParsedDeepLink } | { ok: false; code: DeepLinkErrorCode; reason: string };

export const SCHEME = "agentmux://";

const UNRESERVED = /^[A-Za-z0-9\-._~]$/;
const PCT = /%[0-9A-Fa-f]{2}/;
/** C0 and C1 controls, DEL, the bidirectional marks, embeddings, overrides and isolates, zero-width characters (G8). */
const CONTROL = /[\u0000-\u001f\u007f-\u009f\u061c\u200b-\u200f\u202a-\u202e\u2060-\u2069\ufeff]/u;
const ID = /^[a-z0-9][a-z0-9._-]{0,127}$/;
const SLUG = /^[a-z0-9][a-z0-9-]{0,23}$/;
const INT = /^(0|[1-9][0-9]{0,9})$/;
const OPAQUE = /^e_[a-z0-9]{8,32}$/;
const PARAM_KEY = /^[a-z][a-z0-9_]{0,31}$/;
const PACKAGE = /^[a-z0-9][a-z0-9-]*\.[a-z0-9][a-z0-9-]*$/;
/** Windows device names, which open a device whatever folder or extension they are written with. */
const DEVICE = /^(con|prn|aux|nul|com[0-9\u00b9\u00b2\u00b3]|lpt[0-9\u00b9\u00b2\u00b3]|conin\$|conout\$)$/i;

class Refusal {
    constructor(readonly code: DeepLinkErrorCode) {}
}

const refuse = (code: DeepLinkErrorCode): never => {
    throw new Refusal(code);
};

/** The one percent-encoding of `text` (R7): unreserved characters as they are, every other byte as `%HH`, upper case. */
export function encodePart(text: string): string {
    let out = "";
    for (const byte of new TextEncoder().encode(text)) {
        const ch = String.fromCharCode(byte);
        out += byte < 0x80 && UNRESERVED.test(ch) ? ch : `%${byte.toString(16).toUpperCase().padStart(2, "0")}`;
    }
    return out;
}

/** `raw` decoded once, with every check of step 3. */
function decodePart(raw: string): string {
    const bytes: number[] = [];
    for (let i = 0; i < raw.length; i++) {
        const ch = raw[i];
        if (ch === "%") {
            const hex = raw.slice(i + 1, i + 3);
            if (!/^[0-9A-Fa-f]{2}$/.test(hex)) refuse("bad_encoding");
            bytes.push(parseInt(hex, 16));
            i += 2;
        } else if (UNRESERVED.test(ch)) {
            bytes.push(ch.charCodeAt(0));
        } else {
            refuse("bad_syntax");
        }
    }
    let text: string;
    try {
        // ignoreBOM keeps a leading byte-order mark as a character, as Rust's from_utf8 does, so it is refused below.
        text = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(new Uint8Array(bytes));
    } catch {
        return refuse("bad_encoding");
    }
    if (PCT.test(text)) refuse("double_encoded");
    if (CONTROL.test(text)) refuse("control_char");
    if (encodePart(text) !== raw) refuse("not_canonical");
    return text;
}

/** Why `path` may not be opened by a link, as a structural check both sides make the same way (G7), or null. srv
 *  applies the rest of G7 (what the caller may read, where a symbolic link leads) when the link is opened. */
export function pathProblem(path: string): DeepLinkErrorCode | null {
    if (path.includes("\\")) return "bad_path";
    let rest: string;
    if (/^[A-Za-z]:\//.test(path)) rest = path.slice(3);
    else if (path.startsWith("/") && !path.startsWith("//")) rest = path.slice(1);
    else return "bad_path"; // relative, a drive-relative `C:x`, or a UNC `//server`
    if (rest === "") return null;
    for (const seg of rest.split("/")) {
        if (seg === "" || seg === "." || seg === "..") return "bad_path";
        if (seg.includes(":")) return "bad_path"; // an alternate data stream, or a second drive
        if (seg.endsWith(".") || seg.endsWith(" ")) return "bad_path"; // Windows drops them: one path, two spellings
        if (DEVICE.test(seg.split(".")[0])) return "bad_path";
    }
    return null;
}

function nameProblem(name: string): boolean {
    return [...name].length > 255 || name === "." || name === ".." || /[/\\:]/.test(name) || DEVICE.test(name.split(".")[0]) || name.endsWith(" ") || name.endsWith(".");
}

type Typed = { kind: DeepLinkParam["kind"]; values?: { id: string }[]; min?: number; max?: number };

/** `text` read as a value of `spec`'s kind. An enum value that isn't one is the caller's to name (place or value). */
function typedValue(spec: Typed, text: string, enumMiss: DeepLinkErrorCode): DeepLinkValue {
    switch (spec.kind) {
        case "enum":
            if (spec.values!.some((v) => v.id === text)) return text;
            return refuse(spec.values!.some((v) => v.id === text.toLowerCase()) ? "not_canonical" : enumMiss);
        case "id":
            if (ID.test(text)) return text;
            return refuse(ID.test(text.toLowerCase()) ? "not_canonical" : "bad_value");
        case "slug":
            if (SLUG.test(text)) return text;
            return refuse(SLUG.test(text.toLowerCase()) ? "not_canonical" : "bad_value");
        case "int": {
            if (!INT.test(text)) return refuse(/^0[0-9]+$/.test(text) ? "not_canonical" : "bad_value");
            const n = Number(text);
            if (n < (spec.min ?? 0) || n > (spec.max ?? 2147483647)) return refuse("bad_value");
            return n;
        }
        case "opaque":
            return OPAQUE.test(text) ? text : refuse("bad_value");
        case "path":
            return pathProblem(text) ? refuse("bad_path") : text;
        case "name":
            return nameProblem(text) ? refuse("bad_value") : text;
    }
}

interface Shape {
    head: string;
    segments: string[];
    params: [string, string][];
    anchor: string | null;
}

/** Step 2 and 3: the link cut into its parts, each decoded once. */
function shapeOf(link: string, reg: DeepLinkRegistry): Shape {
    if (new TextEncoder().encode(link).length > reg.maxBytes) refuse("too_long");
    if (!link.startsWith(SCHEME)) refuse("not_agentmux");
    let rest = link.slice(SCHEME.length);
    let anchorRaw: string | null = null;
    const hash = rest.indexOf("#");
    if (hash >= 0) {
        anchorRaw = rest.slice(hash + 1);
        rest = rest.slice(0, hash);
        if (anchorRaw === "" || anchorRaw.includes("#")) refuse("bad_syntax");
    }
    let queryRaw: string | null = null;
    const qm = rest.indexOf("?");
    if (qm >= 0) {
        queryRaw = rest.slice(qm + 1);
        rest = rest.slice(0, qm);
        if (queryRaw === "" || queryRaw.includes("?")) refuse("bad_syntax");
    }
    const rawSegments = rest.split("/");
    if (rawSegments.some((s) => s === "")) refuse("bad_syntax");
    const rawParams: [string, string][] = [];
    for (const piece of queryRaw === null ? [] : queryRaw.split("&")) {
        const eq = piece.indexOf("=");
        if (piece === "" || eq < 0) refuse("bad_syntax");
        rawParams.push([piece.slice(0, eq), piece.slice(eq + 1)]);
    }
    const [head, ...segments] = rawSegments.map(decodePart);
    const params: [string, string][] = rawParams.map(([k, v]) => {
        if (!PARAM_KEY.test(k)) refuse(PARAM_KEY.test(k.toLowerCase()) ? "not_canonical" : "bad_syntax");
        if (v === "") refuse("empty_param");
        return [k, decodePart(v)];
    });
    const anchor = anchorRaw === null ? null : decodePart(anchorRaw);
    return { head, segments, params, anchor };
}

/** An alias rewritten to the form it stands for (R7). */
function unalias(shape: Shape, reg: DeepLinkRegistry): Shape {
    for (const alias of reg.aliases) {
        const parts = [shape.head, ...shape.segments];
        if (alias.from.length !== parts.length || alias.from[0] !== parts[0]) continue;
        const bound: Record<string, string> = {};
        const fits = alias.from.every((f, i) => {
            if (/^\{\w+\}$/.test(f)) {
                bound[f] = parts[i];
                return true;
            }
            return f === parts[i];
        });
        if (!fits) continue;
        const to = alias.to.map((t) => bound[t] ?? t);
        return { ...shape, head: to[0], segments: to.slice(1) };
    }
    return shape;
}

function findView(head: string, reg: DeepLinkRegistry): { view: DeepLinkView; pkg?: string } {
    const exact = reg.views.find((v) => !v.prefix && v.id === head);
    if (exact) return { view: exact };
    for (const v of reg.views.filter((x) => x.prefix)) {
        const lead = `${v.id}.`;
        if (head.startsWith(lead) && PACKAGE.test(head.slice(lead.length))) return { view: v, pkg: head.slice(lead.length) };
    }
    if (head !== head.toLowerCase() && reg.views.some((v) => v.id === head.toLowerCase())) refuse("not_canonical");
    return refuse("unknown_view");
}

/** The place `segments` names, with its vars' values. */
function matchPlace(view: DeepLinkView, segments: string[]): { place: DeepLinkPlace; values: Record<string, DeepLinkValue> } {
    const shaped = view.places.filter(
        (p) =>
            p.segments.length === segments.length &&
            p.segments.every((s, i) => typeof s !== "string" || s === segments[i] || s === segments[i].toLowerCase())
    );
    if (shaped.length === 0) return refuse("unknown_place");
    // A literal written in another case is the right place in the wrong form.
    const exact = shaped.filter((p) => p.segments.every((s, i) => typeof s !== "string" || s === segments[i]));
    if (exact.length === 0) return refuse("not_canonical");
    let first: Refusal | null = null;
    for (const place of exact) {
        try {
            const values: Record<string, DeepLinkValue> = {};
            place.segments.forEach((s, i) => {
                if (typeof s !== "string") values[s.var] = typedValue(s, segments[i], "unknown_place");
            });
            return { place, values };
        } catch (err) {
            if (!(err instanceof Refusal)) throw err;
            first ??= err;
        }
    }
    throw first!;
}

function readParams(place: DeepLinkPlace | null, values: Record<string, DeepLinkValue>, given: [string, string][]): Record<string, DeepLinkValue> {
    const declared = place?.params ?? [];
    const seen = new Set<string>();
    let lastIndex = -1;
    const out: Record<string, DeepLinkValue> = {};
    for (const [key, text] of given) {
        if (seen.has(key)) refuse("duplicate_param");
        seen.add(key);
        const index = declared.findIndex((p) => p.name === key);
        if (index < 0) refuse("unknown_param");
        const spec = declared[index];
        if (spec.onlyWith && !Object.entries(spec.onlyWith).every(([v, allowed]) => allowed.includes(String(values[v])))) refuse("unknown_param");
        if (index < lastIndex) refuse("not_canonical");
        lastIndex = index;
        out[key] = typedValue(spec, text, "bad_value");
    }
    return out;
}

function headOf(view: DeepLinkView, values: Record<string, DeepLinkValue>): string {
    return view.prefix ? `${view.id}.${values.package}` : view.id;
}

/** The one written form of a destination (R7): what copying a link gives, and what the app writes. */
export function formatLink(
    dest: { view: string; place?: string | null; values?: Record<string, DeepLinkValue>; params?: Record<string, DeepLinkValue>; anchor?: string | null },
    reg: DeepLinkRegistry = DEEPLINK_REGISTRY
): string {
    const view = reg.views.find((v) => v.id === dest.view);
    if (!view) throw new Error(`no deep-link view "${dest.view}"`);
    const values = dest.values ?? {};
    const place = dest.place ? view.places.find((p) => p.id === dest.place) : null;
    if (dest.place && !place) throw new Error(`the ${view.id} view has no place "${dest.place}"`);
    const parts = [headOf(view, values), ...(place?.segments ?? []).map((s) => (typeof s === "string" ? s : String(values[s.var])))];
    let link = SCHEME + parts.map(encodePart).join("/");
    const params = (place?.params ?? []).filter((p) => dest.params?.[p.name] !== undefined);
    if (params.length) link += "?" + params.map((p) => `${p.name}=${encodePart(String(dest.params![p.name]))}`).join("&");
    if (dest.anchor) link += `#${encodePart(dest.anchor)}`;
    return link;
}

/** Reads `link` against the registry: the destination it names, or why it is refused (R8). */
export function parseLink(link: string, reg: DeepLinkRegistry = DEEPLINK_REGISTRY): DeepLinkParse {
    try {
        const shape = unalias(shapeOf(link, reg), reg);
        const { view, pkg } = findView(shape.head, reg);
        if (view.build > reg.build) refuse("not_yet");
        let place: DeepLinkPlace | null = null;
        let values: Record<string, DeepLinkValue> = {};
        if (shape.segments.length === 0) {
            if (view.requiresPlace) refuse("missing_place");
        } else {
            ({ place, values } = matchPlace(view, shape.segments));
        }
        if (pkg) values = { package: pkg, ...values };
        const params = readParams(place, values, shape.params);
        if (shape.anchor !== null) {
            if (!place?.anchor) refuse("unknown_anchor");
            typedValue({ kind: "id" }, shape.anchor, "bad_value");
        }
        const parsed = { view: view.id, place: place?.id ?? null, values, params, anchor: shape.anchor };
        return { ok: true, link: { ...parsed, canonical: formatLink(parsed, reg) } };
    } catch (err) {
        if (!(err instanceof Refusal)) throw err;
        return { ok: false, code: err.code, reason: reg.errors[err.code] ?? err.code };
    }
}
