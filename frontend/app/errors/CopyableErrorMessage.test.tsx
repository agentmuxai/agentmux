// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const writeTextMock = vi.fn();
vi.mock("@/util/clipboard", () => ({ writeText: (text: string) => writeTextMock(text) }));

import { CopyableErrorMessage } from "./CopyableErrorMessage";

afterEach(() => cleanup());
beforeEach(() => writeTextMock.mockReset());

describe("CopyableErrorMessage", () => {
    it("shows the message text", () => {
        const { container } = render(() => <CopyableErrorMessage message="Sign-in failed" class="agent-identity-error" />);
        expect(container.querySelector(".agent-identity-error-text")?.textContent).toBe("Sign-in failed");
    });

    it("redacts the message before it reaches the clipboard (moved to the redacted path, §5)", async () => {
        writeTextMock.mockResolvedValue(undefined);
        const { container } = render(() => (
            <CopyableErrorMessage message="Authorization: Bearer ghp_abcdefghijklmnopqrstuvwxyz0123" class="agent-identity-error" />
        ));
        fireEvent.click(container.querySelector(".agent-identity-error-copy-btn")!);
        await waitFor(() => expect(writeTextMock).toHaveBeenCalled());
        expect(writeTextMock).toHaveBeenCalledExactlyOnceWith("Authorization: [redacted]");
    });
});
