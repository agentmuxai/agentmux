// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Vault as Warden's Vault section (operator decision 2026-10-21,
 * docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §5.6). The Vault keeps its
 * model and view; this gives it Warden's block as its host, with two differences that keep its
 * protections tied to the section rather than the pane:
 * - it is shown (the on-screen note that blocks window captures, an open Reveal) only while
 *   Warden is on screen **on its Vault section**, since Warden keeps its sections mounted;
 * - it has no zoom of its own: Warden zooms the whole pane.
 */

import type { Accessor } from "solid-js";

import type { PaneTabHostContext } from "@/app/block/pane-tab-registry";

/** The pane zoom key (zoom-factor.ts), which Warden applies to the whole pane. */
const ZOOM_META_KEY = "term:zoom";

export function vaultHostIn(warden: PaneTabHostContext, onVault: Accessor<boolean>): PaneTabHostContext {
    return {
        blockId: warden.blockId,
        meta: () => {
            const m = warden.meta();
            if (!m || !(ZOOM_META_KEY in m)) return m;
            const { [ZOOM_META_KEY]: _zoom, ...rest } = m as Record<string, unknown>;
            return rest as MetaType;
        },
        setMeta: (patch) => warden.setMeta(patch),
        isFocused: warden.isFocused,
        visibility: () => (onVault() ? warden.visibility() : "dormant"),
        splitterDragging: warden.splitterDragging,
    };
}
