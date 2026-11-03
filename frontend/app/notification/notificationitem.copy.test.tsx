// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Copy button on every toast.

import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { NotificationItem } from "./notificationitem";

const toast: NotificationType = { id: "t", icon: "bell", title: "Couldn't show Agent1", message: "it is no longer open", timestamp: "" };

const renderItem = (onCopy: (id: string) => void | Promise<void>, n: NotificationType = toast, isBubble = true) =>
    render(() => (
        <NotificationItem
            notification={n}
            onRemove={() => {}}
            onCopy={onCopy}
            onActionClick={() => {}}
            formatTimestamp={() => ""}
            isBubble={isBubble}
        />
    ));

afterEach(() => {
    cleanup();
    vi.useRealTimers();
});

describe("the toast's Copy button", () => {
    it("is a real button, reachable by keyboard, named Copy details", () => {
        const { getByRole } = renderItem(async () => {});
        const btn = getByRole("button", { name: "Copy details" }) as HTMLButtonElement;
        expect(btn.tagName).toBe("BUTTON");
        expect(btn.tabIndex).toBe(0);
    });

    it("copies once when pressed (the click does not also reach the toast), then says Copied", async () => {
        const onCopy = vi.fn(async () => {});
        const { getByRole, findByText } = renderItem(onCopy);
        fireEvent.click(getByRole("button", { name: "Copy details" }));
        expect(onCopy).toHaveBeenCalledTimes(1);
        expect(onCopy).toHaveBeenCalledWith("t");
        const status = await findByText("Copied");
        // Said in a live region beside the button, not inside it, where its name would hide it.
        expect(status.getAttribute("role")).toBe("status");
        expect(getByRole("button", { name: "Copy details" }).contains(status)).toBe(false);
    });

    it("keeps the button the same size: the check replaces the icon and no label is added inside it", async () => {
        const { getByRole, findByText } = renderItem(async () => {});
        const btn = getByRole("button", { name: "Copy details" });
        const before = btn.className;
        fireEvent.click(btn);
        await findByText("Copied");
        expect(btn.textContent).toBe("");
        expect(btn.className).toBe(before);
    });

    it("stops saying Copied after a moment", async () => {
        vi.useFakeTimers();
        const { getByRole, queryByText } = renderItem(async () => {});
        fireEvent.click(getByRole("button", { name: "Copy details" }));
        await vi.advanceTimersByTimeAsync(10);
        expect(queryByText("Copied")).not.toBeNull();
        await vi.advanceTimersByTimeAsync(2000);
        expect(queryByText("Copied")).toBeNull();
    });

    it("does not say Copied when the copy failed", async () => {
        const onCopy = vi.fn(async () => {
            throw new Error("denied");
        });
        const { getByRole, queryByText } = renderItem(onCopy);
        fireEvent.click(getByRole("button", { name: "Copy details" }));
        await waitFor(() => expect(onCopy).toHaveBeenCalled());
        await Promise.resolve();
        expect(queryByText("Copied")).toBeNull();
    });

    it("is on the list variant too", () => {
        const { getByRole } = renderItem(async () => {}, toast, false);
        expect(getByRole("button", { name: "Copy details" })).toBeTruthy();
    });

    it("keeps clicking the toast itself as a way to copy", () => {
        const onCopy = vi.fn(async () => {});
        const { container } = renderItem(onCopy);
        fireEvent.click(container.querySelector(".notification-bubble") as HTMLElement);
        expect(onCopy).toHaveBeenCalledTimes(1);
    });
});
