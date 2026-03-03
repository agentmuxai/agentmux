// Copyright 2024, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * Unified Agent Widget - Main exports
 */

export { AgentViewModel } from "./agent-model";
export { AgentView } from "./agent-view";
export { ClaudeCodeStreamParser } from "./stream-parser";
export * from "./types";
export * from "./state";

// Provider exports
export { PROVIDERS, getProvider, getProviderList } from "./providers";
export { createTranslator } from "./providers/translator-factory";
export type { OutputTranslator } from "./providers/translator";
export type { ProviderDefinition } from "./providers";
