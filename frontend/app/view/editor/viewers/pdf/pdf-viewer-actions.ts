// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** What the reader does in the PDF viewer: go to a page, find, the keys, and the facts the bar shows. Plain functions over the state; they add no reactives. */

import { formatBytes } from "@/util/format-bytes";
import { linkAction } from "./pdf-policy";
import type { PdfViewerState } from "./pdf-viewer-state";

export function goTo(s: PdfViewerState, n: number) {
    if (!s.viewer || !s.pages()) return;
    s.viewer.currentPageNumber = Math.max(1, Math.min(s.pages(), Math.round(n)));
}

export function find(s: PdfViewerState, again: boolean, previous = false) {
    if (!s.eventBus) return;
    s.eventBus.dispatch("find", {
        source: null,
        type: again ? "again" : "",
        query: s.query(),
        caseSensitive: false,
        entireWord: false,
        highlightAll: true,
        findPrevious: previous,
        matchDiacritics: false,
    });
}

export function onKeyDown(s: PdfViewerState, e: KeyboardEvent) {
    const mod = e.ctrlKey || e.metaKey;
    if (mod && (e.key === "f" || e.key === "F")) {
        e.preventDefault();
        e.stopPropagation();
        s.findInput?.focus();
        s.findInput?.select();
    } else if (mod && (e.key === "g" || e.key === "G")) {
        e.preventDefault();
        e.stopPropagation();
        s.pageInput?.focus();
        s.pageInput?.select();
    } else if (!mod && e.key === "Home" && e.target === s.container) {
        e.preventDefault();
        goTo(s, 1);
    } else if (!mod && e.key === "End" && e.target === s.container) {
        e.preventDefault();
        goTo(s, s.pages());
    }
}

/** Links: internal ones are pdf.js's; an external one opens a Browser pane or is dropped, and never navigates the app. */
export function onLinkClick(s: PdfViewerState, e: MouseEvent) {
    const a = (e.target as Element | null)?.closest?.("a[href]") as HTMLAnchorElement | null;
    if (!a) return;
    const href = a.getAttribute("href") ?? "";
    if (href.startsWith("#")) return;
    e.preventDefault();
    e.stopPropagation();
    if (linkAction(a.href) === "browser") s.props.onOpenLink(a.href);
}

export function facts(s: PdfViewerState): string[] {
    const out: string[] = [];
    if (s.pages()) out.push(`${s.pages()} page${s.pages() === 1 ? "" : "s"}`);
    const size = s.props.size ?? s.props.file?.size;
    if (size != null) out.push(formatBytes(size));
    out.push("PDF");
    return out;
}
