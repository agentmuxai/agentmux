// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { MuxBusSignInChoices, offeredProviders } from "./muxbus-sign-in-choices";

function renderChoices(identityProviders: string[], loading = false) {
    const onSignIn = vi.fn();
    const onCancel = vi.fn();
    render(() => (
        <MuxBusSignInChoices
            identityProviders={identityProviders}
            loading={loading}
            label="Sign in"
            cancelLabel="Cancel sign-in"
            onSignIn={onSignIn}
            onCancel={onCancel}
        />
    ));
    const names = screen.getAllByRole("button").map((b) => b.textContent);
    return { onSignIn, onCancel, names };
}

describe("MuxBusSignInChoices", () => {
    afterEach(() => cleanup());

    it("is one plain sign-in when the cloud publishes no providers", () => {
        const { onSignIn, names } = renderChoices([]);
        expect(names).toEqual(["Sign in"]);
        expect(screen.queryByText("What each sign-in shares with AgentMux")).toBeNull();
        fireEvent.click(screen.getByRole("button", { name: "Sign in" }));
        expect(onSignIn).toHaveBeenCalledWith();
    });

    it("offers Google, then email, and says only what Google shares", () => {
        const { onSignIn, names } = renderChoices(["Google"]);
        expect(names).toEqual(["Continue with Google", "Sign in with email"]);
        expect(screen.getByText("What each sign-in shares with AgentMux")).toBeInTheDocument();
        expect(screen.getByText("Google:")).toBeInTheDocument();
        expect(screen.queryByText("Apple:")).toBeNull();
        fireEvent.click(screen.getByRole("button", { name: "Continue with Google" }));
        expect(onSignIn).toHaveBeenLastCalledWith("Google");
        fireEvent.click(screen.getByRole("button", { name: "Sign in with email" }));
        expect(onSignIn).toHaveBeenLastCalledWith();
    });

    it("offers Google and Apple in that order whatever order they are published in", () => {
        const { onSignIn, names } = renderChoices(["SignInWithApple", "Google"]);
        expect(names).toEqual(["Continue with Google", "Continue with Apple", "Sign in with email"]);
        expect(screen.getByText("Apple:")).toBeInTheDocument();
        expect(screen.getByRole("separator").textContent).toBe("or");
        fireEvent.click(screen.getByRole("button", { name: "Continue with Apple" }));
        expect(onSignIn).toHaveBeenLastCalledWith("SignInWithApple");
    });

    it("shows only Cancel while a sign-in is running", () => {
        const { onSignIn, onCancel, names } = renderChoices(["Google", "SignInWithApple"], true);
        expect(names).toEqual(["Cancel sign-in"]);
        fireEvent.click(screen.getByRole("button", { name: "Cancel sign-in" }));
        expect(onCancel).toHaveBeenCalledTimes(1);
        expect(onSignIn).not.toHaveBeenCalled();
    });

    it("ignores providers it has no button for", () => {
        expect(offeredProviders(["Facebook", "Google"]).map((p) => p.name)).toEqual(["Google"]);
    });
});
