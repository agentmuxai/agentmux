// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Browser module barrel — wires viewComponent onto BrowserViewModel.

import type { PaneTabManifest } from "@/app/block/pane-tab-registry";
import { BrowserViewModel } from "./browser-model";
import { BrowserViewComponent } from "./browser-view";

/** The browser as a native pane tab (Pane Tab contract Phase 2c). */
export const browserPaneTab: PaneTabManifest = {
    apiVersion: 1,
    view: "browser",
    label: "Browser",
    icon: "globe",
    // Keep-alive per the repo owner's decision (SPEC_PANE_TAB_CONTRACT_V1 §5):
    // remounting reloads the page. Its page is a native surface, collapsed
    // whenever the tab isn't visible.
    capabilities: { lifecycle: "keepAlive", nativeSurface: true, noPadding: true },
    create: (ctx) => {
        const model = new BrowserViewModel(ctx);
        return {
            component: () => <BrowserViewComponent model={model} />,
            liveTitle: () => ({ text: model.viewName(), placeholder: model.viewNameIsPlaceholder() }),
            liveFavicon: () => model.viewFaviconUrl(),
            contextMenu: (browserCtx) => model.getBodyContextMenuItems(browserCtx as never),
            focus: () => model.giveFocus(),
            dispose: () => model.dispose(),
        };
    },
};

export { BrowserViewModel };
