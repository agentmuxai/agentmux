// "Checked 14:05" / "Checked Oct 3" follows the person's locale: nothing here forces one.
import { afterEach, describe, expect, it, vi } from "vitest";
import { checkedAt } from "./app-update";

describe("checkedAt", () => {
    afterEach(() => vi.restoreAllMocks());

    it("leaves the locale to the runtime for a date on another day", () => {
        const spy = vi.spyOn(Date.prototype, "toLocaleDateString");
        checkedAt(new Date(2026, 9, 3, 14, 5).getTime(), new Date(2026, 9, 4));
        expect(spy).toHaveBeenCalledTimes(1);
        expect(spy.mock.calls[0][0], "no locale given, so the browser's applies").toBeUndefined();
    });

    it("shows the clock for a check made today", () => {
        const spy = vi.spyOn(Date.prototype, "toLocaleDateString");
        const now = new Date(2026, 9, 4, 20, 0);
        expect(checkedAt(new Date(2026, 9, 4, 14, 5).getTime(), now)).toMatch(/14:05|2:05/);
        expect(spy).not.toHaveBeenCalled();
    });
});
