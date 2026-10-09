// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a running tool call is doing, in words, for the working row's live
 * status. Prefers the words the model already wrote for exactly this: Bash's
 * `description`, the Agent tool's `description`. Otherwise a plain verb and
 * the object (a file's base name, a search pattern). Never `Tool · arg`.
 *
 * docs/specs/SPEC_AGENT_TURN_MODEL_AND_LIVE_STATUS_2026_10_08.md §6.2.
 */

/** Tools grouped by what they do, so a burst of one kind folds into one line. */
export type ToolFamily = "read" | "search" | "edit" | "bash" | "agent" | "web" | "plan" | "other";

export interface ToolActivity {
    family: ToolFamily;
    /** One call, alone: "Running the srv test suite", "Reading AgentFooter.tsx". */
    label: string;
}

const MAX_LABEL = 80;

function clip(s: string, max = MAX_LABEL): string {
    const one = s.replace(/\s+/g, " ").trim();
    return one.length > max ? `${one.slice(0, max - 1)}…` : one;
}

function basename(path: string): string {
    const parts = path.split(/[\\/]/).filter(Boolean);
    return parts.at(-1) ?? path;
}

function str(v: unknown): string | null {
    return typeof v === "string" && v.trim() ? v.trim() : null;
}

/** "Run the tests" → "Running the tests"; anything else is kept as written. */
function asProgressive(description: string): string {
    const m = /^(\w+)(\b[\s\S]*)$/.exec(description);
    if (!m) return description;
    const [, verb, rest] = m;
    const lower = verb.toLowerCase();
    // Already "-ing", or not a plain imperative verb we can safely inflect.
    if (lower.endsWith("ing") || !/^[a-z]+$/.test(lower) || lower.length < 3) return description;
    const IRREGULAR: Record<string, string> = { run: "Running", get: "Getting", set: "Setting", stop: "Stopping", put: "Putting", cut: "Cutting", see: "Seeing", be: "Being" };
    let ing = IRREGULAR[lower];
    if (!ing) {
        const stem = lower.endsWith("ie") ? `${lower.slice(0, -2)}y` : lower.endsWith("e") && !lower.endsWith("ee") ? lower.slice(0, -1) : lower;
        ing = `${stem[0].toUpperCase()}${stem.slice(1)}ing`;
    }
    return `${ing}${rest}`;
}

/** The working row's words for one tool call. */
export function toolActivity(name: string, params: Record<string, unknown> | undefined): ToolActivity {
    const p = params ?? {};
    const short = name.startsWith("mcp__") ? (name.split("__").at(-1) ?? name) : name;
    switch (name) {
        case "Bash":
        case "BashOutput":
        case "PowerShell": {
            const description = str(p.description);
            if (description) return { family: "bash", label: clip(asProgressive(description)) };
            const command = str(p.command);
            return { family: "bash", label: command ? `Running ${clip(command.split(/\s+/)[0] ?? command, 40)}` : "Running a command" };
        }
        case "Agent":
        case "Task": {
            const description = str(p.description);
            const kind = str(p.subagent_type);
            const who = kind && kind !== "general-purpose" ? `${kind} agent` : "Subagent";
            return { family: "agent", label: description ? clip(`${who}: ${description}`) : `${who} working` };
        }
        case "Read":
        case "NotebookRead": {
            const path = str(p.file_path) ?? str(p.notebook_path) ?? str(p.path);
            return { family: "read", label: path ? `Reading ${clip(basename(path), 60)}` : "Reading a file" };
        }
        case "Grep":
        case "Glob": {
            const pattern = str(p.pattern);
            return { family: "search", label: pattern ? `Searching for ${clip(pattern, 50)}` : "Searching the code" };
        }
        case "Edit":
        case "MultiEdit":
        case "NotebookEdit": {
            const path = str(p.file_path) ?? str(p.notebook_path);
            return { family: "edit", label: path ? `Editing ${clip(basename(path), 60)}` : "Editing a file" };
        }
        case "Write": {
            const path = str(p.file_path);
            return { family: "edit", label: path ? `Writing ${clip(basename(path), 60)}` : "Writing a file" };
        }
        case "WebSearch": {
            const query = str(p.query);
            return { family: "web", label: query ? `Searching the web: ${clip(query, 60)}` : "Searching the web" };
        }
        case "WebFetch": {
            const url = str(p.url);
            let host: string | null = null;
            try {
                host = url ? new URL(url).host : null;
            } catch {
                host = null;
            }
            return { family: "web", label: host ? `Reading ${host}` : "Fetching a page" };
        }
        case "TodoWrite":
            return { family: "plan", label: "Updating the plan" };
        default:
            return { family: "other", label: `Using ${clip(short, 40)}` };
    }
}

const FAMILY_PLURAL: Partial<Record<ToolFamily, (n: number) => string>> = {
    read: (n) => `Reading ${n} files`,
    search: (n) => `Running ${n} searches`,
    edit: (n) => `Editing ${n} files`,
    bash: (n) => `Running ${n} commands`,
    agent: (n) => `${n} subagents working`,
    web: (n) => `${n} web lookups`,
};

/** Several calls running at once, in one line: the single call's own label,
 *  "Reading 3 files" for one kind, or "3 tools running" for a mix. */
export function foldActivities(activities: readonly ToolActivity[]): string | null {
    if (activities.length === 0) return null;
    if (activities.length === 1) return activities[0].label;
    const families = new Set(activities.map((a) => a.family));
    if (families.size === 1) {
        const plural = FAMILY_PLURAL[activities[0].family];
        if (plural) return plural(activities.length);
    }
    // A subagent among them is the one worth naming.
    const agent = activities.find((a) => a.family === "agent");
    if (agent) return `${agent.label} (+${activities.length - 1})`;
    return `${activities.length} tools running`;
}
