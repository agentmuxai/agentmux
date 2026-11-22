// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The executor's contract (docs/specs/SPEC_DEEP_LINKS_ONE_FRAMEWORK_2026_11_20.md R1 to R5, R11a, §4.2). Types only:
 * `openDestination` and the typed wrappers implement it (PR 1b), srv's `OpenDestination` request carries the same
 * shapes (PR 1c), and the links recognition layer (links/entities/openers.ts) calls the wrappers.
 */

import type { DeepLinkValue, ParsedDeepLink } from "./parse";

/** Where a destination shows (R2). Only MuxAPI names one; a link never does. */
export type OpenTarget =
    /** Next to the caller's own conversation (an agent), or next to the focused pane (a person). The default. */
    | { kind: "beside" }
    | { kind: "pane"; blockId: string }
    | { kind: "new" }
    | { kind: "tab"; tabId?: string }
    | { kind: "window"; windowId?: string };

/** Who asked, which decides the rules the executor applies (G3, G4, G9). srv's guard has already applied an
 *  agent's limits before an `agent` open reaches the app. */
export type OpenSource =
    /** A person's own click or key, in the app's own text or a reply they are reading (G4a, G4b). */
    | { kind: "person"; via: "click" | "keyboard" | "palette" | "toast" | "notice" }
    /** An agent's MuxAPI open, carried by srv after its guard (G3, G9). `pane` is the caller's own conversation. */
    | { kind: "agent"; agentId: string; pane?: string; requestId: string }
    /** The operating system's handler (build 4, G4e). */
    | { kind: "os" }
    /** A sandboxed widget with the permission for this place (G4f). */
    | { kind: "widget"; packageId: string };

/** What to open: a parsed link, or the same fields named directly (`Pane` open with `view`, `place`, `params`). */
export interface DestinationRequest {
    view: string;
    place?: string | null;
    values?: Record<string, DeepLinkValue>;
    params?: Record<string, DeepLinkValue>;
    anchor?: string | null;
}

export interface OpenOptions {
    target?: OpenTarget;
    source: OpenSource;
    /** Ctrl or Cmd with a click: a floating pane (R22). */
    floating?: boolean;
    /** A middle click: a new pane (R22). */
    newPane?: boolean;
}

export type OpenOutcomeKind = "opened" | "reused" | "queued" | "refused";

/** Why an open is refused or queued, as a code the caller can act on, and the sentence it shows. */
export type OpenReasonCode =
    | "invalid_link"
    | "not_yet"
    | "not_allowed"
    | "not_open"
    | "not_on_this_surface"
    | "feature_off"
    | "would_replace"
    | "another_window"
    | "rate_limited"
    | "too_many_panes"
    | "person_typing"
    | "prompt_open"
    | "would_cover_card"
    | "app_starting"
    | "queue_full"
    | "on_another_computer";

/** The one answer every open gets (R3, G9). */
export interface OpenOutcome {
    outcome: OpenOutcomeKind;
    /** The words for where it went, from the registry (R10): "Settings › Recording". */
    label: string;
    /** The pane it landed in, when it landed. */
    blockId?: string;
    /** The view opened but the place inside it is gone (§8): the pane says why in its own notice. */
    place?: "missing";
    reason?: { code: OpenReasonCode; text: string };
    /** What would work instead, when there is something: a link to the setting that turns the feature on. */
    remedy?: { text: string; link?: string };
}

/** A cheap, side-effect-free answer to "would this open", so a link can be drawn muted (links spec §12). */
export type CanOpen = { ok: true; label: string } | { ok: false; code: OpenReasonCode; text: string };

/** The executor (R11a). Implemented in PR 1b by `open.ts`. */
export type OpenDestination = (dest: DestinationRequest | ParsedDeepLink | string, options: OpenOptions) => Promise<OpenOutcome>;

/** The typed wrappers for what people name most (R11a), each one `openDestination` call. */
export interface OpenWrappers {
    openTask(task: { number: number; board?: string }, options: OpenOptions): Promise<OpenOutcome>;
    /** `author` is passed to srv, which resolves a confined author's file only in its own folders (links spec §6.4). */
    openFile(file: { path: string; line?: number; author?: { agentId: string; confined: boolean } }, options: OpenOptions): Promise<OpenOutcome>;
    /** The person's browser, or a browser pane when `floating` or `newPane` (links spec D1). */
    openUrl(url: string, options: OpenOptions): Promise<OpenOutcome>;
    /** Build 2. Reveal-only, except a person's click on an agent the app draws (G1's stated exception). */
    openAgent(agent: { id: string; computer?: string; drawnByApp: boolean }, options: OpenOptions): Promise<OpenOutcome>;
    /** Build 2. */
    openTab(tab: { tabId: string }, options: OpenOptions): Promise<OpenOutcome>;
    canOpen(dest: DestinationRequest | string, source: OpenSource): CanOpen;
}
