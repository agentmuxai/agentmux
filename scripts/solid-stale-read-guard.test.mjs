// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for solid-stale-read-guard.mjs. The last tests run it against the installed Solid and import the
// result, so an update that changes how <Show> and <Match> build their accessors fails here, not as a window that
// stops updating.
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { build } from "vite";
import { afterEach, describe, expect, it, vi } from "vitest";
import { isSolidCore, solidStaleReadGuard, withGuardedStaleReads } from "./solid-stale-read-guard.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const solidFile = (name) => path.resolve(here, "../node_modules/solid-js/dist", name);

describe("isSolidCore", () => {
    it("matches Solid's two core files on either path style, with a query", () => {
        expect(isSolidCore("C:\\repo\\node_modules\\solid-js\\dist\\solid.js")).toBe(true);
        expect(isSolidCore("/repo/node_modules/solid-js/dist/dev.js?v=1")).toBe(true);
    });
    it("leaves Solid's other files and other packages alone", () => {
        expect(isSolidCore("/repo/node_modules/solid-js/web/dist/web.js")).toBe(false);
        expect(isSolidCore("/repo/node_modules/solid-js/store/dist/store.js")).toBe(false);
        expect(isSolidCore("/repo/node_modules/other/dist/solid.js")).toBe(false);
    });
});

describe("withGuardedStaleReads", () => {
    it("throws when Solid's file no longer looks like that", () => {
        expect(() => withGuardedStaleReads("export const a = 1;")).toThrow(/solid-stale-read-guard/);
    });
});

describe("solidStaleReadGuard", () => {
    it("rewrites only Solid's core files", () => {
        expect(solidStaleReadGuard().transform("export const a = 1;", "/repo/src/a.ts")).toBeNull();
    });

    it("is applied by a real vite build, to the Solid it bundles", async () => {
        // The entry sits in this repo so that `solid-js` resolves from its node_modules.
        const dir = fs.mkdtempSync(path.join(here, ".guard-build-"));
        try {
            const entry = path.join(dir, "entry.mjs");
            fs.writeFileSync(entry, 'export { Show, Switch, Match, createRoot } from "solid-js";\n');
            const result = await build({
                root: path.resolve(here, ".."),
                configFile: false,
                logLevel: "silent",
                plugins: [solidStaleReadGuard()],
                build: { write: false, minify: false, lib: { entry, formats: ["es"], fileName: "out" } },
            });
            const code = (Array.isArray(result) ? result[0] : result).output[0].code;
            expect(code).toContain("function staleRead(");
            expect(code).not.toContain("throw narrowedError");
        } finally {
            fs.rmSync(dir, { recursive: true, force: true });
        }
    }, 60_000);
});

/** Solid's `name` file, rewritten by the plugin and imported from a temporary folder. */
async function loadGuarded(name, dirs) {
    const file = solidFile(name);
    const out = solidStaleReadGuard().transform(fs.readFileSync(file, "utf8"), file);
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "solid-stale-read-"));
    dirs.push(dir);
    const patched = path.join(dir, name.replace(".js", ".mjs"));
    fs.writeFileSync(patched, out.code);
    return import(pathToFileURL(patched).href);
}

describe.each(["solid.js", "dev.js"])("the installed %s, guarded", (name) => {
    const dirs = [];
    afterEach(() => {
        vi.restoreAllMocks();
        for (const dir of dirs.splice(0)) fs.rmSync(dir, { recursive: true, force: true });
    });

    it("a <Show> accessor read after the condition went false answers with its last value and warns with the reader's stack", async () => {
        const solid = await loadGuarded(name, dirs);
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        const [when, setWhen] = solid.createSignal({ id: "a" });
        let accessor;
        solid.createRoot(() => {
            solid.Show({
                get when() {
                    return when();
                },
                children: (a) => ((accessor = a), null),
            });
        });
        expect(accessor().id).toBe("a");
        setWhen({ id: "b" });
        expect(accessor().id).toBe("b");
        setWhen(undefined);

        let value;
        function readerOfTheStaleAccessor() {
            value = accessor();
        }
        expect(readerOfTheStaleAccessor).not.toThrow();
        expect(value).toEqual({ id: "b" });
        expect(warn).toHaveBeenCalledTimes(1);
        const [message, error] = warn.mock.calls[0];
        expect(message).toContain("[solid] a stale read from <Show>");
        expect(error).toBeInstanceOf(Error);
        expect(error.stack).toContain("readerOfTheStaleAccessor");
    });

    it("a stale read does not leave a computation stale, so one that reads it keeps running", async () => {
        const solid = await loadGuarded(name, dirs);
        vi.spyOn(console, "warn").mockImplementation(() => {});
        const [when, setWhen] = solid.createSignal({ id: "a" });
        const [tick, setTick] = solid.createSignal(0);
        let accessor;
        const seen = [];
        solid.createRoot(() => {
            solid.Show({
                get when() {
                    return when();
                },
                children: (a) => ((accessor = a), null),
            });
            // Not owned by the <Show>: it outlives it and keeps reading its accessor on every tick.
            solid.createEffect(() => {
                tick();
                seen.push(accessor().id);
            });
        });
        // The effect also tracks the condition, so it can run before the <Show> does: the order that threw.
        expect(() => setWhen(undefined)).not.toThrow();
        expect(() => setTick(1)).not.toThrow();
        expect(() => setTick(2)).not.toThrow();
        expect(seen.length).toBeGreaterThanOrEqual(3);
        expect(seen.every((id) => id === "a")).toBe(true);
    });

    it("a <Match> accessor read after its branch went false answers with its last value", async () => {
        const solid = await loadGuarded(name, dirs);
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        const [when, setWhen] = solid.createSignal({ id: "m" });
        let accessor;
        solid.createRoot(() => {
            solid.Switch({
                get children() {
                    return solid.Match({
                        get when() {
                            return when();
                        },
                        children: (a) => ((accessor = a), null),
                    });
                },
            });
        });
        expect(accessor().id).toBe("m");
        setWhen(undefined);
        expect(accessor()).toEqual({ id: "m" });
        expect(warn).toHaveBeenCalledTimes(1);
        expect(warn.mock.calls[0][0]).toContain("<Match>");
    });

    it("a keyed <Show> and a read while the condition holds are unchanged", async () => {
        const solid = await loadGuarded(name, dirs);
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        const [when, setWhen] = solid.createSignal("on");
        let given;
        solid.createRoot(() => {
            solid.Show({
                keyed: true,
                get when() {
                    return when();
                },
                children: (v) => ((given = v), null),
            });
        });
        expect(given).toBe("on");
        setWhen("again");
        expect(given).toBe("again");
        expect(warn).not.toHaveBeenCalled();
    });
});
