// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createMemo, For, type JSX } from "solid-js";
import LinkifyIt from "linkify-it";
import { openLink } from "@/app/store/global";

const linkify = new LinkifyIt();
linkify.set({ fuzzyLink: true, fuzzyEmail: false });

const SAFE_SCHEMES = /^(https?|ftp|mailto|ssh|file):\/\//i;

function isSafeHref(url: string): boolean {
    return SAFE_SCHEMES.test(url) || url.startsWith("//");
}

// Local addresses use http://, not https://
const LOCAL_HOST_RE = /^(localhost|127\.\d+\.\d+\.\d+|0\.0\.0\.0|\[::1\])(:\d+)?$/i;

function normalizeHref(url: string): string {
    if (url.startsWith("//")) return "https:" + url;
    if (url.includes("://")) return url;
    const scheme = LOCAL_HOST_RE.test(url.split("/")[0]) ? "http" : "https";
    return `${scheme}://${url}`;
}

type Segment = { text: string; href?: string };

function toSegments(text: string): Segment[] {
    const matches = linkify.match(text);
    if (!matches) return [{ text }];

    const segments: Segment[] = [];
    let lastIndex = 0;

    for (const match of matches) {
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
