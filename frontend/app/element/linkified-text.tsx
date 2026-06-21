// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createMemo, For, type JSX } from "solid-js";
import { openLink } from "@/app/store/global";
import { linkify, normalizeHref, isLikelyFilename, isSafeHref } from "./linkify-config";

type Segment = { text: string; href?: string };

function toSegments(text: string): Segment[] {
    const matches = linkify.match(text);
    if (!matches) return [{ text }];

    const segments: Segment[] = [];
    let lastIndex = 0;

    for (const match of matches) {
        // Drop fuzzy matches that are actually source-code filenames
        // (e.g. README.md, main.rs, setup.py treated as ccTLD URLs)
        if (isLikelyFilename(match.schema, match.url)) {
            if (match.index > lastIndex) {
                segments.push({ text: text.slice(lastIndex, match.index) });
            }
            segments.push({ text: match.raw });
            lastIndex = match.lastIndex;
            continue;
        }

        if (match.index > lastIndex) {
            segments.push({ text: text.slice(lastIndex, match.index) });
        }
        segments.push({ text: match.text, href: normalizeHref(match.url) });
        lastIndex = match.lastIndex;
    }

    if (lastIndex < text.length) {
        segments.push({ text: text.slice(lastIndex) });
    }

    return segments;
}

/**
 * Renders plain text with detected URLs converted to clickable links.
 * Safe to use inside <pre> — renders as inline text + <a> elements.
 */
export const LinkifiedText = (props: { text: string }): JSX.Element => {
    const segments = createMemo(() => toSegments(props.text));
    return (
        <For each={segments()}>
            {(seg) => {
                if (seg.href && isSafeHref(seg.href)) {
                    return (
                        <a
                            href={seg.href}
                            class="linkified-url"
                            onClick={(e) => {
                                e.preventDefault();
                                openLink(seg.href!);
                            }}
                        >
                            {seg.text}
                        </a>
                    );
                }
                return <>{seg.text}</>;
            }}
        </For>
    );
};
