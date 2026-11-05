// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The kit Select shows its `value` from the first render, whichever form its choices come in. Given as `<option>`
 * children (Settings > Recording's Engine, Model and Input device), they are not in the element yet when the value is
 * first set, and the browser used to fall back to the first one.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal, For } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Select } from "./inputs";

afterEach(() => cleanup());

const select = () => document.querySelector("select") as HTMLSelectElement;

describe("Select shows its value on first render", () => {
    it("with its options as children", () => {
        render(() => (
            <Select value="b" onChange={vi.fn()} ariaLabel="Engine">
                <option value="a">A</option>
                <option value="b">B</option>
                <option value="c">C</option>
            </Select>
        ));
        expect(select().value).toBe("b");
    });

    it("with its options as children made by a list", () => {
        render(() => (
            <Select value="c" onChange={vi.fn()} ariaLabel="Model">
                <For each={["a", "b", "c"]}>{(v) => <option value={v}>{v}</option>}</For>
            </Select>
        ));
        expect(select().value).toBe("c");
    });

    it("with its options in the options prop", () => {
        render(() => (
            <Select
                value="b"
                onChange={vi.fn()}
                ariaLabel="Engine"
                options={[
                    { value: "a", label: "A" },
                    { value: "b", label: "B" },
                ]}
            />
        ));
        expect(select().value).toBe("b");
    });

    it("keeps showing the value when the children arrive later, and follows it when it changes", async () => {
        const [value, setValue] = createSignal("b");
        const [choices, setChoices] = createSignal<string[]>([]);
        render(() => (
            <Select value={value()} onChange={vi.fn()} ariaLabel="Input device">
                <For each={choices()}>{(v) => <option value={v}>{v}</option>}</For>
            </Select>
        ));
        setChoices(["a", "b", "c"]);
        await Promise.resolve();
        await new Promise((r) => setTimeout(r, 0));
        expect(select().value).toBe("b");
        setValue("c");
        await Promise.resolve();
        expect(select().value).toBe("c");
    });
});
