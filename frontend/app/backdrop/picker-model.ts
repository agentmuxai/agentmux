// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What the backdrop picker shows and writes, apart from how it draws (BackdropPicker.tsx): the 14 swatches for a
 * side, the contrast readout, a typed colour, and the records a choice makes (spec §9, §10).
 */

import { t } from "../i18n/i18n";
import type { SwatchOption } from "../element/ui/Swatch";
import { contrastRatio } from "../block/pane-color-scheme";
import { BUILTINS, deriveBuiltin, type AppScheme } from "./builtins";
import type { DerivedBackdrop } from "./ink";
import { BACKDROP_VERSION, type BackdropMeta, type BackdropPolarity } from "./meta";

/** The side a polarity setting means under the app theme. */
export const schemeFor = (polarity: BackdropPolarity, app: AppScheme): AppScheme => (polarity === "follow" ? app : polarity);

/** `9.4:1`, one decimal. */
const ratio = (n: number) => `${n.toFixed(1)}:1`;

/** The one-line readout of a backdrop's contrast: primary and muted text on its surface. */
export function contrastReadout(d: DerivedBackdrop): string {
    return t("backdrop.readout", {
        text: ratio(contrastRatio(d.surfaces.surface, d.ink.text)),
        muted: ratio(contrastRatio(d.surfaces.surface, d.ink.muted)),
    });
}

/** The 14 built-ins as swatches, each drawn as the mini-pane a choice would make under `scheme`. */
export function builtinOptions(scheme: AppScheme): SwatchOption<string>[] {
    return BUILTINS.map((b) => {
        const d = deriveBuiltin(b.id, scheme)!;
        return { value: b.id, label: b.name, color: d.surfaces.surface, ink: d.ink.text, detail: contrastReadout(d) };
    });
}

/** A typed colour as `#rrggbb`: `#abc`, `abc`, `#aabbcc` and `aabbcc` are accepted (any case); anything else is not a colour. */
export function parseHexInput(text: string): string | undefined {
    const m = /^#?([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(text.trim());
    if (!m) return undefined;
    const h = m[1].toLowerCase();
    return `#${h.length === 3 ? [...h].map((c) => c + c).join("") : h}`;
}

export const builtinRecord = (id: string, polarity: BackdropPolarity): BackdropMeta => ({ kind: "builtin", id, polarity, version: BACKDROP_VERSION });
export const colorRecord = (hex: string, polarity: BackdropPolarity): BackdropMeta => ({ kind: "color", hex, polarity, version: BACKDROP_VERSION });
