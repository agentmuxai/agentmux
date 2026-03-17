// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { SwarmViewModel } from "./swarm-model";
import { SwarmView } from "./swarm-view";

Object.defineProperty(SwarmViewModel.prototype, "viewComponent", {
    get() {
        return SwarmView;
    },
});

export { SwarmViewModel };
