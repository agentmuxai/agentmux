// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const sent: string[] = [];
let answer: (command: string) => unknown = () => ({ ok: true });

vi.mock("@/app/store/rpc-util-base", () => ({
    sendRpcCommand: (_open: unknown, msg: RpcMessage) => {
        sent.push(msg.command);
        const gen = (async function* () {
            yield null;
            const v = answer(msg.command);
            if (v instanceof Error) throw v;
            return v;
        })();
        void gen.next(); // primed, as the real one is
        return gen;
    },
    sendRpcResponse: () => {},
}));

import {
    expectOperatorProof,
    isOperatorCommand,
    operatorProofReady,
    proveOperatorConnection,
    resetOperatorProofForTests,
    sendAsOperator,
    setOperatorRefusalHandler,
    sendRetryingUnproven,
} from "./operator-proof";
import { RpcClient } from "./rpc-client";

const UNPROVEN = "FORBIDDEN: only the operator acts on Kanban tasks (this connection isn't the AgentMux window)";

function deferred<T = void>() {
    let resolve!: (v: T) => void;
    const promise = new Promise<T>((r) => (resolve = r));
    return { promise, resolve };
}

beforeEach(() => {
    resetOperatorProofForTests();
    sent.length = 0;
    answer = () => ({ ok: true });
});

afterEach(() => {
    vi.useRealTimers();
});

describe("operator proof", () => {
    it("hands the host the nonce srv issued on this connection", async () => {
        const attest = vi.fn(async (_nonce: string) => {});
        const proven = await proveOperatorConnection({ challenge: async () => ({ nonce: "n-1" }), attest });
        expect(proven).toBe(true);
        expect(attest).toHaveBeenCalledWith("n-1");
        await expect(operatorProofReady()).resolves.toBe(true);
    });

    it("does nothing without a host to vouch", async () => {
        const challenge = vi.fn(async () => ({ nonce: "n" }));
        await expect(proveOperatorConnection({ challenge })).resolves.toBe(false);
        expect(challenge).not.toHaveBeenCalled();
        await expect(operatorProofReady()).resolves.toBe(false);
    });

    it("reports a refused proof as unproven instead of throwing", async () => {
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        const proven = await proveOperatorConnection({
            challenge: async () => ({ nonce: "n" }),
            attest: async () => {
                throw new Error("srv refused the proof");
            },
        });
        expect(proven).toBe(false);
        warn.mockRestore();
    });

    it("holds operator RPCs made before the first connection is proven", async () => {
        expectOperatorProof();
        const send = vi.fn(async () => "done");
        const result = sendAsOperator(send);
        await Promise.resolve();
        expect(send).not.toHaveBeenCalled();

        const hostAnswered = deferred();
        void proveOperatorConnection({ challenge: async () => ({ nonce: "n" }), attest: () => hostAnswered.promise });
        await Promise.resolve();
        expect(send).not.toHaveBeenCalled();
        hostAnswered.resolve();
        await expect(result).resolves.toBe("done");
        expect(send).toHaveBeenCalledTimes(1);
    });

    it("sends anyway once the wait for a proof runs out", async () => {
        vi.useFakeTimers();
        expectOperatorProof();
        const send = vi.fn(async () => "sent");
        const result = sendAsOperator(send);
        await vi.advanceTimersByTimeAsync(10_000);
        await expect(result).resolves.toBe("sent");
    });

    it("sends once more when a reconnect started a new connection meanwhile", async () => {
        await proveOperatorConnection({ challenge: async () => ({ nonce: "a" }), attest: async () => {} });
        let calls = 0;
        const result = sendAsOperator(async () => {
            calls++;
            if (calls === 1) {
                // The socket reconnected before this reached srv.
                void proveOperatorConnection({ challenge: async () => ({ nonce: "b" }), attest: async () => {} });
                throw new Error(UNPROVEN);
            }
            return "ok";
        });
        await expect(result).resolves.toBe("ok");
        expect(calls).toBe(2);
    });

    it("proves the same connection again when its proof lapsed, and sends once more, once", async () => {
        const attest = vi.fn(async (_nonce: string) => {});
        await proveOperatorConnection({ challenge: async () => ({ nonce: "a" }), attest });
        let calls = 0;
        const send = vi.fn(async () => {
            calls++;
            if (calls === 1) throw new Error(UNPROVEN);
            return "ok";
        });
        await expect(sendAsOperator(send)).resolves.toBe("ok");
        expect(send).toHaveBeenCalledTimes(2);
        expect(attest).toHaveBeenCalledTimes(2);
    });

    it("sends no more than once more when the refusal stands, and tells the person", async () => {
        const seen: string[] = [];
        setOperatorRefusalHandler((r) => seen.push(`${r.unproven}:${r.message}`));
        await proveOperatorConnection({ challenge: async () => ({ nonce: "a" }), attest: async () => {} });
        const send = vi.fn(async () => {
            throw new Error(UNPROVEN);
        });
        await expect(sendAsOperator(send)).rejects.toThrow(UNPROVEN);
        expect(send).toHaveBeenCalledTimes(2);
        expect(seen).toEqual([`true:${UNPROVEN}`]);
    });

    it("tells the person when the proof cannot be made again, without sending twice", async () => {
        const seen: string[] = [];
        setOperatorRefusalHandler((r) => seen.push(r.message));
        let ok = true;
        await proveOperatorConnection({
            challenge: async () => ({ nonce: "a" }),
            attest: async () => {
                if (!ok) throw new Error("signed out");
            },
        });
        ok = false;
        const send = vi.fn(async () => {
            throw new Error(UNPROVEN);
        });
        await expect(sendAsOperator(send)).rejects.toThrow(UNPROVEN);
        expect(send).toHaveBeenCalledTimes(1);
        expect(seen).toHaveLength(1);
    });

    it("hands any operator-only refusal to the handler, and no other error", async () => {
        const seen: string[] = [];
        setOperatorRefusalHandler((r) => seen.push(`${r.unproven}:${r.message}`));
        await proveOperatorConnection({ challenge: async () => ({ nonce: "a" }), attest: async () => {} });
        await expect(
            sendAsOperator(async () => {
                throw new Error("FORBIDDEN: change the seat");
            })
        ).rejects.toThrow("change the seat");
        await expect(
            sendAsOperator(async () => {
                throw new Error("timeout");
            })
        ).rejects.toThrow("timeout");
        expect(seen).toEqual(["false:FORBIDDEN: change the seat"]);
    });

    it("proves a lapsed connection again once when several sends are refused together", async () => {
        const attest = vi.fn(async (_nonce: string) => {});
        await proveOperatorConnection({ challenge: async () => ({ nonce: "a" }), attest });
        const make = () => {
            let calls = 0;
            return vi.fn(async () => {
                calls++;
                if (calls === 1) throw new Error(UNPROVEN);
                return "ok";
            });
        };
        await expect(Promise.all([sendAsOperator(make()), sendAsOperator(make()), sendAsOperator(make())])).resolves.toEqual(["ok", "ok", "ok"]);
        expect(attest).toHaveBeenCalledTimes(2);
    });

    it("resends an unlisted command once srv says the connection is unproven and it is then proven", async () => {
        await proveOperatorConnection({ challenge: async () => ({ nonce: "a" }), attest: async () => {} });
        let calls = 0;
        const send = vi.fn(async () => {
            calls++;
            if (calls === 1) throw new Error(UNPROVEN);
            return "ok";
        });
        await expect(sendRetryingUnproven(send)).resolves.toBe("ok");
        expect(send).toHaveBeenCalledTimes(2);
        const other = vi.fn(async () => {
            throw new Error("FORBIDDEN: something else");
        });
        await expect(sendRetryingUnproven(other)).rejects.toThrow("something else");
        expect(other).toHaveBeenCalledTimes(1);
    });

    it("proves a lapsed connection again for an unlisted command too, though it was proven before", async () => {
        const attest = vi.fn(async (_nonce: string) => {});
        await proveOperatorConnection({ challenge: async () => ({ nonce: "a" }), attest });
        let calls = 0;
        const send = vi.fn(async () => {
            calls++;
            if (calls === 1) throw new Error(UNPROVEN);
            return "ok";
        });
        await expect(sendRetryingUnproven(send)).resolves.toBe("ok");
        expect(attest).toHaveBeenCalledTimes(2);
    });

    it("does not prove again with an earlier connection's host while a new connection's proof is expected", async () => {
        const attest = vi.fn(async (_nonce: string) => {});
        await proveOperatorConnection({ challenge: async () => ({ nonce: "a" }), attest });
        expectOperatorProof();
        vi.useFakeTimers();
        const send = vi.fn(async () => {
            throw new Error(UNPROVEN);
        });
        const result = sendAsOperator(send);
        const settled = expect(result).rejects.toThrow(UNPROVEN);
        await vi.advanceTimersByTimeAsync(10_000);
        await settled;
        expect(attest).toHaveBeenCalledTimes(1);
    });

    it("names the commands srv keeps for the operator, and only those", () => {
        for (const c of [
            "kanban.act",
            "jobs.act",
            "jobs.save",
            "jobs.templates.approve",
            "agent.setSeat",
            "identity.account.move_agents",
            "tower.sample",
            "gpu.sample",
            "jobs.templates.install",
            "jobs.templates.uninstall",
            "jobs.templates.catalog",
            "jobs.templates.catalog.install",
            "fleet.broadcast",
            "agent.pane.unregister",
            "agent.takeover",
            "agent.move-here",
        ]) {
            expect(isOperatorCommand(c)).toBe(true);
        }
        for (const c of ["kanban.list", "kanban.task", "operator.challenge", "jobs.list", "jobs.dryrun", "jobs.get"]) {
            expect(isOperatorCommand(c)).toBe(false);
        }
    });
});

describe("RpcClient and the proof", () => {
    it("waits for the proof before sending an operator RPC, and not for others", async () => {
        expectOperatorProof();
        const client = new RpcClient("tab:test");
        const act = client.rpcCall("kanban.act", { action: "create" }, {});
        const list = client.rpcCall("kanban.list", {}, {});
        await Promise.resolve();
        expect(sent).toEqual(["kanban.list"]);
        await list;

        await proveOperatorConnection({ challenge: async () => ({ nonce: "n" }), attest: async () => {} });
        await act;
        expect(sent).toEqual(["kanban.list", "kanban.act"]);
    });
});
