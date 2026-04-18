// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Editor module barrel — wires viewComponent onto EditorViewModel.

import { EditorViewModel } from "./editor-model";
import { EditorViewComponent } from "./editor-view";

Object.defineProperty(EditorViewModel.prototype, "viewComponent", {
    get() {
        return EditorViewComponent;
    },
});

export { EditorViewModel };
