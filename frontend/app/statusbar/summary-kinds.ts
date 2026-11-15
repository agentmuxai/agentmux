// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Where every kind of call to action srv announces (`userattention`, crates/srv/src/backend/user_attention.rs `KINDS`)
 * lands in the status bar's summary, and where a request that belongs to no pane is answered
 * (docs/specs/SPEC_STATUS_BAR_COMPACT_SUMMARY_AND_VAULT_REQUESTS_2026_11_15.md §1b, §2). A tone must have a row the
 * person can find, so a kind with no entry here fails the test that reads srv's list.
 */

/** The parts of the summary, left to right. */
export type PartId = "agents" | "asks" | "browser" | "vault" | "tasks";

/** Where a request with no pane of its own is answered. */
export type AnswerPlace = "pane" | "connections" | "widgets" | "vault" | "upgrade" | "desktop";

export interface KindPlace {
    /** The part its rows are in. `vault` rows come from the Vault's own lists, so a pane's announcement of one is not counted twice. */
    part: "asks" | "browser" | "vault";
    /** Where to answer it when it has no pane. */
    answer: AnswerPlace;
}

export const KIND_PLACES: Record<string, KindPlace> = {
    browser: { part: "browser", answer: "pane" },
    consent: { part: "asks", answer: "connections" },
    confirm: { part: "asks", answer: "pane" },
    vault: { part: "vault", answer: "vault" },
    widget: { part: "asks", answer: "widgets" },
    upgrade: { part: "asks", answer: "upgrade" },
    desktop: { part: "asks", answer: "desktop" },
};
