// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The PDF viewer's safety rules, apart from pdf.js so they test without it.
 * A PDF the user opens may be hostile (an agent downloaded it, or was told
 * to); nothing in it may run script in the app.
 * docs/specs/SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.7.
 */

/** The oldest pdf.js without a known script-execution flaw: CVE-2026-16633
 *  is fixed in 6.2.108. pdf-policy.test.ts fails the build below it. */
export const MIN_PDFJS_VERSION = "6.2.108";

/** Every document opens with these: no XFA forms, no eval of font
 *  programs. (Scripting needs a scripting manager, which the viewer never
 *  gets.) */
export const SAFE_FLAGS = { enableXfa: false, isEvalSupported: false } as const;

/** Where an external link may go: http(s) opens a Browser pane; anything
 *  else (javascript:, file:, data:, a custom scheme) is dropped. */
export function linkAction(href: string): "browser" | "drop" {
    try {
        const u = new URL(href);
        return u.protocol === "http:" || u.protocol === "https:" ? "browser" : "drop";
    } catch {
        return "drop";
    }
}

/** a >= b for dotted version numbers. */
export function versionAtLeast(a: string, b: string): boolean {
    const pa = a.split(".").map(Number);
    const pb = b.split(".").map(Number);
    for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
        const x = pa[i] ?? 0;
        const y = pb[i] ?? 0;
        if (x !== y) return x > y;
    }
    return true;
}
