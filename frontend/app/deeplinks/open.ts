// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The executor (docs/specs/SPEC_DEEP_LINKS_ONE_FRAMEWORK_2026_11_20.md R1 to R5, R11a, §8): `openDestination` and the
 * typed wrappers. Every destination goes through one path: a request is written as a link and read back, so there is
 * one validation; the link becomes a pane and meta (`destination-meta.ts`); the pane is found or made through the
 * window's own openers (`open-deps.ts`), and nothing else makes panes.
 *
 * What it guarantees: an open is one pane, never a chain (G10); a person's opens are at most five a second (G11); an
 * agent's open takes no focus, is announced in a polite live region (G13), waits while the person is typing, lands
 * beside the agent's own conversation, and never replaces an agent that is running, a terminal with a process, an
 * editor with unsaved changes, a browser pane or a pane the person has used (R5); the same destination asked for
 * again reuses its pane and reveals the place again (R4); a place that is gone opens the view with
 * `place: "missing"` (§8).
 */

import type {
    CanOpen,
    DestinationRequest,
    OpenOptions,
    OpenOutcome,
    OpenReasonCode,
    OpenSource,
    OpenTarget,
    OpenWrappers,
} from "./open-types";
import { formatLink, parseLink, type ParsedDeepLink } from "./parse";
import { destinationLabel, deepLinkView } from "./registry";
import { isPlanProblem, planDestination, type PanePlan } from "./destination-meta";

/** What a pane is, for the replace rules (R5). */
export interface PaneFacts {
    view: string;
    /** An agent pane whose agent is running. */
    agentRunning?: boolean;
    /** A terminal with a process. */
    termProcess?: boolean;
    /** An editor with unsaved changes. */
    editorDirty?: boolean;
    /** The agent that opened it through a link, when one did. */
    openedBy?: string;
    /** The person has clicked or typed in it since. */
    usedByPerson?: boolean;
}

/** Whether `pane` may be replaced by `view` for `source` (R5). "reuse": it shows the same view. */
export function replaceRule(pane: PaneFacts, view: string, source: OpenSource): "reuse" | "replace" | "never" {
    if (pane.view === view) return "reuse";
    if (pane.agentRunning || pane.termProcess || pane.editorDirty || pane.view === "browser" || pane.usedByPerson) return "never";
    // Only a pane the caller opened itself and the person has not used.
    if (source.kind === "agent" && pane.openedBy === source.agentId) return "replace";
    return "never";
}

/** Who may have the person's attention for an agent's open: typing, dictating or composing (2 s, at most 10 s). */
export interface PersonInput {
    lastKeyAt: number;
    recording: boolean;
    composing: boolean;
}

export const TYPING_QUIET_MS = 2000;
export const TYPING_MAX_WAIT_MS = 10_000;
export const PERSON_OPENS_PER_SECOND = 5;
export const AGENT_PANES_PER_TAB = 8;
export const AGENT_PANES_IN_ALL = 24;

/** What the executor does to the window. The real ones are `open-deps.ts`; the tests give their own. */
export interface ExecutorDeps {
    now(): number;
    sleep(ms: number): Promise<void>;
    translate(key: string): string;
    /** Runs a person's open as one step of the window's trail of places (Back and Forward walk it); tests may leave it out. */
    gesture?<T>(run: () => Promise<T>): Promise<T>;
    /** A person's open: the window's own `openOrFocusPaneIdByView` (it focuses), or a new or floating pane. */
    openForPerson(plan: PanePlan, how: { newPane: boolean; floating: boolean }): Promise<string | undefined>;
    /** The pane of `view` (and its identity) in `tabId`, or in the shown tab when no tab is named. */
    findPane(plan: PanePlan, tabId?: string): Promise<string | null>;
    setMeta(blockId: string, meta: Record<string, unknown>): Promise<void>;
    /** Show a pane in its tab without moving focus. */
    showWithoutFocus(blockId: string): Promise<void>;
    /** A new pane, without focus: beside `besideBlockId` when given, else in `tabId`. */
    openWithoutFocus(plan: PanePlan, at: { besideBlockId?: string; tabId?: string }): Promise<string | undefined>;
    /** A new pane as a tab of `blockId`'s pane, without focus. */
    stackOnto(plan: PanePlan, blockId: string): Promise<string | undefined>;
    paneFacts(blockId: string): PaneFacts | undefined;
    tabOf(blockId: string): string | undefined;
    tabExists(tabId: string): boolean;
    /** Whether a setting with this id is in this version's Settings. */
    settingExists(id: string): Promise<boolean>;
    /** Whether a Vault entry has this opaque link id. */
    vaultEntryExists(linkId: string): Promise<boolean>;
    /** The locate flash on the pane: the person sees where it went. */
    locate(blockId: string): void;
    /** A polite live region; never moves focus. */
    announce(text: string): void;
    personInput(): PersonInput;
    /** The person's own browser. */
    openExternal(url: string): Promise<void>;
    /** A browser pane, floating or new. */
    openBrowserPane(url: string, floating: boolean): Promise<string | undefined>;
}

const TEXT: Record<OpenReasonCode, string> = {
    invalid_link: "That link isn't a place in AgentMux.",
    not_yet: "That place doesn't open in this version of AgentMux yet.",
    not_allowed: "That place can't be opened from here.",
    not_open: "That tab isn't open.",
    not_on_this_surface: "That place isn't on this surface.",
    feature_off: "That feature is off.",
    would_replace: "The pane you named has work in it, so this opened beside it.",
    another_window: "An agent can't open a place in another window.",
    rate_limited: "Too many links at once: try again in a moment.",
    too_many_panes: "Agents already have too many unused panes open here: close some first.",
    person_typing: "Waiting until you stop typing.",
    prompt_open: "Waiting until you answer the open prompt.",
    would_cover_card: "Waiting: it would cover a card that is waiting for you.",
    app_starting: "AgentMux is still starting.",
    queue_full: "Too many opens are waiting.",
    on_another_computer: "That place is on another computer.",
};

function refused(code: OpenReasonCode, label = ""): OpenOutcome {
    return { outcome: "refused", label, reason: { code, text: TEXT[code] } };
}

/** One validation path: whatever comes in is written as a link and read back (R7, R8). */
function readDestination(dest: DestinationRequest | ParsedDeepLink | string): { ok: true; link: ParsedDeepLink } | { ok: false; code: OpenReasonCode } {
    let text: string;
    if (typeof dest === "string") text = dest;
    else if ("canonical" in dest && typeof dest.canonical === "string") text = dest.canonical;
    else {
        try {
            text = formatLink({ ...dest, values: dest.values ?? {}, params: dest.params ?? {} });
        } catch {
            return { ok: false, code: "invalid_link" };
        }
    }
    const read = parseLink(text);
    if (read.ok === false) return { ok: false, code: read.code === "not_yet" ? "not_yet" : "invalid_link" };
    return { ok: true, link: read.link };
}

/** The executor's own state: the person's recent opens (G11) and the panes agents opened (the caps). */
export class Executor {
    private personOpens: number[] = [];
    private agentPanes = new Map<string, { tabId?: string; agentId: string }>();
    /** A destination being opened for a person: asking for it again while it runs is the same open (§8, a double click). */
    private inFlight = new Map<string, Promise<OpenOutcome>>();

    constructor(private deps: ExecutorDeps) {}

    /** The agent that opened `blockId` through a link, for the replace rules. */
    openedBy(blockId: string): string | undefined {
        return this.agentPanes.get(blockId)?.agentId;
    }

    private label(link: ParsedDeepLink): string {
        return destinationLabel(link, (k) => this.deps.translate(k));
    }

    async open(dest: DestinationRequest | ParsedDeepLink | string, options: OpenOptions): Promise<OpenOutcome> {
        const read = readDestination(dest);
        if (read.ok === false) return refused(read.code);
        const link = read.link;
        const label = this.label(link);
        const plan = planDestination(link, this.deps.now());
        if (isPlanProblem(plan)) return refused("not_yet", label);
        if (options.source.kind === "person") {
            const key = link.canonical + (options.newPane ? "|new" : "") + (options.floating ? "|float" : "");
            const running = this.inFlight.get(key);
            if (running) return running;
            if (!this.takePersonSlot()) return refused("rate_limited", label);
            const run = () => this.openForPerson(plan, label, options);
            const p = (this.deps.gesture ? this.deps.gesture(run) : run()).finally(() => this.inFlight.delete(key));
            this.inFlight.set(key, p);
            return p;
        }
        if (options.source.kind === "agent") return this.openForAgent(link, plan, label, options);
        // The OS handler (build 4) and widgets (build 3) have no opens yet.
        return refused("not_yet", label);
    }

    /** G11: five a second. */
    private takePersonSlot(): boolean {
        const now = this.deps.now();
        this.personOpens = this.personOpens.filter((t) => now - t < 1000);
        if (this.personOpens.length >= PERSON_OPENS_PER_SECOND) return false;
        this.personOpens.push(now);
        return true;
    }

    private async placeOutcome(plan: PanePlan, label: string, blockId: string | undefined, reused: boolean): Promise<OpenOutcome> {
        if (!blockId) return { outcome: "refused", label, reason: { code: "not_open", text: "The pane couldn't be opened." } };
        const missing =
            plan.missing !== undefined ||
            (plan.revealSetting !== undefined && !(await this.deps.settingExists(plan.revealSetting))) ||
            (plan.vaultEntry !== undefined && !(await this.deps.vaultEntryExists(plan.vaultEntry)));
        return { outcome: reused ? "reused" : "opened", label, blockId, ...(missing ? { place: "missing" as const } : {}) };
    }

    private async openForPerson(plan: PanePlan, label: string, options: OpenOptions): Promise<OpenOutcome> {
        const newPane = !!options.newPane || options.target?.kind === "new";
        const floating = !!options.floating || options.target?.kind === "window";
        const before = newPane || floating ? null : await this.deps.findPane(plan);
        const blockId = await this.deps.openForPerson(plan, { newPane, floating });
        return this.placeOutcome(plan, label, blockId, !!before && before === blockId);
    }

    /** Unused panes agents opened, in `tabId` and in all (the caps). A pane that is gone or that the person used no longer counts. */
    private agentPaneCounts(tabId: string | undefined): { tab: number; all: number } {
        let tab = 0;
        for (const [blockId, at] of [...this.agentPanes]) {
            const facts = this.deps.paneFacts(blockId);
            if (!facts || facts.usedByPerson) {
                this.agentPanes.delete(blockId);
                continue;
            }
            if (tabId !== undefined && at.tabId === tabId) tab++;
        }
        return { tab, all: this.agentPanes.size };
    }

    /** Waits while the person is typing, dictating or composing: until 2 s are quiet, at most 10 s. */
    private async waitForQuiet(): Promise<void> {
        const start = this.deps.now();
        for (;;) {
            const input = this.deps.personInput();
            const now = this.deps.now();
            const busy = input.recording || input.composing || now - input.lastKeyAt < TYPING_QUIET_MS;
            if (!busy || now - start >= TYPING_MAX_WAIT_MS) return;
            await this.deps.sleep(250);
        }
    }

    private async reveal(blockId: string, plan: PanePlan): Promise<void> {
        if (Object.keys(plan.focusMeta).length > 0) await this.deps.setMeta(blockId, plan.focusMeta);
        await this.deps.showWithoutFocus(blockId);
    }

    private async openForAgent(link: ParsedDeepLink, plan: PanePlan, label: string, options: OpenOptions): Promise<OpenOutcome> {
        const source = options.source as Extract<OpenSource, { kind: "agent" }>;
        const target: OpenTarget = options.target ?? { kind: "beside" };
        if (target.kind === "window") return refused("another_window", label);
        // G1: a worker view is never started by an agent; build 1 has none, and the parser refuses them as not_yet.
        if (deepLinkView(link.view)?.effect !== "none") return refused("not_allowed", label);
        const ownTab = source.pane ? this.deps.tabOf(source.pane) : undefined;
        if (target.kind === "tab" && target.tabId !== undefined && !this.deps.tabExists(target.tabId)) return refused("not_open", label);
        const tabId = target.kind === "tab" && target.tabId !== undefined ? target.tabId : ownTab;

        await this.waitForQuiet();

        let blockId: string | undefined;
        let reused = false;
        let note: OpenOutcome["reason"];
        if (target.kind === "pane") {
            const facts = this.deps.paneFacts(target.blockId);
            const rule = facts ? replaceRule({ ...facts, openedBy: facts.openedBy ?? this.openedBy(target.blockId) }, plan.view, source) : "never";
            if (rule === "reuse") {
                await this.reveal(target.blockId, plan);
                blockId = target.blockId;
                reused = true;
            } else if (rule === "replace") {
                blockId = await this.newAgentPane(tabId, () => this.deps.stackOnto(plan, target.blockId), source);
                if (!blockId) return refused("too_many_panes", label);
            } else {
                note = { code: "would_replace", text: TEXT.would_replace };
            }
        }
        if (blockId === undefined && target.kind !== "new") {
            const found = await this.deps.findPane(plan, tabId);
            if (found) {
                await this.reveal(found, plan);
                blockId = found;
                reused = true;
            }
        }
        if (blockId === undefined) {
            blockId = await this.newAgentPane(tabId, () => this.deps.openWithoutFocus(plan, { besideBlockId: source.pane, tabId }), source);
            if (!blockId) return refused("too_many_panes", label);
        }
        this.deps.locate(blockId);
        // G13: announced, focus left where it is.
        this.deps.announce(`Opened ${label} beside this conversation`);
        const out = await this.placeOutcome(plan, label, blockId, reused);
        return note && out.outcome !== "refused" ? { ...out, reason: note } : out;
    }

    /** A new pane for an agent, within the caps (8 unused per tab, 24 in all). Undefined when over them. */
    private async newAgentPane(
        tabId: string | undefined,
        make: () => Promise<string | undefined>,
        source: Extract<OpenSource, { kind: "agent" }>
    ): Promise<string | undefined> {
        const counts = this.agentPaneCounts(tabId);
        if (counts.tab >= AGENT_PANES_PER_TAB || counts.all >= AGENT_PANES_IN_ALL) return undefined;
        const blockId = await make();
        if (blockId) this.agentPanes.set(blockId, { tabId, agentId: source.agentId });
        return blockId;
    }

    canOpen(dest: DestinationRequest | string, source: OpenSource): CanOpen {
        const read = readDestination(dest);
        if (read.ok === false) {
            const code: OpenReasonCode = read.code;
            return { ok: false, code, text: TEXT[code] };
        }
        const label = this.label(read.link);
        if (isPlanProblem(planDestination(read.link, 0))) return { ok: false, code: "not_yet", text: TEXT.not_yet };
        if (source.kind === "os" || source.kind === "widget") return { ok: false, code: "not_yet", text: TEXT.not_yet };
        return { ok: true, label };
    }

    wrappers(): OpenWrappers {
        return {
            openTask: (task, options) =>
                this.open(task.board ? { view: "kanban", place: "board-task", values: { board: task.board, task: task.number } } : { view: "kanban", place: "task", values: { task: task.number } }, options),
            openFile: (file, options) =>
                this.open({ view: "editor", place: "file", values: { path: file.path }, params: file.line !== undefined ? { line: file.line } : {} }, options),
            openUrl: async (url, options) => {
                if (!/^https?:\/\//i.test(url)) return refused("invalid_link");
                if (options.source.kind === "person" && !this.takePersonSlot()) return refused("rate_limited", url);
                if (options.floating || options.newPane) {
                    const blockId = await this.deps.openBrowserPane(url, !!options.floating);
                    return blockId ? { outcome: "opened", label: url, blockId } : refused("not_open", url);
                }
                await this.deps.openExternal(url);
                return { outcome: "opened", label: url };
            },
            // Build 2.
            openAgent: async () => refused("not_yet"),
            openTab: async () => refused("not_yet"),
            canOpen: (dest, source) => this.canOpen(dest, source),
        };
    }
}

let shared: Executor | undefined;

async function executor(): Promise<Executor> {
    if (!shared) {
        const { windowDeps } = await import("./open-deps");
        shared = new Executor(windowDeps());
    }
    return shared;
}

/** The executor (R11a). */
export async function openDestination(dest: DestinationRequest | ParsedDeepLink | string, options: OpenOptions): Promise<OpenOutcome> {
    return (await executor()).open(dest, options);
}

/** srv's request for an agent's open, after its guard (1c): the link, where, and who. */
export interface AgentOpenRequest {
    link: string;
    target?: OpenTarget;
    source: Extract<OpenSource, { kind: "agent" }>;
    requestId: string;
}

/** The window's side of an agent's open: what srv carries to the app, answered with the outcome. */
export async function handleAgentOpen(req: AgentOpenRequest): Promise<OpenOutcome> {
    return openDestination(req.link, { source: { ...req.source, requestId: req.requestId }, target: req.target });
}

/** The typed wrappers (R11a), each one `openDestination`. */
export async function openWrappers(): Promise<OpenWrappers> {
    return (await executor()).wrappers();
}
