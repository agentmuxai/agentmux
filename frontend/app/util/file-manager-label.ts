// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The one label for "show this in the computer's own file manager", named for the platform it runs on. */

import { isMacOS, isWindows } from "@/util/platformutil";

/** What the thing shown is: a file is selected in its folder, a folder is opened. */
export type RevealKind = "file" | "folder";

/**
 * What the action says: the platform's own file manager by name. Linux file managers can reliably open the
 * folder that holds a file (selecting it where the desktop's file manager supports it), so Linux says that,
 * the name VS Code uses there; for a folder itself it says "Open in File Manager".
 */
export function revealInFileManagerLabel(kind: RevealKind = "file"): string {
    if (isMacOS()) return "Reveal in Finder";
    if (isWindows()) return "Reveal in File Explorer";
    return kind === "folder" ? "Open in File Manager" : "Open Containing Folder";
}
