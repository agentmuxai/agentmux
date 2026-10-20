// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** Reveal and Copy: the host confirms the person (Windows Hello) and fetches
 *  the value; Copy never brings it into the page; a revealed value hides after
 *  15 seconds, on Hide, when the window loses focus and when the dialog closes
 *  (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §5.3). */

import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { setPlatform } from "@/util/platformutil";
import type { VaultEntryInfo } from "@/types/rpc/VaultEntryInfo";

const vaultReveal = vi.fn();
const vaultCopy = vi.fn();
vi.mock("@/app/store/app-api", () => ({ getApi: () => ({ approvals: { vaultReveal, vaultCopy } }) }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));

import { REVEAL_SHOWN_MS, VaultRevealDialog } from "./vault-reveal";

const card: VaultEntryInfo = {
    id: "card-company",
    kind: "card",
    section: "cards",
    label: "Company card",
    fields: {},
    tags: [],
    parts_set: ["number", "cvc"],
    tail: "1111",
    created_at: 1,
    updated_at: 1,
    last_used_at: 0,
};

function open(onClose = vi.fn()) {
    const r = render(() => <VaultRevealDialog entry={card} onClose={onClose} />);
    return { ...r, onClose, dialog: () => screen.getByRole("dialog") };
}

const part = (name: string) => document.querySelector(`[data-part="${name}"]`) as HTMLElement;

describe("Reveal and Copy", () => {
    beforeEach(() => {
        setPlatform("win32");
        vaultReveal.mockReset();
        vaultCopy.mockReset();
    });
    afterEach(() => {
        cleanup();
        vi.useRealTimers();
    });

    it("lists only the parts that have a value, hidden", async () => {
        open();
        await screen.findByRole("dialog");
        expect(part("number")).not.toBeNull();
        expect(part("cvc")).not.toBeNull();
        expect(part("expiry")).toBeNull();
        expect(part("number").textContent).toContain("••••••••");
    });

    it("shows a revealed value, then hides it after 15 seconds", async () => {
        vaultReveal.mockResolvedValue({ value: "4111111111111111" });
        vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
        open();
        fireEvent.click(within(part("number")).getByRole("button", { name: /Reveal/ }));
        await vi.advanceTimersByTimeAsync(0);
        expect(part("number").textContent).toContain("4111111111111111");
        expect(vaultReveal).toHaveBeenCalledWith({ id: "card-company", part: "number", label: "Company card" });
        await vi.advanceTimersByTimeAsync(REVEAL_SHOWN_MS - 1);
        expect(part("number").textContent).toContain("4111111111111111");
        await vi.advanceTimersByTimeAsync(2);
        expect(part("number").textContent).not.toContain("4111");
    });

    it("hides on Hide and when the window loses focus", async () => {
        vaultReveal.mockResolvedValue({ value: "123" });
        open();
        await screen.findByRole("dialog");
        fireEvent.click(within(part("cvc")).getByRole("button", { name: /Reveal/ }));
        await waitFor(() => expect(part("cvc").textContent).toContain("123"));
        fireEvent.click(within(part("cvc")).getByRole("button", { name: /Hide/ }));
        expect(part("cvc").textContent).not.toContain("123");
        fireEvent.click(within(part("cvc")).getByRole("button", { name: /Reveal/ }));
        await waitFor(() => expect(part("cvc").textContent).toContain("123"));
        window.dispatchEvent(new Event("blur"));
        await waitFor(() => expect(part("cvc").textContent).not.toContain("123"));
    });

    it("still shows the value when Windows Hello took focus from the window while it was open", async () => {
        let answer: (v: unknown) => void = () => {};
        vaultReveal.mockReturnValue(new Promise((r) => (answer = r)));
        open();
        await screen.findByRole("dialog");
        fireEvent.click(within(part("number")).getByRole("button", { name: /Reveal/ }));
        await waitFor(() => expect(vaultReveal).toHaveBeenCalledTimes(1));
        // The Hello prompt opens over the window: the window loses focus.
        window.dispatchEvent(new Event("blur"));
        answer({ value: "4111111111111111" });
        await waitFor(() => expect(part("number").textContent).toContain("4111111111111111"));
        // Once it's shown, losing focus hides it.
        window.dispatchEvent(new Event("blur"));
        await waitFor(() => expect(part("number").textContent).not.toContain("4111"));
    });

    it("shows nothing when the dialog was closed while Windows Hello was still open", async () => {
        let answer: (v: unknown) => void = () => {};
        vaultReveal.mockReturnValue(new Promise((r) => (answer = r)));
        const { unmount } = open();
        await screen.findByRole("dialog");
        fireEvent.click(within(part("number")).getByRole("button", { name: /Reveal/ }));
        await waitFor(() => expect(vaultReveal).toHaveBeenCalledTimes(1));
        unmount();
        answer({ value: "4111111111111111" });
        await new Promise((r) => setTimeout(r, 20));
        expect(document.body.innerHTML).not.toContain("4111111111111111");
    });

    it("copies on the host and says when it clears, without the value reaching the page", async () => {
        vaultCopy.mockResolvedValue({ clears_in_secs: 30 });
        const { container } = open();
        await screen.findByRole("dialog");
        fireEvent.click(within(part("number")).getByRole("button", { name: /Copy/ }));
        await screen.findByText("Copied. It clears from the clipboard in 30 seconds.");
        expect(vaultCopy).toHaveBeenCalledWith({ id: "card-company", part: "number", label: "Company card" });
        expect(vaultReveal).not.toHaveBeenCalled();
        expect(container.ownerDocument.body.innerHTML).not.toContain("4111111111111111");
    });

    it("stays quiet when the person dismisses Windows Hello, and says why otherwise", async () => {
        vaultReveal.mockRejectedValueOnce(new Error("cancelled"));
        open();
        await screen.findByRole("dialog");
        fireEvent.click(within(part("number")).getByRole("button", { name: /Reveal/ }));
        await waitFor(() => expect(vaultReveal).toHaveBeenCalledTimes(1));
        await waitFor(() => expect(within(part("number")).getByRole("button", { name: /Reveal/ })).not.toBeDisabled());
        expect(screen.queryByRole("alert")).toBeNull();
        vaultReveal.mockRejectedValueOnce(new Error("set up Windows Hello (Settings, Accounts, Sign-in options) first"));
        fireEvent.click(within(part("number")).getByRole("button", { name: /Reveal/ }));
        expect((await screen.findByRole("alert")).textContent).toContain("set up Windows Hello");
        expect(part("number").textContent).toContain("••••••••");
    });

    it("hides every value when the dialog closes", async () => {
        vaultReveal.mockResolvedValue({ value: "4111111111111111" });
        const { unmount } = open();
        await screen.findByRole("dialog");
        fireEvent.click(within(part("number")).getByRole("button", { name: /Reveal/ }));
        await waitFor(() => expect(part("number").textContent).toContain("4111"));
        unmount();
        expect(document.body.innerHTML).not.toContain("4111111111111111");
    });

    it("explains instead of asking where Windows Hello isn't built", async () => {
        setPlatform("darwin");
        open();
        await screen.findByRole("dialog");
        expect(within(screen.getByRole("dialog")).getAllByText(/need Windows Hello for now/).length).toBeGreaterThan(0);
        expect(within(part("number")).getByRole("button", { name: /Reveal/ })).toBeDisabled();
        expect(within(part("number")).getByRole("button", { name: /Copy/ })).toBeDisabled();
    });
});
