// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What a view inside a pane needs to make a colour of its own read on the pane's backdrop: an agent's name colour in a
 * Kanban chip or the Files places list is chosen for the window's theme, and a backdrop of the other side (a White
 * one in a dark theme) leaves it unreadable. The pane's frame provides its resolved backdrop; `useFitToPane` gives a
 * view the function that moves a colour, in lightness only, to read on the pane's three layers (`identityInPane`).
 * A pane with no backdrop gets the colour back as it was.
 */

import { createContext, useContext, type Accessor } from "solid-js";
import type { ResolvedBackdrop } from "./meta";
import { identityInPane } from "./scope";

export const PaneBackdropContext = createContext<Accessor<ResolvedBackdrop | undefined>>(() => undefined);

/** A colour as it reads in the pane this is called in. Call it where the component is made, then use the result in JSX. */
export function useFitToPane(): <T extends string | undefined>(hex: T) => T {
    const backdrop = useContext(PaneBackdropContext);
    return ((hex) => {
        const d = backdrop()?.derived;
        return d && hex ? identityInPane(hex, d) : hex;
    }) as <T extends string | undefined>(hex: T) => T;
}
