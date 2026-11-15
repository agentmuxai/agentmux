// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { BUILTINS, deriveBuiltin } from "./builtins";
import { builtinOptions, builtinRecord, colorRecord, contrastReadout, parseHexInput, schemeFor } from "./picker-model";

describe("picker-model", () => {
    it("takes the side a polarity means under the app theme", () => {
        expect(schemeFor("follow", "dark")).toBe("dark");
        expect(schemeFor("follow", "light")).toBe("light");
        expect(schemeFor("dark", "light")).toBe("dark");
        expect(schemeFor("light", "dark")).toBe("light");
    });

    it("reads a typed colour as #rrggbb, three digits or six, with or without the #, any case", () => {
        expect(parseHexInput("#336699")).toBe("#336699");
        expect(parseHexInput("336699")).toBe("#336699");
        expect(parseHexInput(" ABC ")).toBe("#aabbcc");
        expect(parseHexInput("#aBc")).toBe("#aabbcc");
        for (const bad of ["", "#", "#12", "#1234", "#12345", "#1234567", "teal", "#ggg", "rgb(1,2,3)"]) expect(parseHexInput(bad), bad).toBeUndefined();
    });

    it("makes the 16 swatches in the palette's order, from the same derivation a pane uses", () => {
        const options = builtinOptions("dark");
        expect(options.map((o) => o.value)).toEqual(BUILTINS.map((b) => b.id));
        const d = deriveBuiltin(BUILTINS[3].id, "dark")!;
        expect(options[3]).toMatchObject({ label: BUILTINS[3].name, color: d.surfaces.surface, ink: d.ink.text, detail: contrastReadout(d) });
        expect(builtinOptions("light")[3].color).toBe(deriveBuiltin(BUILTINS[3].id, "light")!.surfaces.surface);
    });

    it("reads contrast as one decimal, text then muted", () => {
        expect(contrastReadout(deriveBuiltin("teal", "dark")!)).toMatch(/^text \d+\.\d:1, muted \d+\.\d:1$/);
    });

    it("builds the records a choice writes", () => {
        expect(builtinRecord("teal", "dark")).toEqual({ kind: "builtin", id: "teal", polarity: "dark", version: 1 });
        expect(colorRecord("#336699", "follow")).toEqual({ kind: "color", hex: "#336699", polarity: "follow", version: 1 });
    });
});
