// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Hands the layout model's constructor to layoutModelHooks. Imported once, for this effect, by app-init.

import { LayoutModel } from "./layoutModel";
import { setLayoutModelFactory } from "./layoutModelHooks";

setLayoutModelFactory((tabAtom) => new LayoutModel(tabAtom));
