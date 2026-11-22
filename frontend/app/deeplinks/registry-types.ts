// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The shape of the deep-link registry (docs/specs/SPEC_DEEP_LINKS_ONE_FRAMEWORK_2026_11_20.md §4, R10), as
 * `registry.generated.ts` holds it. The data lives in crates/common/deeplinks/destinations.json; srv reads the same
 * file through its own generated table (crates/common/src/deeplinks/registry.rs).
 */

/** What a variable segment, a parameter or an anchor holds. */
export type DeepLinkKind = "enum" | "id" | "slug" | "int" | "opaque" | "path" | "name";

/** What showing a view does beyond showing (G1): a view whose effect is not `none` is a worker. */
export type DeepLinkEffect = "none" | "starts" | "network";

/** What a view shows, which decides who may open and capture it (G5). `path`: by the path; `widget`: as the widget declares. */
export type DeepLinkSensitivity = "public" | "private" | "secret" | "path" | "widget";

/** Whether an agent may open it by MuxAPI: `own` only its own place, `reveal` only an open pane, `approved` only an approved package. */
export type DeepLinkAgentRule = "yes" | "own" | "reveal" | "approved" | "no";

export type DeepLinkSurface = "desktop" | "web" | "mobile";

export interface DeepLinkEnumValue {
    id: string;
    labelKey: string;
}

export interface DeepLinkVar {
    var: string;
    kind: DeepLinkKind;
    values?: DeepLinkEnumValue[];
    min?: number;
    max?: number;
    /** The value names an agent: an agent's open may name only itself (G5). */
    agentOwn?: boolean;
}

/** A literal segment, or a variable one. */
export type DeepLinkSegment = string | DeepLinkVar;

export interface DeepLinkParam {
    name: string;
    kind: DeepLinkKind;
    labelKey: string;
    values?: DeepLinkEnumValue[];
    min?: number;
    max?: number;
    /** Allowed only when the place's enum var holds one of these values. */
    onlyWith?: Record<string, string[]>;
    agentOwn?: boolean;
}

export interface DeepLinkPlace {
    id: string;
    /** Absent when the place is one enum segment: each value is labelled instead. */
    labelKey?: string;
    segments: DeepLinkSegment[];
    /** The enum var is the pane's section (`<view>:section`, R14). */
    sections?: boolean;
    anchor?: { kind: "id"; labelKey: string };
    params?: DeepLinkParam[];
}

export interface DeepLinkView {
    id: string;
    /** The pane view it opens (block `meta.view`), when it is not `id`; null for a view that is not a pane. */
    paneView?: string | null;
    labelKey: string;
    icon: string;
    build: number;
    effect: DeepLinkEffect;
    sensitivity: DeepLinkSensitivity;
    agent: DeepLinkAgentRule;
    surfaces: DeepLinkSurface[];
    /** The palette's command id, which is also its keybinding id (§4.1). */
    command?: string;
    keywords?: string[];
    group: string;
    /** The shortcut table's flag names; absent means always. */
    when?: string[];
    rank?: number;
    /** The link's view is `<id>.<package>` (a widget). */
    prefix?: boolean;
    /** The view opens only at a place inside it. */
    requiresPlace?: boolean;
    places: DeepLinkPlace[];
}

export interface DeepLinkAlias {
    from: string[];
    to: string[];
}

export interface DeepLinkRegistry {
    grammar: number;
    /** The build this AgentMux has: a view of a later build is declared and opens nothing. */
    build: number;
    maxBytes: number;
    separator: string;
    errors: Record<string, string>;
    views: DeepLinkView[];
    aliases: DeepLinkAlias[];
    /** Registered pane views that have no destination, and why. */
    unlinked: { view: string; reason: string }[];
}
