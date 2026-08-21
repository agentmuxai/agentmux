// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Single source of truth for each gain chain's default value — used both
 * as the player class's initial gain (before any settings effect has run)
 * and as the settings-read fallback (once a setting exists but hasn't been
 * explicitly set by the user). Before this module existed, each of these
 * three numbers was duplicated as a literal across sound-player.ts,
 * tool-tones-player.ts, waiting-tone-player.ts, sound-service.ts (multiple
 * call sites), and sounds-section.tsx — a real drift risk (a future
 * default change would need to touch 5+ scattered literals and is easy to
 * miss one of). `schema/settings.json`'s own `"default"` field and
 * `agentmux-srv`'s Rust doc comment are necessarily separate (different
 * language/build), but should be kept in sync with these by hand.
 */
export const DEFAULT_MASTER_VOLUME = 0.6;
export const DEFAULT_TOOLTONES_VOLUME = 0.25;
export const DEFAULT_WAITING_VOLUME = 0.25;
