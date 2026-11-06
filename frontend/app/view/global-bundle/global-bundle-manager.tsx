// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// GlobalBundleManager — the Memory pane's Global section (Global Memory).
// Presents the workspace-wide global bundles (is_global rows) as an ordered
// set of memories that compose into every agent's startup instructions file
// (CLAUDE.md, GEMINI.md, or similar, depending on provider) at launch.
//
// Context-free: owns its own GlobalBundleViewModel and drives off the
// bundle_* RPCs. Spec: docs/specs/archive/SPEC_TRUST_CENTER_GLOBAL_BRAIN_2026_06_19.md.
//
// Layout restructured three times: per docs/specs/SPEC_ARMORY_GLOBAL_MEMORY_
// DECLUTTER_2026_09_15.md (one consistently-shaped file list), per
// docs/specs/SPEC_GLOBAL_MEMORY_UNIFY_SYSTEM_AND_ORDINARY_2026_09_15.md (the
// system tier and ordinary entries render as one list — the backend's
// write-path isolation between them is real and unchanged; only the
// presentation unified), and — since 2026-09-24 — per
// docs/specs/SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.3: TILES FIRST,
// then expand to full, matching an agent's Personal Memory file tiles
// (MemoryTile, the same grid and tile CSS). The always-visible 240px preview
// per card and the inline card editors are gone; a tile opens
// GlobalMemoryFullView with the Personal Memory back/breadcrumb header. The
// full view has history (globalmemory:history/diff/revert), an editor pinned
// to the bottom, and dirty-draft protection. Creating a NEW system-tier entry
// still has no UI trigger; existing ones remain editable/removable via their
// own still-isolated RPCs.

import { type JSX, Show } from "solid-js";

import { FullViewPane, MemoryGrid } from "./global-bundle-manager-parts";
import { createGlobalBundleManager } from "./global-bundle-manager-state";
import "@/app/view/native-memory/native-memory-manager.scss";
import "./global-bundle.scss";

export const GlobalBundleManager = (): JSX.Element => {
    const s = createGlobalBundleManager();

    // Built once and shown again each time a full view is closed, as the tiles keep their own state.
    const grid = <MemoryGrid s={s} />;

    return (
        <div class="global-bundle">
            <s.confirm.Dialog />
            <Show when={s.model.viewAtom()} fallback={grid}>
                {(view) => <FullViewPane s={s} view={view} />}
            </Show>
        </div>
    );
};

GlobalBundleManager.displayName = "GlobalBundleManager";
