// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The one label for "show this in the computer's own file manager", named for the platform it runs on. */

import { isMacOS, isWindows } from "@/util/platformutil";

/** What the action says: the platform's own file manager by name. Linux has several, so it stays generic there. */
export function revealInFileManagerLabel(): string {
    if (isMacOS()) return "Reveal in Finder";
    if (isWindows()) return "Reveal in File Explorer";
    return "Reveal in file manager";
}
