// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Drone module barrel — wires the view component onto the model
// prototype to avoid a circular import.

import { DroneViewModel } from "./drone-model";
import { DroneView } from "./drone-view";

Object.defineProperty(DroneViewModel.prototype, "viewComponent", {
    get() {
        return DroneView;
    },
});

export { DroneViewModel };
