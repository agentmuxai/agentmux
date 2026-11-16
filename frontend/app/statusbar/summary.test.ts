// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import type { AttentionItem } from "@/app/notification/attention-list";
import type { WaitingApp } from "@/app/notification/waiting-for-you";
import type { UseRequest } from "@/types/rpc/UseRequest";
import type { VaultRequest } from "@/types/rpc/VaultRequest";
import { isVaultOnly, summarize, type SummaryInput } from "./summary";
import { KIND_PLACES } from "./summary-kinds";

const item = (blockId: string, over: Partial<AttentionItem> = {}): AttentionItem => ({
    blockId, name: `Agent ${blockId}`, what: "asks you a question", since: 100, kinds: [], local: true, ...over,
});
const use = (id: string, over: Partial<UseRequest> = {}) =>
    ({ id, agent: "Builder", block_id: "", entry_label: "GitHub token", requested_ms: 50, goes_ahead: false, deadline_ms: 0, ...over }) as UseRequest;
const request = (id: string, over: Partial<VaultRequest> = {}) =>
    ({ id, agent: "Builder", kind: "login", label: "Stripe", why: "", requested_ms: 60, block_id: "", vault: "", ...over }) as VaultRequest;
const app = (key: string, kind: string, over: Partial<WaitingApp> = {}): WaitingApp => ({ key, kind, what: "asks for your consent", since: 10, ...over });
const input = (over: Partial<SummaryInput> = {}): SummaryInput => ({ running: 0, items: [], apps: [], uses: [], requests: [], tasks: 0, ...over });
const counts = (s: ReturnType<typeof summarize>) => Object.fromEntries(s.parts.map((p) => [p.id, p.count]));

describe("summarize", () => {
    it("is empty when nothing runs and nothing waits", () => {
        const s = summarize(input());
        expect(counts(s)).toEqual({ agents: 0, asks: 0, browser: 0, vault: 0, tasks: 0 });
        expect(s.waiting).toBe(0);
    });

    it("counts running agents without counting them as waiting", () => {
        const s = summarize(input({ running: 20 }));
        expect(counts(s).agents).toBe(20);
        expect(s.waiting).toBe(0);
    });

    it("sorts a pane into Asks, or Browser when all it waits on is a browser request", () => {
        const s = summarize(input({ items: [item("a"), item("b", { kinds: ["browser"], what: "needs you in the browser" })] }));
        expect(counts(s)).toMatchObject({ asks: 1, browser: 1 });
    });

    it("keeps a pane that waits on its own question as well as on a Vault or a browser request as an ask", () => {
        const s = summarize(input({ items: [item("a", { kinds: ["own", "vault"] }), item("b", { kinds: ["own", "browser"] })], uses: [use("u1", { block_id: "a" })] }));
        expect(counts(s)).toMatchObject({ asks: 2, browser: 0, vault: 1 });
        expect(isVaultOnly(item("a", { kinds: ["own", "vault"] }))).toBe(false);
        expect(isVaultOnly(item("a", { kinds: ["vault"] }))).toBe(true);
    });

    it("counts a pane that waits on a question and a browser request once, as an ask", () => {
        expect(counts(summarize(input({ items: [item("a", { kinds: ["browser"] }), item("b", { kinds: ["browser", "consent"] })] })))).toMatchObject({ asks: 1, browser: 1 });
    });

    it("counts a pane that waits only on a Vault request under Vault, not also as an ask", () => {
        const s = summarize(input({ items: [item("p", { kinds: ["vault"] })], uses: [use("u1", { block_id: "p" })] }));
        expect(counts(s)).toMatchObject({ asks: 0, vault: 1 });
        expect(s.waiting).toBe(1);
    });

    it("counts every Vault request, whatever its pane, and one whose pane closed is still found", () => {
        const s = summarize(input({ uses: [use("u1", { block_id: "gone" }), use("u2")], requests: [request("r1")] }));
        expect(counts(s).vault).toBe(3);
        expect(s.parts.find((p) => p.id === "vault")!.rows.every((r) => r.click.to === "vault")).toBe(true);
    });

    it("sends a Vault row to the pane's card when this window knows the pane", () => {
        const s = summarize(input({ items: [item("p", { kinds: ["vault"] })], uses: [use("u1", { block_id: "p" })] }));
        expect(s.parts.find((p) => p.id === "vault")!.rows[0].click).toMatchObject({ to: "pane" });
    });

    it("gives a request that belongs to no pane a row", () => {
        const s = summarize(input({ apps: [app("k1", "consent"), app("k2", "widget", { what: "asks to install a widget" })] }));
        expect(counts(s).asks).toBe(2);
        expect(s.parts.find((p) => p.id === "asks")!.rows.map((r) => r.click)).toEqual([
            { to: "app", kind: "consent", key: "k1" },
            { to: "app", kind: "widget", key: "k2" },
        ]);
    });

    it("does not count an app-level Vault announcement twice: the Vault's own lists have it", () => {
        const s = summarize(input({ apps: [app("k1", "vault")], uses: [use("u1")] }));
        expect(counts(s)).toMatchObject({ asks: 0, vault: 1 });
    });

    it("says when a use goes ahead on its own", () => {
        const s = summarize(input({ uses: [use("u1", { goes_ahead: true, deadline_ms: 9000 })] }));
        expect(s.parts.find((p) => p.id === "vault")!.rows[0].goesAheadAtMs).toBe(9000);
    });

    it("adds the Kanban tasks as one row and counts them all", () => {
        const s = summarize(input({ tasks: 4 }));
        expect(counts(s).tasks).toBe(4);
        expect(s.parts.find((p) => p.id === "tasks")!.rows).toHaveLength(1);
        expect(s.waiting).toBe(4);
    });

    it("totals everything that waits and orders each part's rows oldest first", () => {
        const s = summarize(
            input({ running: 3, items: [item("a", { since: 300 }), item("b", { since: 200 })], apps: [app("k", "consent", { since: 100 })], uses: [use("u")], tasks: 2 })
        );
        expect(s.waiting).toBe(3 + 1 + 2);
        expect(s.parts.find((p) => p.id === "asks")!.rows.map((r) => r.since)).toEqual([100, 200, 300]);
    });
});

describe("every kind srv can announce has a place in the summary", () => {
    // crates/srv-core/src/backend/user_attention.rs: `pub const KINDS: [&str; N] = [...]`.
    const here = dirname(fileURLToPath(import.meta.url));
    const rust = readFileSync(join(here, "../../../crates/srv-core/src/backend/user_attention.rs"), "utf8");
    const kinds = [...(/pub const KINDS: \[&str; \d+\] = \[([^\]]*)\]/.exec(rust)?.[1] ?? "").matchAll(/"([a-z_]+)"/g)].map((m) => m[1]);

    it("reads srv's list", () => {
        expect(kinds.length).toBeGreaterThan(0);
    });

    it("gives each kind a part and a place to answer it", () => {
        for (const kind of kinds) expect(KIND_PLACES[kind], `kind ${kind} has no part in the summary`).toBeDefined();
    });

    it("has no part for a kind srv no longer announces", () => {
        expect(Object.keys(KIND_PLACES).sort()).toEqual([...kinds].sort());
    });
});
