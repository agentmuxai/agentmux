// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the pane's Backdrop menu shows and writes (backdrop-menu.ts): the 14 colours for a side, each as the surface a
 * choice would make, and the record a choice writes (spec §9, §10).
 */

import { BUILTINS, deriveBuiltin, type AppScheme } from "./builtins";
import { BACKDROP_VERSION, type BackdropMeta, type BackdropPolarity } from "./meta";

/** One backdrop colour in the menu: the built-in's id and name, and the surface it makes under the menu's side. */
export interface BackdropOption {
    value: string;
    label: string;
    color: string;
}

/** The side a polarity setting means under the app theme. */
export const schemeFor = (polarity: BackdropPolarity, app: AppScheme): AppScheme => (polarity === "follow" ? app : polarity);

/** The 14 built-ins, each drawn as the surface a choice would make under `scheme`. */
export function builtinOptions(scheme: AppScheme): BackdropOption[] {
    return BUILTINS.map((b) => ({ value: b.id, label: b.name, color: deriveBuiltin(b.id, scheme)!.surfaces.surface }));
}

export const builtinRecord = (id: string, polarity: BackdropPolarity): BackdropMeta => ({ kind: "builtin", id, polarity, version: BACKDROP_VERSION });
