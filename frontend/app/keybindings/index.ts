// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Shortcut labels for the current platform. Every place the UI shows a key
// uses these, so a hint can't disagree with the binding.

import { isMacOS } from "@/util/platformutil";
import { helpSections, type HelpSection } from "./help";
import { formatKey, type KeyPlatform } from "./keys";
import { formatCommand } from "./registry";

export function keyPlatform(): KeyPlatform {
    return isMacOS() ? "mac" : "other";
}

/** The shortcut for a command ("Ctrl+Shift+T", "⌘T"), or "" if it has none. */
export function shortcutFor(command: string): string {
    return formatCommand(command, keyPlatform());
}

/** A component-local key in this platform's notation: `keyLabel("mod+s")`. */
export function keyLabel(spec: string): string {
    return formatKey(spec, keyPlatform());
}

export function shortcutHelp(): HelpSection[] {
    return helpSections(keyPlatform());
}
