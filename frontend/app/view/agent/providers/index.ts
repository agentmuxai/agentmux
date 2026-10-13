// Copyright 2025, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Barrel: this module used to bundle everything (type contracts, the static
// per-CLI catalog, and the dynamic model-catalog overlay) in one file. It's
// now split for readability — see ./types, ./catalog, ./model-overlay — but
// every external consumer keeps importing from "providers"/"providers/index"
// unchanged.

export type { ProviderModel, ProviderDefinition } from "./types";

export { PROVIDERS, resolveProviderAlias } from "./catalog";

export { setProviderModels, getProvider, getProviderList, familyKey } from "./model-overlay";
