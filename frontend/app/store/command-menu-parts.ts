// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// What a menu item for a registry command shows and does, read from the registry by id: the name
// from the command, the key from the shortcut table, the action from the command's own `execute`.
// A menu keeps its own icon and place; it no longer keeps its own copy of the name, the key or the
// action, so renaming a command or rebinding its key reaches the menu too.

import { shortcutFor } from "@/app/keybindings";
import { commandRegistry } from "@/app/store/command-registry";

export interface CommandMenuParts {
    label: string;
    /** The key in this platform's notation; "" when the command has none. Read when asked, from the shortcut table. */
    readonly shortcut: string;
    /** Runs the command as the palette does; declines it where the host or a condition says so. */
    run: () => void;
}

/** The label, key and action of the registry command `id`. Throws on an id that is not registered. */
export function commandMenuParts(id: string): CommandMenuParts {
    const command = commandRegistry.get(id);
    if (command == null) throw new Error(`menu item for an unregistered command: ${id}`);
    return {
        label: command.label,
        get shortcut() {
            return shortcutFor(id);
        },
        run: () => void commandRegistry.run(id),
    };
}
