// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Block meta keys for an agent's title and suggestion, read and written in
 * several places on both sides. Block meta is untyped, so a mistyped key would
 * compile and silently read nothing; these names are the one spelling.
 * Keys owned by one module stay with it (`swarm-line.ts`, `title-schedule.ts`).
 */

/** The session title. Written by the backend only (`crates/srv/src/ambient/title.rs`,
 *  `META_TITLE`); cleared here when the session ends (useBlockActivity.ts). */
export const META_TITLE = "term:ambient_summary";

/** When the title was stored or last confirmed as still right (epoch ms), and the human turn it
 *  was judged at. Written by the backend with the title (`ambient/title.rs`); `0` means "judge it
 *  again at the next message" (`markTitleStale`). The pane ages a title by them (`title-schedule.ts`). */
export const META_TITLE_AT = "term:ambient_summary_at";
export const META_TITLE_TURN = "term:ambient_summary_turn";

/** The terminal's own title (OSC 0/2), the free fallback when there is no session title. */
export const META_OSC_TITLE = "term:osc_title";

/** The composer's next-message suggestion (useNextPromptSuggestion.ts). */
export const META_SUGGESTION = "term:next_prompt_suggestion";

/** Bumped with every write of `META_SUGGESTION`, so a send can mask one write
 *  rather than one text value (AgentFooter.tsx). */
export const META_SUGGESTION_GEN = "term:next_prompt_suggestion_gen";
