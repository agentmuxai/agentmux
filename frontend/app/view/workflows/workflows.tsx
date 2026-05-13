// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Workflows module barrel — wires the view component onto the model
// prototype to avoid a circular import.

import { WorkflowsViewModel } from "./workflows-model";
import { WorkflowsView } from "./workflows-view";

Object.defineProperty(WorkflowsViewModel.prototype, "viewComponent", {
    get() {
        return WorkflowsView;
    },
});

export { WorkflowsViewModel };
