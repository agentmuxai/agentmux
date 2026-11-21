// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { BUILTINS, deriveBuiltin } from "./builtins";
import { builtinOptions, builtinRecord, schemeFor } from "./picker-model";

describe("picker-model", () => {
    it("takes the side a polarity means under the app theme", () => {
        expect(schemeFor("follow", "dark")).toBe("dark");
        expect(schemeFor("follow", "light")).toBe("light");
        expect(schemeFor("dark", "light")).toBe("dark");
        expect(schemeFor("light", "dark")).toBe("light");
    });

    it("makes the 14 colours in the palette's order, from the same derivation a pane uses", () => {
        const options = builtinOptions("dark");
        expect(options).toHaveLength(14);
        expect(options.map((o) => o.value)).toEqual(BUILTINS.map((b) => b.id));
        // Black and White are the same colour under either side.
        for (const scheme of ["dark", "light"] as const) {
            expect(builtinOptions(scheme).slice(12).map((o) => [o.label, o.color])).toEqual([
                ["Black", "#000000"],
                ["White", "#ffffff"],
            ]);
        }
        const d = deriveBuiltin(BUILTINS[3].id, "dark")!;
        expect(options[3]).toEqual({ value: BUILTINS[3].id, label: BUILTINS[3].name, color: d.surfaces.surface });
        expect(builtinOptions("light")[3].color).toBe(deriveBuiltin(BUILTINS[3].id, "light")!.surfaces.surface);
    });

    it("builds the record a choice writes", () => {
        expect(builtinRecord("teal", "dark")).toEqual({ kind: "builtin", id: "teal", polarity: "dark", version: 1 });
    });
});
