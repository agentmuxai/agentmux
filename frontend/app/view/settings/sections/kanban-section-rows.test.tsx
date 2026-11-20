// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Kanban's behaviour rows (docs/specs/SPEC_KANBAN_CUSTOM_STAGES_2026_11_16.md §8.2): a switch for closing a task when its
// PR merges (on unless the setting is false), and Review's limit as a number (0 is none), each writing its own setting.

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ writes: [] as string[], settings: undefined as unknown, arg: 0 as number }));

vi.mock("@/app/store/global", () => ({ settingsAtom: () => h.settings }));
vi.mock("../settings-controls", async (orig) => {
    const actual = await orig<typeof import("../settings-controls")>();
    return {
        ...actual,
        set: (key: string, value: unknown) => h.writes.push(`${key} ${JSON.stringify(value)}`),
        ToggleControl: (props: { checked: boolean; onChange: (v: boolean) => void }) => (
            <button type="button" data-stand="toggle" data-checked={String(props.checked)} onClick={() => props.onChange(!props.checked)}>
                toggle
            </button>
        ),
        NumberControl: (props: { value: number; onChange: (v: number) => void }) => (
            <button type="button" data-stand="number" data-value={String(props.value)} onClick={() => props.onChange(h.arg)}>
                number
            </button>
        ),
    };
});

import { closeOnMergeOn, KanbanBehaviourRows } from "./kanban-section-rows";

afterEach(() => {
    cleanup();
    h.writes.length = 0;
    h.settings = undefined;
});

const toggle = () => document.querySelector<HTMLElement>('[data-stand="toggle"]')!;
const number = () => document.querySelector<HTMLElement>('[data-stand="number"]')!;

describe("Kanban behaviour rows", () => {
    it("says what each does, under a Behaviour heading", () => {
        render(() => <KanbanBehaviourRows />);
        expect(screen.getByText("Behaviour")).toBeTruthy();
        expect(screen.getByText("Close a task when its PR merges")).toBeTruthy();
        expect(screen.getByText("Review limit")).toBeTruthy();
        expect(screen.getByText(/the stage that shows Review turns amber/)).toBeTruthy();
    });

    it("shows closing on merge as on unless the setting is false, and writes the switch", () => {
        expect(closeOnMergeOn(undefined)).toBe(true);
        expect(closeOnMergeOn({ "kanban:closeonmerge": true })).toBe(true);
        expect(closeOnMergeOn({ "kanban:closeonmerge": false })).toBe(false);
        h.settings = { "kanban:closeonmerge": false };
        render(() => <KanbanBehaviourRows />);
        expect(toggle().dataset.checked).toBe("false");
        fireEvent.click(toggle());
        expect(h.writes).toEqual(["kanban:closeonmerge true"]);
    });

    it("shows the limit as the board reads it (a positive whole number, else 0) and writes a whole number", () => {
        h.settings = { "kanban:reviewlimit": 4 };
        render(() => <KanbanBehaviourRows />);
        expect(number().dataset.value).toBe("4");
        h.arg = 6.7;
        fireEvent.click(number());
        h.arg = -3;
        fireEvent.click(number());
        expect(h.writes).toEqual(["kanban:reviewlimit 6", "kanban:reviewlimit 0"]);
    });

    it("shows no limit (0) when the setting is unset or not a number", () => {
        h.settings = { "kanban:reviewlimit": "4" };
        render(() => <KanbanBehaviourRows />);
        expect(number().dataset.value).toBe("0");
    });
});
