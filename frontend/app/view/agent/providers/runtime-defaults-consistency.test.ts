// Drift guard for the runtime defaults srv duplicates from the frontend.
//
// `agent.open` (MCP `OpenAgent`, layouts) builds a pane without the frontend, so
// srv has to pick the pane's starting model, effort and permission mode itself
// (`crates/srv/src/server/app_api/agent_runtime_seed.rs`). The runtime menu shows
// the FRONTEND's idea of the default; if the two disagree, a pane opens running
// one model while its menu reads another — the bug this exists to keep closed.
// docs/reports/REPORT_AGENT_RUNTIME_BINDINGS_2026_09_30.md (G3).
//
//   frontend/app/view/agent/types.ts            DEFAULT_RUNTIME_CONFIG
//   frontend/app/view/agent/providers/catalog.ts  models[].default
//   crates/srv/src/backend/providers.rs          default_model_for()
//   crates/srv/src/server/app_api/agent_runtime_seed.rs  DEFAULT_PERMISSION_MODE / DEFAULT_EFFORT
import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { providerSupportsModelFlag } from "../buildRuntimeArgs";
import { DEFAULT_RUNTIME_CONFIG } from "../types";
import { PROVIDERS } from "./index";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../../../..");
const read = (rel: string) => readFileSync(resolve(repoRoot, rel), "utf8");

/** `default_model_for`'s match arms: provider id → model (`null` for `None`). */
function srvDefaultModels(): Map<string, string> {
    const fn = read("crates/srv/src/backend/providers.rs").match(
        /pub fn default_model_for\(provider_id: &str\) -> Option<&'static str> \{([\s\S]*?)\n\}/
    );
    if (!fn) throw new Error("default_model_for not found in crates/srv/src/backend/providers.rs");
    const out = new Map<string, string>();
    for (const m of fn[1].matchAll(/"([^"]+)"\s*=>\s*Some\("([^"]+)"\)/g)) out.set(m[1], m[2]);
    return out;
}

function srvConst(name: string): string {
    const m = read("crates/srv/src/server/app_api/agent_runtime_seed.rs").match(
        new RegExp(`const ${name}: &str = "([^"]+)";`)
    );
    if (!m) throw new Error(`${name} not found in agent_runtime_seed.rs`);
    return m[1];
}

describe("srv runtime defaults match the frontend", () => {
    const srv = srvDefaultModels();

    it("srv names a default model for exactly the providers whose model the menu wires", () => {
        const wired = Object.keys(PROVIDERS)
            .filter((id) => providerSupportsModelFlag(id))
            .sort();
        expect([...srv.keys()].sort()).toEqual(wired);
    });

    for (const [id, model] of srvDefaultModels()) {
        it(`${id}: srv's default model is the catalog's default row`, () => {
            const catalogDefault = PROVIDERS[id]?.models?.find((m) => m.default)?.value;
            expect(model).toBe(catalogDefault);
        });
    }

    it("the permission mode and effort a seeded pane gets are DEFAULT_RUNTIME_CONFIG's", () => {
        expect(srvConst("DEFAULT_PERMISSION_MODE")).toBe(DEFAULT_RUNTIME_CONFIG.permissionMode);
        expect(srvConst("DEFAULT_EFFORT")).toBe(DEFAULT_RUNTIME_CONFIG.effort);
    });

    it("claude's default is the one DEFAULT_RUNTIME_CONFIG falls back to", () => {
        expect(srv.get("claude")).toBe(DEFAULT_RUNTIME_CONFIG.model);
    });
});
