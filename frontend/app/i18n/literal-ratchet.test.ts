// A ratchet on literal text in the app.
//
// Every .tsx file under frontend/app may hold no literal visible text, unless it is listed in
// `literals.baseline.json` with the number it had when it was listed; that number may only go down. The baseline
// is the work still to do, in the open, and its total is the progress meter of the translation work: migrating
// a file lowers its number (run with I18N_BASELINE_UPDATE=1 to write the lower one), and a new file must start
// clean, or list its text, which a reviewer sees in the diff.
//
// What counts, from the parsed file and not a pattern over text: JSX text with a letter in it; a string literal
// given to title, aria-label, ariaLabel, placeholder, alt, label, description, tooltip, text or message; and, as
// the content of a JSX child or one of those attributes written in braces, a string, a template literal, or the
// string branches of a conditional, of ||, ?? and + (for example {open() ? "Hide" : "Show"}, label={`Save ${n}`}).
//
// What it cannot see, so a count of zero is not "the app is translated":
//   - strings in .ts files (menus, command and setting registries, error messages and notices built in code,
//     toasts, tray and window titles), and in the Rust side (srv and the host)
//   - text assembled from parts, or chosen by a function, before it reaches the JSX; text a component receives
//     as a prop from such a file
//   - text set from script (document.title, aria-live announcements, tooltips given to tip() or title set in
//     code), canvas or SVG text, and text inside images
//   - the landing site, the docs site and the mobile app, which have their own code
//   - dates, numbers, plurals and sort order: translated text can still be formatted the English way
//   - whether the translation is right, and layout when a translation is longer than the English
//   - a string that is a code, a model name, a path or a keyword is counted though it should not be translated:
//     list it in the baseline and leave it
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";
import { describe, expect, it } from "vitest";

const here = dirname(fileURLToPath(import.meta.url));
const appDir = join(here, "..");
const baselinePath = join(here, "literals.baseline.json");

const ATTRIBUTES = new Set(["title", "aria-label", "ariaLabel", "placeholder", "alt", "label", "description", "tooltip", "text", "message"]);

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

/** The visible strings an expression can yield: its literals, template parts and the string branches of a conditional, ||, ?? and +. */
function expressionStrings(node: ts.Expression): number {
    if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) return HAS_LETTER.test(node.text) ? 1 : 0;
    if (ts.isTemplateExpression(node)) {
        const text = node.head.text + node.templateSpans.map((span) => span.literal.text).join(" ");
        return HAS_LETTER.test(text) ? 1 : 0;
    }
    if (ts.isParenthesizedExpression(node)) return expressionStrings(node.expression);
    if (ts.isConditionalExpression(node)) return expressionStrings(node.whenTrue) + expressionStrings(node.whenFalse);
    if (ts.isBinaryExpression(node)) {
        const op = node.operatorToken.kind;
        if (op === ts.SyntaxKind.AmpersandAmpersandToken) return expressionStrings(node.right);
        if (op === ts.SyntaxKind.BarBarToken || op === ts.SyntaxKind.QuestionQuestionToken || op === ts.SyntaxKind.PlusToken) {
            return expressionStrings(node.left) + expressionStrings(node.right);
        }
    }
    return 0;
}

/** How many pieces of literal visible text `source` (a .tsx file) holds. */
export function countLiterals(source: string, fileName = "file.tsx"): number {
    const tree = ts.createSourceFile(fileName, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
    let count = 0;
    const visit = (node: ts.Node) => {
        if (ts.isJsxText(node)) {
            const text = node.getText().replace(/&\w+;|&#\d+;/g, " ").trim();
            if (HAS_LETTER.test(text)) count++;
        } else if (ts.isJsxAttribute(node) && node.initializer && ATTRIBUTES.has(node.name.getText())) {
            if (ts.isStringLiteral(node.initializer)) {
                if (HAS_LETTER.test(node.initializer.text)) count++;
            } else if (ts.isJsxExpression(node.initializer) && node.initializer.expression) {
                count += expressionStrings(node.initializer.expression);
            }
        } else if (ts.isJsxExpression(node) && node.expression && (ts.isJsxElement(node.parent) || ts.isJsxFragment(node.parent))) {
            count += expressionStrings(node.expression);
        }
        ts.forEachChild(node, visit);
    };
    visit(tree);
    return count;
}

function currentCounts(): Record<string, number> {
    const counts: Record<string, number> = {};
    for (const file of sourceFiles(appDir)) {
        const text = readFileSync(file, "utf8");
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

    it("counts strings written in braces as a child or as a label-like attribute", () => {
        const source = `export const A = (p) => (
            <div>
                <b>{open() ? "Hide" : "Show"}</b>
                <i>{\`Reclaim \${size}…\`}</i>
                <u>{name() || "Unnamed"}</u>
                <s>{ready() && "Ready"}</s>
                <button aria-label={\`Remove \${p.name}\`} title={p.busy ? "Wait" : t("x.y")} label={t("a.b")} />
            </div>
        );`;
        // "Hide", "Show", the Reclaim template, "Unnamed", "Ready", the Remove template and "Wait".
        expect(countLiterals(source)).toBe(7);
    });

    it("does not count strings that only decide something", () => {
        const source = `export const A = (p) => (
            <div class={p.kind === "wide" ? "a" : "b"}>
                <Show when={p.mode === "full"}>{t("a.b")}</Show>
                <input value={p.value ?? ""} onInput={() => set("agent:name", 1)} data-store="main store" />
                {items().map((x) => x.label)}
            </div>
        );`;
        expect(countLiterals(source)).toBe(0);
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
