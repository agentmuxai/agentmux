// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** A pane's backdrop record: what is read from meta, and what it resolves to under each app theme. */

import { describe, expect, it } from "vitest";
import { BUILTINS, deriveBuiltin } from "./builtins";
import { deriveInk } from "./ink";
import { BACKDROP_VERSION, parseBackdrop, resolveBackdrop, sameBackdrop, sameResolved, supportsBackdrop } from "./meta";

const teal = { kind: "builtin", id: "teal", polarity: "follow", version: BACKDROP_VERSION };

describe("parseBackdrop", () => {
    it("reads a built-in, a custom colour (lower-cased) and an image, and fills in the polarity and version", () => {
        expect(parseBackdrop(teal)).toEqual(teal);
        expect(parseBackdrop({ kind: "color", hex: "#ABCDEF" })).toEqual({ kind: "color", hex: "#abcdef", polarity: "follow", version: 1 });
        expect(parseBackdrop({ kind: "image", imageId: "img-1", polarity: "dark" })).toEqual({ kind: "image", imageId: "img-1", polarity: "dark", version: 1 });
    });

    it("reads none for what is not a record, or is not one this build can use", () => {
        for (const bad of [
            undefined,
            null,
            "teal",
            42,
            [],
            {},
            { kind: "builtin" },
            { kind: "builtin", id: "not-a-colour" },
            { kind: "color", hex: "#abc" },
            { kind: "color", hex: "teal" },
            { kind: "image" },
            { kind: "image", imageId: "" },
            { ...teal, polarity: "sideways" },
            { ...teal, version: BACKDROP_VERSION + 1 },
            { ...teal, version: 0 },
            { ...teal, version: "1" },
        ]) {
            expect(parseBackdrop(bad), JSON.stringify(bad)).toBeUndefined();
        }
    });

    it("knows every one of the 14 built-ins", () => {
        expect(BUILTINS).toHaveLength(14);
        for (const b of BUILTINS) expect(parseBackdrop({ kind: "builtin", id: b.id })?.id).toBe(b.id);
    });

    it("reads an id retired with the 16 slots as the slot it became", () => {
        for (const [was, now] of [
            ["crimson", "red"],
            ["lime", "yellow"],
            ["cyan", "teal"],
            ["azure", "blue"],
            ["indigo", "violet"],
            ["pink", "rose"],
        ]) {
            expect(parseBackdrop({ kind: "builtin", id: was }), was).toEqual({ kind: "builtin", id: now, polarity: "follow", version: 1 });
        }
        expect(resolveBackdrop({ backdrop: { kind: "builtin", id: "azure" } }, "dark")).toEqual({ id: "builtin:blue", derived: deriveBuiltin("blue", "dark") });
    });
});

describe("resolveBackdrop", () => {
    it("derives a built-in for the app theme under follow, and keeps its own side under dark or light", () => {
        expect(resolveBackdrop({ backdrop: teal }, "dark")).toEqual({ id: "builtin:teal", derived: deriveBuiltin("teal", "dark") });
        expect(resolveBackdrop({ backdrop: teal }, "light")).toEqual({ id: "builtin:teal", derived: deriveBuiltin("teal", "light") });
        const fixed = { ...teal, polarity: "dark" };
        expect(resolveBackdrop({ backdrop: fixed }, "light")?.derived).toEqual(deriveBuiltin("teal", "dark"));
        expect(resolveBackdrop({ backdrop: { ...teal, polarity: "light" } }, "dark")?.derived).toEqual(deriveBuiltin("teal", "light"));
    });

    it("keeps Black and White the same surface under either app theme and any polarity", () => {
        for (const [id, hex] of [
            ["black", "#000000"],
            ["white", "#ffffff"],
        ]) {
            for (const polarity of ["follow", "dark", "light"]) {
                for (const app of ["dark", "light"] as const) {
                    const resolved = resolveBackdrop({ backdrop: { kind: "builtin", id, polarity } }, app);
                    expect(resolved?.id).toBe(`builtin:${id}`);
                    expect(resolved?.derived.surfaces.surface, `${id}/${polarity}/${app}`).toBe(hex);
                    expect(resolved?.derived).toEqual(deriveInk(hex));
                }
            }
        }
    });

    it("derives a custom colour from the colour, whatever the app theme or polarity", () => {
        const color = { kind: "color", hex: "#336699", polarity: "light", version: 1 };
        expect(resolveBackdrop({ backdrop: color }, "dark")).toEqual({ id: "color", derived: deriveInk("#336699") });
        expect(resolveBackdrop({ backdrop: color }, "light")?.derived).toEqual(deriveInk("#336699"));
    });

    it("has nothing to apply for no record, an image (before phase 3) or a record it can't read", () => {
        expect(resolveBackdrop(undefined, "dark")).toBeUndefined();
        expect(resolveBackdrop({}, "dark")).toBeUndefined();
        expect(resolveBackdrop({ backdrop: { kind: "image", imageId: "i" } }, "dark")).toBeUndefined();
        expect(resolveBackdrop({ backdrop: { kind: "builtin", id: "nope" } }, "dark")).toBeUndefined();
    });

    it("makes a legacy backdrop of a plain colour bg, #rgb included", () => {
        expect(resolveBackdrop({ bg: "#336699" }, "dark")).toEqual({ id: "legacy", derived: deriveInk("#336699") });
        expect(resolveBackdrop({ bg: " #369 " }, "dark")?.derived).toEqual(deriveInk("#336699"));
    });

    it("leaves a bg that is not a plain colour alone: a gradient, a url, a name, a number", () => {
        for (const bg of ["linear-gradient(red, blue)", "url(a.png)", "tomato", "rgb(1, 2, 3)", "#12345", 7, null, ""]) {
            expect(resolveBackdrop({ bg }, "dark"), String(bg)).toBeUndefined();
        }
    });

    it("lets a readable record win over the old bg, and falls back to bg when the record can't be read", () => {
        expect(resolveBackdrop({ backdrop: teal, bg: "#336699" }, "dark")?.id).toBe("builtin:teal");
        expect(resolveBackdrop({ backdrop: { kind: "builtin", id: "nope" }, bg: "#336699" }, "dark")?.id).toBe("legacy");
    });
});

describe("supportsBackdrop", () => {
    it("is the agent pane for now, and no terminal, widget or browser view", () => {
        expect(supportsBackdrop("agent")).toBe(true);
        for (const view of ["term", "files", "browser", "sysinfo", "kanban", "", undefined, null, 7]) expect(supportsBackdrop(view), String(view)).toBe(false);
    });
});

describe("sameBackdrop and sameResolved", () => {
    it("compares records by their fields, not by identity: a re-parsed record is the same one", () => {
        expect(sameBackdrop(parseBackdrop(teal), parseBackdrop({ ...teal }))).toBe(true);
        expect(sameBackdrop(undefined, undefined)).toBe(true);
    });

    it("sees every field that makes it another backdrop", () => {
        const base = parseBackdrop(teal)!;
        expect(sameBackdrop(base, undefined)).toBe(false);
        expect(sameBackdrop(undefined, base)).toBe(false);
        for (const other of [{ ...teal, id: "red" }, { ...teal, polarity: "dark" }, { kind: "color", hex: "#336699" }]) {
            expect(sameBackdrop(base, parseBackdrop(other)), JSON.stringify(other)).toBe(false);
        }
        expect(sameBackdrop(parseBackdrop({ kind: "color", hex: "#336699" }), parseBackdrop({ kind: "color", hex: "#336698" }))).toBe(false);
    });

    it("treats a backdrop resolved again from the same record as the same, and another colour or side as another", () => {
        const a = resolveBackdrop({ backdrop: teal }, "dark");
        expect(sameResolved(a, resolveBackdrop({ backdrop: { ...teal } }, "dark"))).toBe(true);
        expect(sameResolved(a, resolveBackdrop({ backdrop: teal }, "light"))).toBe(false);
        expect(sameResolved(a, resolveBackdrop({ backdrop: { ...teal, id: "red" } }, "dark"))).toBe(false);
        expect(sameResolved(a, undefined)).toBe(false);
        expect(sameResolved(undefined, undefined)).toBe(true);
    });
});
