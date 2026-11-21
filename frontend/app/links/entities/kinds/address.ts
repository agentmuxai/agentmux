// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Web addresses. The existing `linkify-it` layer (spec 2026-06-20) moved behind the registry, with the scheme rule of
 * spec §6: a link made from text is http or https and nothing else.
 */

import { isLikelyFilename, linkify, normalizeHref } from "@/app/element/linkify-config";
import { openUrlEntity } from "../openers";
import type { Candidate, EntityKind } from "../types";

function safeUrl(raw: string): string | null {
    try {
        const url = new URL(raw);
        return url.protocol === "http:" || url.protocol === "https:" ? raw : null;
    } catch {
        return null;
    }
}

export const addressKind: EntityKind = {
    id: "address",
    priority: 40,
    recognise(text) {
        const found: Candidate[] = [];
        for (const m of linkify.match(text) ?? []) {
            if (isLikelyFilename(m.schema, m.url)) continue;
            // `mailto:`, `ftp:` and a protocol-relative `//host` are not text-derived web links.
            if (m.schema && m.schema !== "http:" && m.schema !== "https:") continue;
            found.push({ kind: "address", start: m.index, end: m.lastIndex, data: { url: normalizeHref(m.schema, m.url) } });
        }
        return found;
    },
    resolve(candidate) {
        const url = safeUrl(candidate.data.url);
        if (!url) return null;
        return { destination: { kind: "url", url }, state: "ready", name: `Open ${url}` };
    },
    open(destination, options) {
        if (destination.kind !== "url") return Promise.resolve({ ok: false, reason: "not an address" });
        return openUrlEntity(destination.url, options);
    },
};
