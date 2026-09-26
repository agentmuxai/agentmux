// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const writeTextMock = vi.fn();
vi.mock("@/util/clipboard", () => ({ writeText: (text: string) => writeTextMock(text) }));

import { CopyErrorButton } from "./CopyErrorButton";

afterEach(() => cleanup());
beforeEach(() => {
    writeTextMock.mockReset();
});

describe("CopyErrorButton — success", () => {
    it("writes via IPC and shows Copied, for both variants", async () => {
        writeTextMock.mockResolvedValue(undefined);
        const { container, unmount } = render(() => (
            <CopyErrorButton report={() => "the report text"} variant="action" />
        ));
        fireEvent.click(container.querySelector(".copy-error-button-action")!);
        await waitFor(() => expect(container.textContent).toContain("Copied"));
        expect(writeTextMock).toHaveBeenCalledExactlyOnceWith("the report text");
        unmount();

        writeTextMock.mockClear();
        const icon = render(() => <CopyErrorButton report={() => "icon report"} variant="icon" />);
        fireEvent.click(icon.container.querySelector(".copy-error-button-icon")!);
        await waitFor(() => expect(icon.container.querySelector(".is-copied")).not.toBeNull());
        expect(writeTextMock).toHaveBeenCalledExactlyOnceWith("icon report");
    });

    it("computes the report lazily — only on click, not on render", () => {
        const report = vi.fn(() => "text");
        render(() => <CopyErrorButton report={report} />);
        expect(report).not.toHaveBeenCalled();
    });

    it("redacts the report text itself, even a raw message that never went through formatErrorReport", async () => {
        writeTextMock.mockResolvedValue(undefined);
        const { container } = render(() => (
            <CopyErrorButton report={() => "Authorization: Bearer ghp_abcdefghijklmnopqrstuvwxyz0123"} />
        ));
        fireEvent.click(container.querySelector(".copy-error-button-action")!);
        await waitFor(() => expect(container.textContent).toContain("Copied"));
        expect(writeTextMock).toHaveBeenCalledExactlyOnceWith("Authorization: [redacted]");
    });

    it('transport="dom" skips IPC entirely', async () => {
        const execSpy = vi.fn().mockReturnValue(true);
        document.execCommand = execSpy;
        const { container } = render(() => <CopyErrorButton report={() => "dom text"} transport="dom" />);
        fireEvent.click(container.querySelector(".copy-error-button-action")!);
        await waitFor(() => expect(container.textContent).toContain("Copied"));
        expect(writeTextMock).not.toHaveBeenCalled();
        expect(execSpy).toHaveBeenCalled();
        delete document.execCommand;
    });
});

describe("CopyErrorButton — IPC failure falls back to execCommand", () => {
    it("falls back to execCommand when the IPC write throws, and still shows Copied", async () => {
        writeTextMock.mockRejectedValue(new Error("no bridge"));
        const execSpy = vi.fn().mockReturnValue(true);
        document.execCommand = execSpy;
        const { container } = render(() => <CopyErrorButton report={() => "fallback text"} />);
        fireEvent.click(container.querySelector(".copy-error-button-action")!);
        await waitFor(() => expect(container.textContent).toContain("Copied"));
        expect(writeTextMock).toHaveBeenCalledExactlyOnceWith("fallback text");
        expect(execSpy).toHaveBeenCalled();
        delete document.execCommand;
    });
});

describe("CopyErrorButton — total failure", () => {
    it("leaves the text selected with a Ctrl+C hint when both transports fail", async () => {
        writeTextMock.mockRejectedValue(new Error("no bridge"));
        const execSpy = vi.fn().mockReturnValue(false);
        document.execCommand = execSpy;
        const { container } = render(() => <CopyErrorButton report={() => "never copied"} />);
        fireEvent.click(container.querySelector(".copy-error-button-action")!);
        await waitFor(() => expect(container.textContent).toContain("Copy failed"));

        const fallback = container.querySelector<HTMLTextAreaElement>(".copy-error-button-fallback-text");
        expect(fallback).not.toBeNull();
        expect(fallback!.value).toBe("never copied");
        expect(container.textContent).toContain("Press Ctrl+C");
        delete document.execCommand;
    });

    it("does not auto-reset out of the failed state (unlike a successful copy)", async () => {
        vi.useFakeTimers();
        writeTextMock.mockRejectedValue(new Error("no bridge"));
        const execSpy = vi.fn().mockReturnValue(false);
        document.execCommand = execSpy;
        const { container } = render(() => <CopyErrorButton report={() => "x"} />);
        fireEvent.click(container.querySelector(".copy-error-button-action")!);
        await vi.waitFor(() => expect(container.textContent).toContain("Copy failed"));
        vi.advanceTimersByTime(10_000);
        expect(container.textContent).toContain("Copy failed");
        delete document.execCommand;
        vi.useRealTimers();
    });
});
