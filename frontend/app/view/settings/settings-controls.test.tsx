// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Tests for NumberControl — see SPEC_SETTINGS_LIVE_COMMIT_AND_TERMINAL_APPLY_GAPS_2026_09_22.md
 * §2 for the bug this exists to fix: every hand-rolled `<input
 * type="number" onBlur={...}>` in the settings UI only committed on blur,
 * so a setting never took effect until the user clicked away from the
 * field — confirmed live, not assumed (that spec's §2 table).
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { NumberControl } from "./settings-controls";

afterEach(() => cleanup());

describe("NumberControl", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    it("does not commit while still typing, within the debounce window", () => {
        const onChange = vi.fn();
        const { getByRole } = render(() => <NumberControl min={0.1} max={10} step={0.1} value={1} onChange={onChange} />);
        const input = getByRole("spinbutton") as HTMLInputElement;

        fireEvent.input(input, { target: { value: "7" } });
        vi.advanceTimersByTime(200); // less than the 400ms debounce

        expect(onChange).not.toHaveBeenCalled();
    });

    it("commits after the debounce window elapses with no further input", () => {
        const onChange = vi.fn();
        const { getByRole } = render(() => <NumberControl min={0.1} max={10} step={0.1} value={1} onChange={onChange} />);
        const input = getByRole("spinbutton") as HTMLInputElement;

        fireEvent.input(input, { target: { value: "7" } });
        vi.advanceTimersByTime(450); // past the 400ms debounce, comfortable margin

        expect(onChange).toHaveBeenCalledExactlyOnceWith(7);
    });

    it("restarts the debounce on each keystroke, committing only the final value", () => {
        const onChange = vi.fn();
        const { getByRole } = render(() => <NumberControl min={0.1} max={10} step={0.1} value={1} onChange={onChange} />);
        const input = getByRole("spinbutton") as HTMLInputElement;

        fireEvent.input(input, { target: { value: "7" } });
        vi.advanceTimersByTime(200);
        fireEvent.input(input, { target: { value: "7.5" } });
        vi.advanceTimersByTime(200); // 400ms since the first keystroke, but only 200ms since the second
        expect(onChange).not.toHaveBeenCalled();

        vi.advanceTimersByTime(250); // now past 400ms since the SECOND keystroke
        expect(onChange).toHaveBeenCalledExactlyOnceWith(7.5);
    });

    it("flushes immediately on blur instead of waiting out the debounce", () => {
        // The one case SliderControl's own debounce (this component's
        // template) doesn't need an equivalent for — a range input has no
        // "half-typed" state a user can tab away from mid-debounce.
        const onChange = vi.fn();
        const { getByRole } = render(() => <NumberControl min={0.1} max={10} step={0.1} value={1} onChange={onChange} />);
        const input = getByRole("spinbutton") as HTMLInputElement;

        fireEvent.input(input, { target: { value: "3" } });
        fireEvent.blur(input);

        expect(onChange).toHaveBeenCalledExactlyOnceWith(3);
    });

    it("blurring after the debounce already fired does not commit a second time", () => {
        const onChange = vi.fn();
        const { getByRole } = render(() => <NumberControl min={0.1} max={10} step={0.1} value={1} onChange={onChange} />);
        const input = getByRole("spinbutton") as HTMLInputElement;

        fireEvent.input(input, { target: { value: "3" } });
        vi.advanceTimersByTime(450);
        fireEvent.blur(input);

        expect(onChange).toHaveBeenCalledOnce();
    });

    it("silently ignores an out-of-range value, on both input and blur, same as every existing onBlur guard", () => {
        const onChange = vi.fn();
        const { getByRole } = render(() => <NumberControl min={0.1} max={10} step={0.1} value={1} onChange={onChange} />);
        const input = getByRole("spinbutton") as HTMLInputElement;

        fireEvent.input(input, { target: { value: "50" } }); // over max
        vi.advanceTimersByTime(450);
        fireEvent.blur(input);

        expect(onChange).not.toHaveBeenCalled();
    });

    it("silently ignores a non-numeric value", () => {
        const onChange = vi.fn();
        const { getByRole } = render(() => <NumberControl min={0.1} max={10} step={0.1} value={1} onChange={onChange} />);
        const input = getByRole("spinbutton") as HTMLInputElement;

        fireEvent.input(input, { target: { value: "abc" } });
        vi.advanceTimersByTime(450);

        expect(onChange).not.toHaveBeenCalled();
    });

    it("treats an absent max as unbounded above", () => {
        const onChange = vi.fn();
        const { getByRole } = render(() => <NumberControl min={1} step={1} value={5} onChange={onChange} />);
        const input = getByRole("spinbutton") as HTMLInputElement;

        fireEvent.input(input, { target: { value: "999999" } });
        vi.advanceTimersByTime(450);

        expect(onChange).toHaveBeenCalledExactlyOnceWith(999999);
    });

    it('parse="int" truncates a fractional value, matching every existing parseInt site', () => {
        const onChange = vi.fn();
        const { getByRole } = render(() => (
            <NumberControl min={1000} max={100000} step={1000} parse="int" value={10000} onChange={onChange} />
        ));
        const input = getByRole("spinbutton") as HTMLInputElement;

        fireEvent.input(input, { target: { value: "4200.9" } });
        vi.advanceTimersByTime(450);

        expect(onChange).toHaveBeenCalledExactlyOnceWith(4200);
    });

    it("defaults to float parsing when parse is omitted", () => {
        const onChange = vi.fn();
        const { getByRole } = render(() => <NumberControl min={0} step={0.1} value={1} onChange={onChange} />);
        const input = getByRole("spinbutton") as HTMLInputElement;

        fireEvent.input(input, { target: { value: "2.7" } });
        vi.advanceTimersByTime(450);

        expect(onChange).toHaveBeenCalledExactlyOnceWith(2.7);
    });

    it("renders the initial value prop", () => {
        const { getByRole } = render(() => <NumberControl min={0} step={1} value={5} onChange={vi.fn()} />);
        expect((getByRole("spinbutton") as HTMLInputElement).value).toBe("5");
    });

    it("reflects an externally-changed value prop, mirroring SliderControl's own local-signal sync", () => {
        // Wraps NumberControl in a component holding the value in a real
        // Solid signal, so updating it exercises the SAME reactive path a
        // real settings row does (`value={(s()[...] as number) ?? default}`
        // re-rendering when settingsAtom changes) rather than a synthetic
        // prop mutation SolidJS wouldn't otherwise observe.
        const [value, setValue] = createSignal(5);
        const { getByRole } = render(() => <NumberControl min={0} step={1} value={value()} onChange={vi.fn()} />);
        const input = getByRole("spinbutton") as HTMLInputElement;
        expect(input.value).toBe("5");

        setValue(9);
        expect(input.value).toBe("9");
    });
});
