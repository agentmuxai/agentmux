// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The shapes of the recognition layer (docs/specs/SPEC_LINKS_ON_EVERYTHING_2026_11_20.md): text goes in, segments come
 * out, and a link's destination is built from the entity a recogniser found, never from the characters written.
 */

export type EntityKindId = "address" | "pull" | "task" | "agent";

/** Where a click goes. Built by a kind from the entity it resolved; the text never supplies one. */
export type Destination =
    | { kind: "agent"; id: string; name: string; computer?: string }
    | { kind: "task"; number: number; board?: string }
    | { kind: "pull"; repo: string; number: number; issue: boolean }
    | { kind: "url"; url: string };

export type LinkState = "ready" | "unchecked" | "dead";

export type Segment =
    | { type: "text"; text: string }
    | {
          type: "link";
          kind: EntityKindId;
          /** The characters as written, so copying a linkified text copies the text. */
          label: string;
          destination: Destination;
          state: LinkState;
          /** What a click does, as a screen reader says it ("Open task 1234"). */
          name: string;
          /** Why a dead link is dead. Never names a host path. */
          reason?: string;
      };

/** Who wrote the text, and where. An app-written text has no author and so resolves no file. */
export type Author =
    | { kind: "app" }
    | { kind: "operator" }
    | { kind: "agent"; id: string; confined: boolean; computer?: string; remote?: boolean };

export type AgentEntry = { id: string; slug: string; name: string; computer?: string };

export type TaskEntry = { board: string; number: number; title: string; state: string };

/** The roster, looked up by the written name. Built once per roster change; see indexes.ts. */
export type AgentIndex = {
    version: number;
    /** The agent written as `written` (exact display name, or slug or id in any case), if any. */
    find(written: string): AgentEntry | undefined;
    /** Whether `written` is one of the names that link only with their computer (D3). */
    neverAlone(written: string): boolean;
    /** First characters any name starts with, to skip most words without a lookup. */
    startsWith(char: string): boolean;
    localComputer?: string;
};

/** The tasks the viewer can see, by board and number. Built once per board refresh; see indexes.ts. */
export type TaskIndex = {
    version: number;
    defaultBoard: string;
    hasBoard(slug: string): boolean;
    get(board: string, number: number): TaskEntry | undefined;
};

export type LinkContext = {
    author: Author;
    agents: AgentIndex;
    tasks: TaskIndex;
    /** The repository the surface knows (`owner/repo`): a task card's, a pane's working one. Never read from the text. */
    repo?: string;
    /** The board the surface belongs to, for a bare `#N`. */
    board?: string;
    /** False while the text is still arriving: a match that touches the end is not linked yet. */
    final: boolean;
};

export type Range = { start: number; end: number };

/** What a kind's recogniser found: a range of the text and what it read there. */
export type Candidate = Range & {
    kind: EntityKindId;
    data: Record<string, string>;
    /** Names its kind outright (`PR 5`, `task 5`): with no entity behind it, it still keeps lower candidates out of its words. */
    claims?: boolean;
};

export type Resolved = {
    /** The kind the link is drawn as, when it is not the recogniser's own (a bare `#N` that is a pull request). */
    kind?: EntityKindId;
    destination: Destination;
    state: LinkState;
    name: string;
    reason?: string;
};

export type OpenResult = { ok: true } | { ok: false; reason: string };

export type OpenOptions = { floating?: boolean };

export type EntityKind = {
    id: EntityKindId;
    /** Overlaps are settled by priority, then by the longer match, then the earlier. */
    priority: number;
    recognise(text: string, ctx: LinkContext): Candidate[];
    /** No entity, no link: null. */
    resolve(candidate: Candidate, ctx: LinkContext): Resolved | null;
    /** Opens the destination through the deep-links framework's wrapper, nothing else. */
    open(destination: Destination, options: OpenOptions): Promise<OpenResult>;
};

/** One line of the debug view: what the recogniser saw and why it did or did not make a link. */
export type TraceLine = { kind: EntityKindId; start: number; end: number; text: string; outcome: string };
