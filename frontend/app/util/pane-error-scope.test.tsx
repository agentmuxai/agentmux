// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/*
 * A handler in a pane's own reactive scope: a throw there is logged with the pane and the part of the pane, and
 * either handed to the handler above (a boundary that shows a fallback) or ended there (a scope with no owner).
 */

import { catchError, createEffect, createRoot, createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const sendLogStructured = vi.fn();
vi.mock("@/util/app-api", () => ({ getApi: () => ({ sendLogStructured }) }));

import { catchPaneErrors, reportPaneError } from "./pane-error-scope";

let dispose: (() => void) | null = null;
const inRoot = <T,>(fn: () => T): T =>
    createRoot((d) => {
        dispose = d;
        return fn();
    });

beforeEach(() => sendLogStructured.mockClear());
afterEach(() => {
    dispose?.();
    dispose = null;
});

const logged = () => sendLogStructured.mock.calls.map((c) => ({ level: c[0], module: c[1], message: c[2] as string, data: c[3] }));

describe("catchPaneErrors, passing on", () => {
    it("logs the pane and the scope, then hands the error to the handler above", () => {
        const above = vi.fn();
        const [boom, setBoom] = createSignal(false);
        inRoot(() =>
            catchError(
                () =>
                    catchPaneErrors("blk-1234567890", "body", () => {
                        createEffect(() => {
                            if (boom()) throw new Error("the reader threw");
                        });
                    }),
                above
            )
        );
        setBoom(true);
        expect(above).toHaveBeenCalledTimes(1);
        expect((above.mock.calls[0][0] as Error).message).toBe("the reader threw");
        const [line] = logged();
        expect(logged()).toHaveLength(1);
        expect(line.level).toBe("error");
        expect(line.message).toContain("the reader threw");
        expect(line.message).toContain("block=blk-123");
        expect(line.message).toContain("in=body");
        expect(line.data).toMatchObject({ block_id: "blk-1234567890", scope: "body", error_name: "Error" });
    });

    it("writes one line for an error passed up through nested scopes, from the nearest", () => {
        const [boom, setBoom] = createSignal(false);
        inRoot(() =>
            catchError(
                () =>
                    catchPaneErrors("blk-1", "chrome", () =>
                        catchPaneErrors("blk-1", "body", () => {
                            createEffect(() => {
                                if (boom()) throw new Error("once");
                            });
                        })
                    ),
                () => {}
            )
        );
        setBoom(true);
        expect(logged()).toHaveLength(1);
        expect(logged()[0].message).toContain("in=body");
    });

    it("still throws a throw in the build itself to its caller, after logging it", () => {
        expect(() =>
            inRoot(() =>
                catchPaneErrors("blk-2", "chrome", () => {
                    throw new Error("while building");
                })
            )
        ).toThrow("while building");
        expect(logged()).toHaveLength(1);
        expect(logged()[0].message).toContain("in=chrome");
    });
});

describe("catchPaneErrors, containing", () => {
    it("ends a throw from an effect made in the scope there, and the flush goes on to the other effects", () => {
        const [boom, setBoom] = createSignal(false);
        const [n, setN] = createSignal(0);
        const seen: number[] = [];
        inRoot(() => {
            catchPaneErrors(
                "blk-3",
                "agent view model",
                () => {
                    createEffect(() => {
                        if (boom()) throw new Error("no handler above");
                    });
                },
                "contain"
            );
            createEffect(() => seen.push(n()));
        });
        expect(() => {
            setBoom(true);
            setN(1);
        }).not.toThrow();
        expect(seen).toEqual([0, 1]);
        expect(logged()).toHaveLength(1);
        expect(logged()[0].message).toContain("in=agent view model");
    });

    it("lets a throw while the scope is being built reach whoever builds it, and logs it", () => {
        expect(() =>
            inRoot(() =>
                catchPaneErrors(
                    "blk-4",
                    "x view model",
                    () => {
                        throw new Error("constructor failed");
                    },
                    "contain"
                )
            )
        ).toThrow("constructor failed");
        expect(logged()).toHaveLength(1);
    });
});

describe("reportPaneError", () => {
    it("names a thrown string (Solid's stale read from <Show>) as an error with that text", () => {
        inRoot(() => {
            expect(reportPaneError("blk-5", "body", "Stale read from <Show>.")).toBe(true);
        });
        expect(logged()[0].message).toContain("Stale read from <Show>.");
    });

    it("says it was already logged the second time for the same error object", () => {
        const e = new Error("same");
        expect(reportPaneError("blk-6", "body", e)).toBe(true);
        expect(reportPaneError("blk-6", "chrome", e)).toBe(false);
        expect(logged()).toHaveLength(1);
    });

    it("never throws when the log itself fails", () => {
        sendLogStructured.mockImplementationOnce(() => {
            throw new Error("host gone");
        });
        expect(() => reportPaneError("blk-7", "body", new Error("x"))).not.toThrow();
    });
});
