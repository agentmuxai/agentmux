// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Claude sign-in panel's props, and the provider it signs in to. */

import type { ModalScope } from "@/element/modal";
import { PROVIDERS } from "@/app/view/agent/providers/catalog";

export const CLAUDE_PROVIDER = PROVIDERS["claude"];

export interface ClaudeLoginProps {
    onClose: () => void;
    /** Set for a per-binding re-login: refreshes THIS account's
     *  isolated dir instead of minting a new one, and links the result back
     *  to the agent. Omit for Connectors' bare Connect (fresh account, no
     *  agent to link yet — the account just needs to exist for a later
     *  launch or a bind in Connectors). */
    existingAccountId?: string;
    linkTarget?: { blockId?: string; agentDefinitionId: string };
    /** Called with the new account's id once it is saved and the sign-in succeeded (the composer's
     *  "Sign in with another account…" switches the agent to it). */
    onSignedIn?: (accountId: string) => void;
    /** Set when the row this panel was opened from carries a legacy-alias
     *  provider string (e.g. "claude-code") — a successful login here links
     *  the CANONICAL "claude" provider (finalizeAccount → agent_identity_link
     *  `ON CONFLICT(agent_id, provider)`), which is a DIFFERENT key than the
     *  alias row, so it INSERTS a second link instead of replacing the broken
     *  one. The old alias row (pointing at a deleted/orphaned account) is left
     *  behind — the resolver's `inject.rs` iterates ALL of an agent's bindings
     *  ORDER BY provider and aborts the ENTIRE spawn on the first one that
     *  fails to resolve, so the orphaned alias row (sorting after "claude")
     *  silently blocks every future spawn even though the healthy "claude"
     *  link now exists — "✓ Signed in" but the agent stays broken. On
     *  success, this panel unlinks `staleAliasProvider` for the same
     *  agent so only the canonical row remains (#2414). */
    staleAliasProvider?: string;
    /** Which region the dialog locks — see `Modal`'s `scope` prop. Connectors
     *  (no ModalLayer ancestor) passes "tab"; a caller already inside a
     *  pane's `ModalLayer scope="pane"` passes "pane" so this stacks
     *  correctly over that modal instead of both competing for the same
     *  window-level backdrop. */
    scope?: ModalScope;
    /** Show the sign-in in the section it is in, not in a dialog. */
    inline?: boolean;
}
