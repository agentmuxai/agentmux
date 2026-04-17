// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Browser module barrel — wires viewComponent onto BrowserViewModel.

import { BrowserViewModel } from "./browser-model";
import { BrowserViewComponent } from "./browser-view";

Object.defineProperty(BrowserViewModel.prototype, "viewComponent", {
    get() {
        return BrowserViewComponent;
    },
});

export { BrowserViewModel };
