// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Memory module barrel — wires the view component onto the model
// prototype to avoid a circular import between bundle-model.ts and
// bundle-view.tsx.

import { BundleViewModel } from "./bundle-model";
import { BundleView } from "./bundle-view";

Object.defineProperty(BundleViewModel.prototype, "viewComponent", {
    get() {
        return BundleView;
    },
});

export { BundleViewModel };
