// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom
//
// Tests for the Animations page (docs/specs/SPEC_ANIMATIONS_SETTINGS_PAGE_2026_11_01.md §4, §8).

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const setConfig = vi.fn();
let settings: Record<string, unknown> = {};

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { SetConfigCommand: (...args: unknown[]) => setConfig(...args) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/global", () => ({ settingsAtom: () => settings }));

import { initMotion, setAnimationSettings } from "@/app/motion/motion";
import { AnimationsSection, ANIMATIONS_SETTINGS } from "./animations-section";

/** Settings as the page and the motion layer see them. */
function use(next: Record<string, unknown>, osReduces = false) {
    settings = next;
    initMotion({
        matchMedia: () => ({ matches: osReduces, addEventListener() {}, removeEventListener() {} }),
        document: { documentElement: { dataset: {}, style: { setProperty() {} } } },
    } as unknown as Window);
    setAnimationSettings(next);
}

/** The Reduce movement switch (the preview has a switch of its own). */
const reduceSwitch = () => document.getElementById("setting-animations.reduce_movement")?.querySelector('[role="switch"]') ?? null;

const written = () => setConfig.mock.calls.map((c) => c[1]);

beforeEach(() => {
    setConfig.mockReset();
    use({});
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
});

describe("the choice", () => {
    it("offers Follow system, Off, Quick, Smooth and Slow, with Smooth selected by default", () => {
        render(() => <AnimationsSection />);
        const labels = screen.getAllByRole("radio").map((r) => r.textContent);
        expect(labels).toEqual(["Follow system", "Off", "Quick", "Smooth", "Slow"]);
        expect(screen.getByRole("radio", { name: "Smooth" })).toBeChecked();
    });

    it("writes only the preset when one is chosen, so the custom numbers are kept", () => {
        render(() => <AnimationsSection />);
        fireEvent.click(screen.getByRole("radio", { name: "Quick" }));
        expect(written()).toEqual([{ "animation:preset": "quick" }]);
    });

    it("shows Custom, selected, only while it is the choice", () => {
        use({ "animation:preset": "custom", "animation:speed": 80 });
        render(() => <AnimationsSection />);
        expect(screen.getByRole("radio", { name: "Custom" })).toBeChecked();
    });

    it("says what the choice does", () => {
        use({ "animation:preset": "off" });
        render(() => <AnimationsSection />);
        expect(screen.getByTestId("animations-help").textContent).toBe("Nothing moves. Changes happen instantly.");
    });
});

describe("the readout", () => {
    it.each([
        [{}, "Menus open in 160 ms"],
        [{ "animation:preset": "quick" }, "Menus open in 80 ms"],
        [{ "animation:preset": "slow" }, "Menus open in 240 ms"],
        [{ "animation:preset": "off" }, "Menus open instantly"],
    ])("for %j says %s", (s, text) => {
        use(s);
        render(() => <AnimationsSection />);
        expect(screen.getByTestId("animations-readout").textContent).toBe(text);
    });
});

describe("the numbers", () => {
    it("editing one turns the choice to Custom and keeps the others the page showed", () => {
        use({ "animation:preset": "quick" });
        render(() => <AnimationsSection />);
        fireEvent.click(reduceSwitch()!);
        expect(written()).toEqual([
            {
                "animation:preset": "custom",
                "animation:speed": 50,
                "animation:polish": 100,
                "animation:reducemovement": true,
            },
        ]);
    });

    it("snaps a slider value to the range and says so", () => {
        vi.useFakeTimers();
        render(() => <AnimationsSection />);
        const slider = screen.getAllByRole("slider")[0] as HTMLInputElement;
        fireEvent.input(slider, { target: { value: "10" } });
        vi.advanceTimersByTime(200);
        expect(written()[0]).toMatchObject({ "animation:preset": "custom", "animation:speed": 25 });
        expect(screen.getByTestId("animations-snap").textContent).toContain("using 25");
    });

    it("shows the numbers Follow system uses, read-only", () => {
        use({ "animation:preset": "system" });
        render(() => <AnimationsSection />);
        expect(screen.queryAllByRole("slider")).toHaveLength(0);
        expect(reduceSwitch()).toBeNull();
        expect(screen.getAllByText("100%")).toHaveLength(2);
    });

    it("Reset to default writes Smooth and 100, 100, off", () => {
        use({ "animation:preset": "custom", "animation:speed": 200 });
        render(() => <AnimationsSection />);
        fireEvent.click(screen.getByTestId("animations-reset"));
        expect(written()).toEqual([
            {
                "animation:preset": "smooth",
                "animation:speed": 100,
                "animation:polish": 100,
                "animation:reducemovement": false,
            },
        ]);
    });

    it("Use my custom values returns to the stored numbers", () => {
        use({ "animation:preset": "quick", "animation:speed": 200 });
        render(() => <AnimationsSection />);
        fireEvent.click(screen.getByTestId("animations-use-custom"));
        expect(written()).toEqual([{ "animation:preset": "custom" }]);
    });
});

describe("the system notice", () => {
    it("appears when the system asks for reduced motion and another choice is made, and Follow system answers it", () => {
        use({}, true);
        render(() => <AnimationsSection />);
        expect(screen.getByText(/Your system asks for reduced motion\. AgentMux is not following it because you chose Smooth\./)).toBeInTheDocument();
        fireEvent.click(screen.getByTestId("animations-follow-system"));
        expect(written()).toEqual([{ "animation:preset": "system" }]);
    });

    it("is absent when the system asks for nothing, or the choice is Follow system", () => {
        render(() => <AnimationsSection />);
        expect(screen.queryByTestId("animations-follow-system")).toBeNull();
        cleanup();
        use({ "animation:preset": "system" }, true);
        render(() => <AnimationsSection />);
        expect(screen.queryByTestId("animations-follow-system")).toBeNull();
    });
});

describe("the preview", () => {
    it("says there is no animation under Off, and not otherwise", () => {
        use({ "animation:preset": "off" });
        render(() => <AnimationsSection />);
        expect(screen.getByText("No animation")).toBeInTheDocument();
        cleanup();
        use({});
        render(() => <AnimationsSection />);
        expect(screen.queryByText("No animation")).toBeNull();
    });
});

describe("search rows", () => {
    it("each row the page renders has its index id", () => {
        render(() => <AnimationsSection />);
        for (const entry of Object.values(ANIMATIONS_SETTINGS)) {
            expect(document.getElementById(`setting-${entry.id}`), entry.id).not.toBeNull();
        }
    });
});
