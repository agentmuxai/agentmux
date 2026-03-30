// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Drone module barrel — wires viewComponent onto the model prototype
// to avoid circular imports between drone-model.ts and drone-view.tsx.

import { DroneViewModel } from "./drone-model";
import { DroneView } from "./drone-view";

Object.defineProperty(DroneViewModel.prototype, "viewComponent", {
    get() {
        return DroneView;
    },
});

export { DroneViewModel };
