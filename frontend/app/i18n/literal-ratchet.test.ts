// A ratchet on literal text in the files that have started to use the dictionary.
//
// A file that imports the i18n module says it is being translated. In it, a piece of visible text written in
// the JSX, or in a label-like attribute, is text a translation will miss. A file may have none, unless it is
// listed in `literals.baseline.json` with the number it had when it was listed; that number may only go down.
// The baseline is the work still to do, in the open: migrating more of a file lowers its number (run with
// I18N_BASELINE_UPDATE=1 to write the lower one), and a file that adopts the dictionary with text left in it
// must list that text, which a reviewer sees in the diff.
//
// What counts: JSX text with a letter in it, and a string literal given to title, aria-label, placeholder,
// alt, label, description or tooltip. The count is from the parsed file, not a pattern over text.
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";
import { describe, expect, it } from "vitest";

const here = dirname(fileURLToPath(import.meta.url));
const appDir = join(here, "..");
const baselinePath = join(here, "literals.baseline.json");

const ATTRIBUTES = new Set(["title", "aria-label", "placeholder", "alt", "label", "description", "tooltip"]);

function sourceFiles(dir: string): string[] {
    const out: string[] = [];
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
        const path = join(dir, entry.name);
        if (entry.isDirectory()) {
            if (entry.name !== "node_modules") out.push(...sourceFiles(path));
        } else if (entry.name.endsWith(".tsx") && !/\.test\.tsx$/.test(entry.name)) {
            out.push(path);
        }
    }
    return out;
}

const HAS_LETTER = /\p{L}/u;

/** How many pieces of literal visible text `source` (a .tsx file) holds. */
export function countLiterals(source: string, fileName = "file.tsx"): number {
    const tree = ts.createSourceFile(fileName, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
    let count = 0;
    const visit = (node: ts.Node) => {
        if (ts.isJsxText(node)) {
            const text = node.getText().replace(/&\w+;|&#\d+;/g, " ").trim();
            if (HAS_LETTER.test(text)) count++;
        } else if (ts.isJsxAttribute(node) && node.initializer && ts.isStringLiteral(node.initializer)) {
            if (ATTRIBUTES.has(node.name.getText()) && HAS_LETTER.test(node.initializer.text)) count++;
        }
        ts.forEachChild(node, visit);
    };
    visit(tree);
    return count;
}

const USES_DICTIONARY = /from\s+"@\/app\/i18n\//;

function currentCounts(): Record<string, number> {
    const counts: Record<string, number> = {};
    for (const file of sourceFiles(appDir)) {
        const text = readFileSync(file, "utf8");
        if (!USES_DICTIONARY.test(text)) continue;
        const key = "frontend/app/" + relative(appDir, file).replace(/\\/g, "/");
        counts[key] = countLiterals(text, file);
    }
    return counts;
}

/** Files over their allowance, and files whose baseline is higher than their count now (it must come down). */
export function ratchetProblems(now: Record<string, number>, baseline: Record<string, number>): { over: string[]; stale: string[] } {
    const over = Object.entries(now)
        .filter(([file, n]) => n > (baseline[file] ?? 0))
        .map(([file, n]) => `${file}: ${n} literal piece(s), allowed ${baseline[file] ?? 0}`);
    const stale = Object.entries(baseline)
        .filter(([file, n]) => (now[file] ?? 0) < n)
        .map(([file, n]) => `${file}: now ${now[file] ?? 0}, baseline ${n}`);
    return { over, stale };
}

describe("ratchetProblems", () => {
    it("fails a file that grew, a new file with text left in it, and a baseline that is too high", () => {
        const { over, stale } = ratchetProblems({ "a.tsx": 3, "new.tsx": 1, "clean.tsx": 0, "down.tsx": 1 }, { "a.tsx": 2, "down.tsx": 4, "gone.tsx": 2 });
        expect(over).toEqual(["a.tsx: 3 literal piece(s), allowed 2", "new.tsx: 1 literal piece(s), allowed 0"]);
        expect(stale).toEqual(["down.tsx: now 1, baseline 4", "gone.tsx: now 0, baseline 2"]);
    });

    it("passes when every file is at or under its allowance and the baseline is exact", () => {
        expect(ratchetProblems({ "a.tsx": 2, "clean.tsx": 0 }, { "a.tsx": 2 })).toEqual({ over: [], stale: [] });
    });
});

describe("countLiterals", () => {
    it("counts JSX text and label-like string attributes, not code or expressions", () => {
        const source = `export const A = () => (
            <div title="Close" aria-label="Close pane" data-testid="x" class="btn">
                Hello <b>{name()}</b>
                <span>{t("a.b")}</span>
                <input placeholder={t("c.d")} />
                <p>&nbsp;</p>
                <i>42</i>
            </div>
        );`;
        // "Close", "Close pane", "Hello"; not data-testid, class, the t() calls, the entity or the number.
        expect(countLiterals(source)).toBe(3);
    });

    it("counts text in any script", () => {
        expect(countLiterals(`export const A = () => <p>日本語</p>;`)).toBe(1);
    });

    it("counts nothing in a file with no literal text", () => {
        expect(countLiterals(`export const A = () => <p>{t("a.b")}</p>;`)).toBe(0);
    });
});

describe("literal text in files that use the dictionary", () => {
    const baseline: Record<string, number> = JSON.parse(readFileSync(baselinePath, "utf8"));
    const now = currentCounts();

    it("is not more than the baseline allows", () => {
        expect(ratchetProblems(now, baseline).over, "move the text into the dictionary, or list it in literals.baseline.json").toEqual([]);
    });

    it("has come down as far as it can: a lower number is written to the baseline", () => {
        if (process.env.I18N_BASELINE_UPDATE === "1") {
            const next = Object.fromEntries(
                Object.entries(now)
                    .filter(([, n]) => n > 0)
                    .sort(([a], [b]) => a.localeCompare(b))
            );
            writeFileSync(baselinePath, JSON.stringify(next, null, 4) + "\n");
            return;
        }
        expect(ratchetProblems(now, baseline).stale, "run with I18N_BASELINE_UPDATE=1 to lower the baseline").toEqual([]);
    });
});
