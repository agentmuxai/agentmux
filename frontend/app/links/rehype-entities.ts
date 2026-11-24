// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The agent names, task numbers and pull requests in rendered Markdown made links (spec §4.5): the one recogniser
 * the Kanban uses, run over each text node that is not under a link, code or a script. Web addresses are left to
 * `rehypeLinkify`, which has already run. A link is an ordinary anchor: the Markdown `Link` draws it and its click
 * runs the usual chain, so the text never supplies a destination (`entity-href.ts` builds it from the entity).
 */

import type { Element, ElementContent, Root, Text } from "hast";
import type { Plugin } from "unified";
import { SKIP, visitParents } from "unist-util-visit-parents";
import { followAgentNames } from "@/app/store/agent-names";
import { linkify } from "./entities/linkify";
import type { Segment } from "./entities/types";
import { hrefOf } from "./entity-href";
import { linkContext, type SurfaceContext } from "./link-context";
import "./linkified.scss";

/** The kinds this plugin makes links of, and the sanitizer lets through with their class. */
export const ENTITY_LINK_CLASSES = ["entity-link", "entity-link--agent", "entity-link--task", "entity-link--pull", "entity-link--file", "entity-link--dead"];

const SKIP_ANCESTORS = new Set(["a", "code", "pre", "script", "style", "button"]);

type LinkSegment = Extract<Segment, { type: "link" }>;

function anchorOf(seg: LinkSegment): Element {
    return {
        type: "element",
        tagName: "a",
        properties: { href: hrefOf(seg.destination), className: ["entity-link", `entity-link--${seg.kind}`], ariaLabel: seg.name },
        children: [{ type: "text", value: seg.label }],
    };
}

/** Addresses are `rehypeLinkify`'s; an agent of another computer has no pane here to open, and its address
 *  could not say which computer. */
function linkable(seg: Segment): boolean {
    return seg.type === "link" && seg.kind !== "address" && !(seg.destination.kind === "agent" && seg.destination.computer);
}

function nodesOf(segments: Segment[]): ElementContent[] {
    const nodes: ElementContent[] = [];
    for (const seg of segments) {
        if (seg.type === "link" && linkable(seg)) nodes.push(anchorOf(seg));
        else nodes.push({ type: "text", value: "label" in seg ? seg.label : seg.text });
    }
    return nodes;
}

function textNodesOf(tree: Root): Array<{ node: Text; parent: Element | Root }> {
    const found: Array<{ node: Text; parent: Element | Root }> = [];
    visitParents(tree, "text", (node: Text, ancestors) => {
        if (ancestors.some((a) => a.type === "element" && SKIP_ANCESTORS.has((a as Element).tagName))) return SKIP;
        found.push({ node, parent: ancestors[ancestors.length - 1] as Element | Root });
    });
    return found;
}

/**
 * `surface` is read at transform time (the processor is built once and reused). `file.data.open` marks the tree that
 * is still growing: only the last text in it can be incomplete, so a match that touches its end waits (§4.4).
 */
export function rehypeEntities(surface: () => SurfaceContext | undefined): ReturnType<Plugin<[], Root>> {
    return (tree, file) => {
        const base = surface();
        if (!base) return;
        followAgentNames();
        const texts = textNodesOf(tree);
        const open = Boolean((file?.data as { open?: boolean } | undefined)?.open);
        texts.forEach(({ node, parent }, i) => {
            const final = !(open && i === texts.length - 1);
            const segments = linkify(node.value, linkContext({ ...base, final }));
            if (!segments.some(linkable)) return;
            const at = parent.children.indexOf(node as ElementContent);
            if (at >= 0) parent.children.splice(at, 1, ...nodesOf(segments));
        });
    };
}
