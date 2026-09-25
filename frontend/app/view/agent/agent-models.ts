// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Every live agent model, by block id. The agent pane chrome finds the
 * active tab's model here (its agent definitions, for fork tabs) rather than
 * casting the host's view model to `AgentViewModel` — which a native pane
 * tab's adapter isn't (Pane Tab contract Phase 2c).
 */

import { createKeyedRegistry } from "@/util/keyed-registry";
import type { AgentViewModel } from "./agent-model";

export const agentModels = createKeyedRegistry<AgentViewModel>();
