// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// A Browser pane's instance, loaded with the first one (browser.tsx).

import type { PaneTabHostContext, PaneTabInstance } from "@/app/block/pane-tab-registry";
import { BrowserViewModel } from "./browser-model";
import { BrowserViewComponent } from "./browser-view";

export function create(ctx: PaneTabHostContext): PaneTabInstance {
    const model = new BrowserViewModel(ctx);
    return {
        component: () => <BrowserViewComponent model={model} />,
        // Asleep from launch, the page never loaded: its saved title stands in
        // (SPEC_BROWSER_PANE_HIBERNATION_2026_10_20.md).
        liveTitle: () => {
            const asleep = model.sleepAtom();
            if (asleep?.title && model.viewNameIsPlaceholder()) return { text: asleep.title, placeholder: false };
            return { text: model.viewName(), placeholder: model.viewNameIsPlaceholder() };
        },
        liveFavicon: () => model.viewFaviconUrl(),
        contextMenu: (browserCtx) => model.getBodyContextMenuItems(browserCtx as never),
        focus: () => model.giveFocus(),
        dispose: () => model.dispose(),
    };
}
