// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, render } from "@solidjs/testing-library";
import { createEffect, createSignal, ErrorBoundary, Show } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { containBindingErrors } from "./contain-binding-errors";

afterEach(() => cleanup());

describe("containBindingErrors", () => {
    it("lets the other bindings of a flush run when one throws, and reports the error", () => {
        const [n, setN] = createSignal(0);
        const seen: number[] = [];
        const report = vi.fn();
        render(() =>
            containBindingErrors(() => {
                createEffect(() => {
                    if (n() === 1) throw new Error("Stale read from <Show>.");
                });
                createEffect(() => void seen.push(n()));
                return <div>app</div>;
            }, report)
        );
        setN(1);
        setN(2);
        expect(seen).toEqual([0, 1, 2]);
        expect(report).toHaveBeenCalledTimes(1);
        expect(report.mock.calls[0][0]).toContain("Stale read from <Show>.");
    });

    it("without it, the same throw abandons the effects queued after it", () => {
        const [n, setN] = createSignal(0);
        const seen: number[] = [];
        render(() => {
            createEffect(() => {
                if (n() === 1) throw new Error("boom");
            });
            createEffect(() => void seen.push(n()));
            return <div>app</div>;
        });
        expect(() => setN(1)).toThrow("boom");
        expect(seen).toEqual([0]);
    });

    it("leaves an error boundary below to show its own fallback", () => {
        const report = vi.fn();
        const [boom, setBoom] = createSignal(false);
        const { container } = render(() =>
            containBindingErrors(
                () => (
                    <ErrorBoundary fallback={<p>pane failed</p>}>
                        <Show when={true}>
                            {(() => {
                                createEffect(() => {
                                    if (boom()) throw new Error("inside");
                                });
                                return <span>pane</span>;
                            })()}
                        </Show>
                    </ErrorBoundary>
                ),
                report
            )
        );
        setBoom(true);
        expect(container.textContent).toBe("pane failed");
        expect(report).not.toHaveBeenCalled();
    });
});
