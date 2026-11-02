// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The window's side of the native / container parity harness
 * (step 1 of the native-and-container-from-one-config plan, task #13666).
 *
 * The window's Launch builds an agent's startup file, settings, skills and `.mcp.json` here, in
 * `buildConfigFiles`, and sends them through `writeagentconfig`; `agent.open` builds the same files in Rust.
 * This renders the shared fixture (crates/srv/tests/fixtures/agent_config_parity/fixture.json) and compares it with
 * the committed `window_files.golden.json`, which the Rust test (`agent_config_parity_tests`) feeds to the Rust half
 * of the picker path and compares with `agent.open`'s output. Regenerate with
 * `UPDATE_PARITY_GOLDEN=1 npx vitest run frontend/app/view/agent/agent-config-parity.test.ts`.
 */

import { readFileSync, writeFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import type { AgentDefinition, AgentSkill } from "@/app/store/rpc-api";
import { buildConfigFiles } from "./agent-config-builder";

const DIR = new URL("../../../../crates/srv/tests/fixtures/agent_config_parity/", import.meta.url);
const read = (name: string): string => readFileSync(new URL(name, DIR), "utf8");

interface Fixture {
    agent: { id: string; name: string; slug: string; provider: string; working_directory: string };
    content: Record<string, string>;
    skills: { id: string; name: string; trigger: string; skill_type: string; description: string; content: string }[];
}

describe("agent config parity: the window's files for the shared fixture", () => {
    it("match the committed golden (the input of the Rust half)", () => {
        const fx = JSON.parse(read("fixture.json")) as Fixture;
        const skills = fx.skills.map((s) => ({ ...s, agent_id: fx.agent.id, created_at: 0 })) as AgentSkill[];
        const agent = { ...fx.agent, agent_type: "host" } as unknown as AgentDefinition;
        const files = buildConfigFiles(fx.content, skills, agent, fx.agent.name, fx.agent.provider);
        const rendered = JSON.stringify(
            files.map((f) => ({ path: f.path, content: f.content })).sort((a, b) => a.path.localeCompare(b.path)),
            null,
            2
        );
        if (process.env.UPDATE_PARITY_GOLDEN) writeFileSync(new URL("window_files.golden.json", DIR), rendered + "\n");
        expect(rendered + "\n").toBe(read("window_files.golden.json"));
    });
});
