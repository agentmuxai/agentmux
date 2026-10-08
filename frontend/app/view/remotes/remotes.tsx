// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Remotes was a pane of its own until it became a section of Connectors
// (docs/specs/SPEC_REMOTES_INTO_CONNECTORS_2026_10_08.md). A block saved as
// `view: "remotes"` still loads through this manifest, which rewrites it to
// Connectors on its Remotes section (keeping `remotes:expand`), so it loads as
// Connectors from then on. Until the block remounts it shows Connectors from
// here: a block's body doesn't follow a `meta.view` change while it's mounted.

import type { PaneTabManifest } from "@/app/block/pane-tab-registry";
import { connectorsPaneTab } from "@/app/view/connectors/connectors";
import { CONNECTORS_SECTION_KEY, CONNECTORS_VIEW, LEGACY_REMOTES_VIEW } from "@/app/view/section-pane/panes";

/** The meta patch that turns a saved Remotes block into Connectors → Remotes. */
export const REMOTES_MIGRATION_PATCH = { view: CONNECTORS_VIEW, [CONNECTORS_SECTION_KEY]: "remotes" } as const;

export const remotesPaneTab: PaneTabManifest = {
    apiVersion: 1,
    view: LEGACY_REMOTES_VIEW,
    label: "Remotes",
    icon: "server",
    legacyOf: CONNECTORS_VIEW,
    capabilities: { paneZoom: {} },
    create: (ctx) => {
        // After create() returns: the host is still building this instance.
        queueMicrotask(() => void ctx.setMeta({ ...REMOTES_MIGRATION_PATCH }));
        // On Remotes before the patch lands, too; a section picked after it is the user's.
        const meta = () => {
            const m = ctx.meta();
            return { ...m, [CONNECTORS_SECTION_KEY]: m?.[CONNECTORS_SECTION_KEY] ?? "remotes" } as MetaType;
        };
        return connectorsPaneTab.create({ ...ctx, meta });
    },
};
