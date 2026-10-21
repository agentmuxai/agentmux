// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// An Editor pane's instance, loaded with the first one (editor.tsx).

import type { PaneTabHostContext, PaneTabInstance } from "@/app/block/pane-tab-registry";
import { registerDocTabHost } from "@/app/doc-tabs/doc-tab-hosts";
import { editorDocTabHost } from "./editor-doc-tabs";
import { EditorViewModel } from "./editor-model";
import { EditorViewComponent } from "./editor-view";

export function create(ctx: PaneTabHostContext): PaneTabInstance {
    const model = new EditorViewModel(ctx);
    // Its tabs can move to and from other Editor panes (editor-doc-tabs.ts).
    const unregisterHost = registerDocTabHost(ctx.blockId, editorDocTabHost(model));
    return {
        component: () => <EditorViewComponent model={model} />,
        liveTitle: () => ({ text: model.viewName() }),
        headerText: () => model.viewText(),
        contextMenu: () => model.getBodyContextMenuItems(),
        focus: () => model.giveFocus(),
        dispose: () => {
            unregisterHost();
            model.dispose();
        },
    };
}
