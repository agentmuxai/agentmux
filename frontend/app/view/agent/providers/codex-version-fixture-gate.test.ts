// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { PROVIDERS } from "./index";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../../../..");
const pin = PROVIDERS.codex.pinnedVersion;
const fixtureDir = resolve(repoRoot, "frontend/test/fixtures/providers/codex", pin);
const requiredScenarios = ["normal", "command", "file-change", "resume", "docker-resume"] as const;

function readJson(path: string): any {
    return JSON.parse(readFileSync(path, "utf8"));
}

function readJsonl(path: string): any[] {
    return readFileSync(path, "utf8")
        .trim()
        .split(/\r?\n/)
        .map((line) => JSON.parse(line));
}

describe("Codex pin compatibility fixtures", () => {
    it("has a complete candidate smoke set for the exact configured pin", () => {
        expect(existsSync(fixtureDir), `missing fixture directory for Codex ${pin}`).toBe(true);

        for (const scenario of requiredScenarios) {
            const manifestPath = resolve(fixtureDir, `${scenario}.manifest.json`);
            const fixturePath = resolve(fixtureDir, `${scenario}.jsonl`);
            expect(existsSync(manifestPath), `missing ${scenario} manifest for Codex ${pin}`).toBe(true);
            expect(existsSync(fixturePath), `missing ${scenario} JSONL for Codex ${pin}`).toBe(true);

            const manifest = readJson(manifestPath);
            expect(manifest.provider).toBe("codex");
            expect(manifest.cli_version).toBe(pin);
            expect(manifest.synthetic).not.toBe(true);
            expect(manifest.unknown_event_types).toEqual([]);
            expect(manifest.unknown_item_types).toEqual([]);

            const messages = readJsonl(fixturePath);
            expect(messages[0]?.type).toBe("thread.started");
            expect(messages.some((message) => message.type === "turn.completed")).toBe(true);
        }
    });

    it("records a successful AgentMux-image two-turn resume", () => {
        const manifest = readJson(resolve(fixtureDir, "docker-resume.manifest.json"));
        expect(manifest.container).toBe(true);
        expect(manifest.container_image).toBe("ghcr.io/agentmuxai/agent-claude:latest");
        expect(manifest.container_user).toBe("agent");
        expect(manifest.same_thread_id).toBe(true);
        expect(manifest.created_file_content_verified).toBe(true);
        expect(manifest.resume_answer_verified).toBe(true);
        expect(manifest.process_tree_reaped).toBe(true);

        const messages = readJsonl(resolve(fixtureDir, "docker-resume.jsonl"));
        const threadIds = messages
            .filter((message) => message.type === "thread.started")
            .map((message) => message.thread_id);
        expect(threadIds).toHaveLength(2);
        expect(new Set(threadIds).size).toBe(1);
        expect(messages.filter((message) => message.type === "turn.completed")).toHaveLength(2);
    });
});
