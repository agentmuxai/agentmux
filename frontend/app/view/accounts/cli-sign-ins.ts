// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The agent CLIs whose own sign-in Connectors runs (ProviderLoginPanel), by provider id, with what the panel calls the
 * sign-in on screen. A leaf with no imports: the tile catalog and the account state both read it, and neither can import
 * the other without a cycle.
 */
export const CLI_SIGN_INS: Record<string, string> = {
    claude: "Anthropic",
    codex: "ChatGPT",
};
