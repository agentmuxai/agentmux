// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { PROVIDERS } from "./index";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../../../..");
const pin = PROVIDERS.codex.pinnedVersion;
const schemaDir = resolve(repoRoot, "schema/providers/codex/app-server", pin);
const manifestPath = resolve(schemaDir, "manifest.json");

function readJson(path: string): any {
    return JSON.parse(readFileSync(path, "utf8"));
}

function sha256(path: string): string {
    return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function methodSet(schema: any): Set<string> {
    const methods = new Set<string>();
    const visit = (value: unknown): void => {
        if (!value || typeof value !== "object") return;
        const record = value as Record<string, any>;
        const methodEnum = record.properties?.method?.enum;
        if (Array.isArray(methodEnum) && methodEnum.length === 1 && typeof methodEnum[0] === "string") {
            methods.add(methodEnum[0]);
        }
        for (const child of Object.values(record)) visit(child);
    };
    visit(schema);
    return methods;
}

describe("Codex App Server schema snapshot", () => {
    it("matches the exact configured CLI pin and committed hashes", () => {
        expect(existsSync(manifestPath), `missing App Server schema manifest for Codex ${pin}`).toBe(true);
        const manifest = readJson(manifestPath);
        expect(manifest.provider).toBe("codex");
        expect(manifest.package).toBe("@openai/codex");
        expect(manifest.cli_version).toBe(pin);
        expect(manifest.mode).toBe("stable");
        expect(manifest.experimental_api).toBe(false);

        for (const [file, expectedHash] of Object.entries(manifest.files as Record<string, string>)) {
            const path = resolve(schemaDir, file);
            expect(existsSync(path), `missing generated schema ${file} for Codex ${pin}`).toBe(true);
            expect(sha256(path), `${file} drifted from its generated ${pin} snapshot`).toBe(expectedHash);
        }
    });

    it("contains every stable method required by the implementation spec", () => {
        const manifest = readJson(manifestPath);
        const schemas = {
            client_notifications: methodSet(readJson(resolve(schemaDir, "ClientNotification.json"))),
            client_requests: methodSet(readJson(resolve(schemaDir, "ClientRequest.json"))),
            server_requests: methodSet(readJson(resolve(schemaDir, "ServerRequest.json"))),
            server_notifications: methodSet(readJson(resolve(schemaDir, "ServerNotification.json"))),
        };

        for (const [kind, requiredMethods] of Object.entries(manifest.required_surface as Record<string, string[]>)) {
            const available = schemas[kind as keyof typeof schemas];
            expect(available, `unknown schema category ${kind}`).toBeDefined();
            for (const method of requiredMethods) {
                expect(available.has(method), `${method} missing from stable Codex ${pin} ${kind}`).toBe(true);
            }
        }

        expect(manifest.experimental_comparison.required_by_agentmux).toEqual([]);
    });

    it("keeps the bundled v2 schema on the recorded JSON Schema draft", () => {
        const manifest = readJson(manifestPath);
        const bundle = readJson(resolve(schemaDir, "codex_app_server_protocol.v2.schemas.json"));
        expect(bundle.$schema).toBe(manifest.schema_draft);
        expect(bundle.definitions?.ClientRequest).toBeDefined();
        expect(bundle.definitions?.ServerNotification).toBeDefined();
    });
});
