// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * patchRuntime — runtime changes from the menu and the slash commands must not
 * overwrite each other.
 *
 * Every writer used to build its new config from `getRuntimeConfig(meta)` at
 * click time and hand it to `applyRuntimeChange`. The Runtime panel stays open
 * across selections on purpose, so two changes can land before the first one's
 * meta write has come back: the second read the old config, and its write
 * silently undid the first — in `agent:runtime` AND in `cmd:args`, so the menu
 * and the process agreed with each other and with neither of the user's clicks.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const setMeta = vi.fn();
const resync = vi.fn();
let setMetaGate: Promise<void> | null = null;

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        SetMetaCommand: (_c: unknown, data: any) => {
            setMeta(data);
            return setMetaGate ?? Promise.resolve();
        },
        ControllerResyncCommand: (_c: unknown, data: any) => {
            resync(data);
            return Promise.resolve();
        },
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mos", () => ({ makeORef: (t: string, id: string) => `${t}:${id}` }));
vi.mock("@/app/store/global", () => ({ staticTabId: () => "tab-1" }));

import { patchRuntime, __resetRuntimeApply } from "./runtime-apply";
import type { ProviderDefinition } from "./providers";
import type { AgentRuntimeConfig } from "./types";

const claude = {
    id: "claude",
    controllerType: "persistent",
    launchArgs: ["-p"],
    persistentLaunchArgs: ["--permission-prompt-tool", "stdio", "--permission-mode", "default"],
} as unknown as ProviderDefinition;

const base: AgentRuntimeConfig = { permissionMode: "bypass", model: "sonnet", effort: "high" };
const meta = (r: AgentRuntimeConfig = base) => ({ agentMode: "host", "agent:runtime": r });

/** The `agent:runtime` values written, in order. */
const runtimeWrites = (): AgentRuntimeConfig[] =>
    setMeta.mock.calls.map((c) => c[0].meta["agent:runtime"]).filter(Boolean);
/** The `cmd:args` written, in order. */
const argWrites = (): string[][] => setMeta.mock.calls.map((c) => c[0].meta["cmd:args"]).filter(Boolean);
const flag = (args: string[], f: string) => args[args.indexOf(f) + 1];

beforeEach(() => {
    setMeta.mockClear();
    resync.mockClear();
    setMetaGate = null;
    __resetRuntimeApply();
});
afterEach(() => vi.useRealTimers());

describe("patchRuntime", () => {
    it("two quick changes both land, even though the menu still reads the old config", async () => {
        // `getMeta` never updates: exactly the window before the first write
        // has round-tripped back into the block.
        const stale = () => meta();
        const a = patchRuntime("b1", claude, { model: "opus" }, stale);
        const b = patchRuntime("b1", claude, { effort: "max" }, stale);
        await Promise.all([a, b]);

        const last = runtimeWrites().at(-1)!;
        expect(last).toMatchObject({ model: "opus", effort: "max" });
        const args = argWrites().at(-1)!;
        expect(flag(args, "--model")).toBe("opus");
        expect(flag(args, "--effort")).toBe("max");
    });

    it("applies changes one at a time, in the order they were made", async () => {
        const order: string[] = [];
        resync.mockImplementation((d) => order.push(`resync:${d.blockid}`));
        await Promise.all([
            patchRuntime("b1", claude, { model: "opus" }, () => meta()),
            patchRuntime("b1", claude, { model: "haiku" }, () => meta()),
        ]);
        const models = runtimeWrites().map((r) => r.model);
        expect(models).toEqual(["opus", "haiku"]);
        expect(order).toHaveLength(2);
    });

    it("a failed change does not poison the next one", async () => {
        resync.mockImplementationOnce(() => {
            throw new Error("resync refused");
        });
        const first = patchRuntime("b1", claude, { model: "opus" }, () => meta());
        const second = patchRuntime("b1", claude, { effort: "low" }, () => meta());
        await expect(first).rejects.toThrow("resync refused");
        await expect(second).resolves.toMatchObject({ effort: "low" });
        // The failed model change is NOT carried into the second: the menu would
        // otherwise show a model that was never applied.
        expect(runtimeWrites().at(-1)).toMatchObject({ model: "sonnet", effort: "low" });
    });

    it("keeps building on its own last write while the block's meta catches up", async () => {
        vi.useFakeTimers();
        await patchRuntime("b1", claude, { model: "opus" }, () => meta());
        // meta still stale a moment later:
        await patchRuntime("b1", claude, { effort: "max" }, () => meta());
        expect(runtimeWrites().at(-1)).toMatchObject({ model: "opus", effort: "max" });
    });

    it("trusts the block's meta again once that moment has passed", async () => {
        vi.useFakeTimers();
        await patchRuntime("b1", claude, { model: "opus" }, () => meta());
        vi.advanceTimersByTime(10_000);
        // someone else changed it meanwhile (another window, a slash command):
        await patchRuntime("b1", claude, { effort: "low" }, () => meta({ ...base, model: "haiku" }));
        expect(runtimeWrites().at(-1)).toMatchObject({ model: "haiku", effort: "low" });
    });

    it("blocks are independent", async () => {
        await Promise.all([
            patchRuntime("b1", claude, { model: "opus" }, () => meta()),
            patchRuntime("b2", claude, { effort: "low" }, () => meta()),
        ]);
        const byBlock = (id: string) =>
            setMeta.mock.calls.filter((c) => c[0].oref === `block:${id}`).map((c) => c[0].meta["agent:runtime"]).filter(Boolean);
        expect(byBlock("b1").at(-1)).toMatchObject({ model: "opus", effort: "high" });
        expect(byBlock("b2").at(-1)).toMatchObject({ model: "sonnet", effort: "low" });
    });
});
