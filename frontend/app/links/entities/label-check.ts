// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Spec §6.2: an agent can write `[Quill](https://example.test)`. A Markdown link keeps its own href, but when its label
 * is itself a recognised entity and the href is not that entity's own, the link is deceiving and the surface draws it
 * with a warning mark and its real address.
 */

import { linkify } from "./linkify";
import { pullUrl } from "./kinds/pull";
import type { Destination, LinkContext } from "./types";

const AGENT_HREF = /^agentmux:\/\/agent\/([^/?#]+)\/?$/i;
const TASK_HREF = /^agentmux:\/\/task\/(\d{1,9})\/?$/i;

function sameUrl(a: string, b: string): boolean {
    try {
        const x = new URL(a);
        const y = new URL(b);
        return x.protocol === y.protocol && x.host.toLowerCase() === y.host.toLowerCase() && x.pathname.replace(/\/$/, "") === y.pathname.replace(/\/$/, "") && x.search === y.search;
    } catch {
        return false;
    }
}

function hrefIs(destination: Destination, href: string): boolean {
    switch (destination.kind) {
        case "url":
            return sameUrl(destination.url, href);
        case "pull":
            return sameUrl(pullUrl(destination), href);
        case "task":
            return !destination.board && Number(TASK_HREF.exec(href)?.[1]) === destination.number;
        case "agent":
            return AGENT_HREF.exec(href)?.[1].toLowerCase() === destination.id.toLowerCase();
    }
}

/** The entity a Markdown label claims, when the whole label is one and `href` is not it; else null. */
export function deceptiveLabel(label: string, href: string, ctx: LinkContext): Destination | null {
    const text = label.trim();
    if (!text) return null;
    const segments = linkify(text, { ...ctx, final: true });
    const only = segments.length === 1 ? segments[0] : undefined;
    if (!only || only.type !== "link" || only.label !== text) return null;
    return hrefIs(only.destination, href.trim()) ? null : only.destination;
}
