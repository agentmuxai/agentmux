// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Every per-tool fact the agent pane uses, in one table
 * (SPEC_AGENT_PANE_TOOL_DESCRIPTORS_2026_09_26.md).
 *
 * Deliberately pure: no JSX, no Solid, no registration side effects. The
 * stream parser, the virtualizer, the activity adapters and btw.ts all read
 * these facts and must not pull in the renderer module graph. How a result
 * body *renders* stays in `components/tool-renderers/registry.ts`; both layers
 * key on `toolNameOf(node)`.
 *
 * Adding a tool, or changing a tool's icon, label, detail or presentation,
 * means editing `TOOL_DESCRIPTORS` below and nothing else.
 */

import type { ToolNode } from "../types";

export interface ToolDescriptor {
    /** Raw provider names this descriptor covers, e.g. ["Read", "read_file"]. */
    names?: readonly string[];
    /** Or a raw-name prefix, e.g. "mcp__". A `names` match always wins over a prefix. */
    prefix?: string;
    icon?: string;
    /** The header's tool name; null omits it. */
    label?: (name: string, detail: string) => string | null;
    /** The call's main argument for the header: path, command, query, host/path. */
    detail?: (params: Record<string, any>) => string;
    /** "content": expanded by default once finished (SPEC_AGENT_PANE_ROW_DISCLOSURE). */
    presentation?: "panel" | "content";
    /** Where the preview box starts: following the latest output, or at the top. */
    scroll?: "follow" | "top";
}

type Fact = Exclude<keyof ToolDescriptor, "names" | "prefix">;

/** The raw provider name when carried, else the coarse kind. */
export function toolNameOf(n: Pick<ToolNode, "tool" | "toolName">): string {
    return n.toolName ?? n.tool;
}

const MCP_PREFIX = "mcp__";

/** "mcp__agentmux__WhoAmI" → "agentmux · WhoAmI"; null if not an MCP name. */
export function mcpDisplayName(name: string): string | null {
    if (!name.startsWith(MCP_PREFIX)) return null;
    const rest = name.slice(MCP_PREFIX.length);
    const sep = rest.indexOf("__");
    if (sep <= 0 || sep + 2 >= rest.length) return null;
    return `${rest.slice(0, sep)} · ${rest.slice(sep + 2)}`;
}

const pathOf = (p: Record<string, any>): string => p.file_path || p.path || "";

const hostPathOf = (p: Record<string, any>): string => {
    try {
        const u = new URL(p.url || "");
        return u.host + (u.pathname === "/" ? "" : u.pathname);
    } catch {
        return p.url || "";
    }
};

/** A globe followed by a query or a host/path can only be a web tool, so the
 *  name is dropped — unless there's no detail, or the row is a bare globe. */
const webLabel = (name: string, detail: string): string | null => (detail ? null : name);

// Order matters only among descriptors that match the same name the same way
// (see resolveFact). The catch-all is last.
export const TOOL_DESCRIPTORS: readonly ToolDescriptor[] = [
    // Documents: read from the top.
    { names: ["Read", "read", "read_file"], icon: "📖", detail: pathOf, scroll: "top" },
    { names: ["Write", "write", "write_file"], icon: "📝", detail: pathOf, scroll: "top" },
    { names: ["Edit", "edit", "str_replace_editor", "multiedit"], icon: "✏️", detail: pathOf, scroll: "top" },
    { names: ["Bash", "bash"], icon: "🔧", detail: (p) => p.command || "" },
    { names: ["computer"], detail: (p) => p.command || "" },
    { names: ["Grep", "grep"], icon: "🔍", detail: (p) => p.pattern || "" },
    { names: ["Glob", "glob"], icon: "📁", detail: (p) => p.pattern || "" },
    { names: ["Agent"], icon: "🤖", detail: (p) => p.description || p.prompt || "" },
    { names: ["Task"], icon: "🛠️" },
    { names: ["Workflow"], icon: "🕸️", detail: (p) => p.title || p.description || "" },
    // Content-first: the answer is the content (SPEC_TOOL_PREVIEW_CONTENT_FIRST §3.1).
    {
        names: ["WebSearch", "web_search"],
        icon: "🌐",
        label: webLabel,
        detail: (p) => p.query || "",
        presentation: "content",
        scroll: "top",
    },
    { names: ["WebFetch", "web_fetch"], icon: "🌐", label: webLabel, detail: hostPathOf },
    { prefix: MCP_PREFIX, label: (name) => mcpDisplayName(name) ?? name },
    // Catch-all: every fact's default. No header detail — see toolActivityArg.
    { icon: "🛠️", label: (name) => name, detail: () => "", presentation: "panel", scroll: "follow" },
];

const byName = (name: string) => (d: ToolDescriptor) => d.names?.includes(name) === true;
const byPrefix = (name: string) => (d: ToolDescriptor) => d.prefix !== undefined && name.startsWith(d.prefix);
const isCatchAll = (d: ToolDescriptor) => d.names === undefined && d.prefix === undefined;

/**
 * The first descriptor that matches `name` AND defines `fact`: exact names
 * first, then prefixes, then the catch-all. Resolving per fact lets a
 * generic descriptor (the `mcp__` prefix) supply one fact while the
 * catch-all supplies the rest. Exported with an explicit table for tests.
 */
export function resolveFact<K extends Fact>(
    table: readonly ToolDescriptor[],
    name: string,
    fact: K,
): ToolDescriptor[K] | undefined {
    for (const matches of [byName(name), byPrefix(name), isCatchAll]) {
        const d = table.find((e) => matches(e) && e[fact] !== undefined);
        if (d) return d[fact];
    }
    return undefined;
}

const fact = <K extends Fact>(name: string, key: K): NonNullable<ToolDescriptor[K]> =>
    resolveFact(TOOL_DESCRIPTORS, name, key)!;

const SPECIFIC = TOOL_DESCRIPTORS.filter((d) => !isCatchAll(d));

/**
 * A node's fact: by its raw name, then by its coarse kind, then the
 * catch-all. normalizeToolName() maps a noncanonical casing ("READ", "bAsH")
 * to the coarse kind, so such a node still gets its kind's facts (Codex P2
 * on #3901); a raw name with its own descriptor (WebSearch, whose kind is
 * "Other") keeps its own.
 */
function nodeFact<K extends Fact>(node: Pick<ToolNode, "tool" | "toolName">, key: K): NonNullable<ToolDescriptor[K]> {
    const own = resolveFact(SPECIFIC, toolNameOf(node), key);
    return (own ?? fact(node.tool, key)) as NonNullable<ToolDescriptor[K]>;
}

// ── facts ─────────────────────────────────────────────────────────────────

export function toolIcon(node: Pick<ToolNode, "tool" | "toolName">): string {
    return nodeFact(node, "icon");
}

/** The header's detail for a raw tool name ("" when the tool has none). */
export function toolDetail(name: string, params: Record<string, any> | undefined): string {
    return fact(name, "detail")(params ?? {});
}

/** A node's header detail: its raw name's, else its coarse kind's. */
export function toolDetailOf(node: Pick<ToolNode, "tool" | "toolName" | "params">): string {
    return nodeFact(node, "detail")((node.params as Record<string, any>) ?? {});
}

export function toolLabel(name: string, detail: string): string | null {
    return fact(name, "label")(name, detail);
}

const GENERIC_ARG_KEYS = ["file_path", "path", "command", "query", "pattern"] as const;

/**
 * The working row's argument: the header detail, else the first common
 * argument key present (a ToolSearch query, an MCP tool's path, …). The
 * header stays without that fallback, or every MCP row would grow a detail.
 */
export function toolActivityArg(name: string, params: Record<string, unknown> | undefined): string | undefined {
    const p = (params ?? {}) as Record<string, any>;
    const detail = toolDetail(name, p);
    if (detail) return detail;
    for (const k of GENERIC_ARG_KEYS) {
        if (typeof p[k] === "string" && p[k]) return p[k];
    }
    return undefined;
}

/** Expanded by default: a content-first tool that finished with a result
 *  (success, or failed with its error). While running it's auto-expanded
 *  like any tool; denied/canceled have nothing to show. */
export function isContentFirstTool(node: ToolNode): boolean {
    return (
        nodeFact(node, "presentation") === "content" && (node.status === "success" || node.status === "failed")
    );
}

/** The preview box opens at the top (documents, content-first answers)
 *  rather than following the latest output. By name, not status: the box is
 *  mounted while the tool is still running. */
export function startsAtTop(node: Pick<ToolNode, "tool" | "toolName">): boolean {
    return nodeFact(node, "scroll") === "top";
}
