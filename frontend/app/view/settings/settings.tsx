// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Settings module barrel — wires viewComponent to avoid circular import.

import { SettingsViewModel } from "./settings-model";
import { SettingsView } from "./settings-view";

Object.defineProperty(SettingsViewModel.prototype, "viewComponent", {
    get() {
        return SettingsView;
    },
});

export { SettingsViewModel };
