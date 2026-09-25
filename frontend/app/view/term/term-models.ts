// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Every live terminal model, by block id. Terminal code finds other
 * terminals (multi-input) and its pane chrome finds the active tab's model
 * here, rather than casting the host's view model to `TermViewModel` —
 * which a native pane tab's adapter isn't (Pane Tab contract Phase 2c).
 */

import { createKeyedRegistry } from "@/util/keyed-registry";
import type { TermViewModel } from "./termViewModel";

export const termModels = createKeyedRegistry<TermViewModel>();

/** Terminals running a shell (not a `cmd` controller): multi-input's set. */
export function basicTermModels(): TermViewModel[] {
    return termModels.all().filter((m) => m.isBasicTerm());
}
