// Every key the code asks for exists in the English dictionary: a mistyped key would show as the key itself.
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import english from "./locales/en.json";

const appDir = join(dirname(fileURLToPath(import.meta.url)), "..");

function sourceFiles(dir: string): string[] {
    const out: string[] = [];
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
        const path = join(dir, entry.name);
        if (entry.isDirectory()) {
            if (entry.name !== "node_modules") out.push(...sourceFiles(path));
        } else if (/\.(ts|tsx)$/.test(entry.name) && !/\.test\.(ts|tsx)$/.test(entry.name)) {
            out.push(path);
        }
    }
    return out;
}

/** `t("a.b")` and `<Rich k="a.b" />`; keys built at run time are not found, and are not used. */
const CALLS = [/\bt\(\s*"([a-z][A-Za-z0-9]*(?:\.[A-Za-z0-9-]+)+)"/g, /\bk="([a-z][A-Za-z0-9]*(?:\.[A-Za-z0-9-]+)+)"/g];

const known = new Set(Object.keys(english).map((k) => k.replace(/\.(zero|one|two|few|many|other)$/, "")));

describe("the keys the code uses", () => {
    const used = new Map<string, string>();
    for (const file of sourceFiles(appDir)) {
        const text = readFileSync(file, "utf8");
        for (const call of CALLS) for (const m of text.matchAll(call)) if (!used.has(m[1])) used.set(m[1], file);
    }

    it("finds the calls (the scan is not empty)", () => {
        expect(used.size).toBeGreaterThan(10);
    });

    it("are all in the English dictionary", () => {
        const missing = [...used].filter(([key]) => !known.has(key)).map(([key, file]) => `${key} (${file.replace(appDir, "app")})`);
        expect(missing).toEqual([]);
    });

    it("are all spelled as the dictionary spells them (the check can fail)", () => {
        expect(known.has("signin.cancel")).toBe(true);
        expect(known.has("signin.cancle")).toBe(false);
    });
});
