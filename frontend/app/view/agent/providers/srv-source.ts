// Reading srv's provider registry (crates/srv/src/backend/providers.rs and providers/*.rs) from a test.
//
// The frontend catalog (catalog.ts) and srv's `ProviderConfig` statics describe the same providers, and a few
// fields of each are copies of the other's. The drift guards (pin-consistency.test.ts, catalog-parity.test.ts)
// compare them by reading srv's Rust source, so a new provider or a changed field is found from the source,
// never from a list kept by hand. Every extraction throws when it finds nothing: a refactor of the Rust side
// fails the test loudly instead of comparing against an empty value.
import { readdirSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../../../..");

export function read(rel: string): string {
    return readFileSync(resolve(repoRoot, rel), "utf8");
}

/** srv's provider registry: providers.rs plus one file per provider in providers/. */
export function readSrvProviders(): string {
    const dir = "crates/srv/src/backend/providers";
    const files = readdirSync(resolve(repoRoot, dir)).filter((f) => f.endsWith(".rs"));
    return [read(`${dir}.rs`), ...files.sort().map((f) => read(`${dir}/${f}`))].join("\n");
}

export interface SrvStatic {
    /** The Rust static's name, e.g. `CLAUDE`. */
    name: string;
    /** The provider's id, e.g. `claude`. */
    id: string;
    /** The block `ProviderConfig { … }`, full-line and trailing comments removed. */
    block: string;
}

function withoutComments(text: string): string {
    return text.replace(/^[ \t]*\/\/[^\n]*\n/gm, "").replace(/([,\[({])[ \t]*\/\/[^"\n]*$/gm, "$1");
}

/** Every `static NAME: ProviderConfig = ProviderConfig { … };` srv defines. */
export function srvStatics(source: string): SrvStatic[] {
    const out: SrvStatic[] = [];
    for (const m of source.matchAll(/static ([A-Z_]+): ProviderConfig = ProviderConfig \{([\s\S]*?)\n\};/g)) {
        const block = `${withoutComments(m[2])}\n`;
        const id = block.match(/\n {4}id: "([^"]+)"/)?.[1];
        if (!id) throw new Error(`static ${m[1]} has no id`);
        out.push({ name: m[1], id, block });
    }
    if (out.length === 0) throw new Error("no ProviderConfig statics found in srv's providers");
    return out;
}

/**
 * The expression of a top-level field (4-space indent: codex nests its own `launch_args` inside `app_server`).
 * Throws when the field is not there.
 */
export function srvField(provider: SrvStatic, field: string): string {
    const m = provider.block.match(new RegExp(`\\n {4}${field}: ([\\s\\S]*?),\\n(?= {4}\\w|$)`));
    if (!m) throw new Error(`${field} not found for static ${provider.name} in agentmux-srv providers`);
    return m[1].trim();
}

const STRING = '"((?:[^"\\\\]|\\\\.)*)"';

/** `"x"` → `x`. */
export function srvString(expr: string): string {
    const m = expr.match(new RegExp(`^${STRING}$`));
    if (!m) throw new Error(`not a string literal: ${expr}`);
    return m[1];
}

/** `None` → `null`, `Some("x")` → `x`. */
export function srvOptString(expr: string): string | null {
    if (expr === "None") return null;
    const m = expr.match(new RegExp(`^Some\\(${STRING}\\)$`));
    if (!m) throw new Error(`not an Option<&str>: ${expr}`);
    return m[1];
}

/** `&["a", "b"]` → `["a", "b"]`; `None` → `null`; `Some(&[…])` → the list. */
export function srvStringList(expr: string): string[] | null {
    if (expr === "None") return null;
    const m = expr.match(/^(?:Some\()?&\[([\s\S]*?)\]\)?$/);
    if (!m) throw new Error(`not a string list: ${expr}`);
    return [...m[1].matchAll(new RegExp(STRING, "g"))].map((s) => s[1]);
}

/** `&[("k", "v")]` → `{ k: "v" }`. */
export function srvPairs(expr: string): Record<string, string> {
    const m = expr.match(/^&\[([\s\S]*)\]$/);
    if (!m) throw new Error(`not a pair list: ${expr}`);
    const out: Record<string, string> = {};
    for (const p of m[1].matchAll(new RegExp(`\\(\\s*${STRING}\\s*,\\s*${STRING}\\s*\\)`, "g"))) out[p[1]] = p[2];
    return out;
}
