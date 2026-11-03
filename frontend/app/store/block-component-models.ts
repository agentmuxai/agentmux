// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Each mounted block's component model, by block id. block-component-registry.ts owns it (register,
 * unregister, dormancy); this module only holds the map, so focus code can read a block's view without
 * importing the registry, which opens panes and so reaches the focus code in turn.
 */
export const blockComponentModelMap = new Map<string, BlockComponentModel>();
