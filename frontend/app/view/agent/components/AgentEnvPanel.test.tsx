// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Environment tab: loads the stored `env` content, shows the lines the
 * launch would skip, saves through setagentcontent, and reloads when an agent
 * changes it, unless there are unsaved edits.
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const getContent = vi.fn();
const setContent = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        GetAgentContentCommand: (...args: unknown[]) => getContent(...args),
        SetAgentContentCommand: (...args: unknown[]) => setContent(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
let changed: (() => void) | undefined;
vi.mock("@/app/store/mps", () => ({
    onMuxEvent: (_event: string, cb: () => void) => {
        changed = cb;
        return () => {};
    },
}));

import { AgentEnvPanel, ENV_HINT } from "./AgentEnvPanel";

const stored = (content: string) => ({ agent_id: "uid-x", content_type: "env", content, updated_at: 1 });
const box = () => screen.getByLabelText("Environment") as HTMLTextAreaElement;
const save = () => screen.getByRole("button", { name: "Save" }) as HTMLButtonElement;

describe("AgentEnvPanel", () => {
    beforeEach(() => {
        getContent.mockReset();
        setContent.mockReset();
        changed = undefined;
    });
    afterEach(() => cleanup());

    it("loads the stored text, with the help text", async () => {
        getContent.mockResolvedValue(stored("# tools\nEDITOR=vim\n"));
        render(() => <AgentEnvPanel agentId="uid-x" agentName="AgentX" />);
        await waitFor(() => expect(box().value).toBe("# tools\nEDITOR=vim\n"));
        expect(getContent).toHaveBeenCalledWith({}, { agent_id: "uid-x", content_type: "env" });
        expect(screen.getByText(ENV_HINT)).toBeInTheDocument();
        expect(ENV_HINT).toContain("next launch");
        expect(ENV_HINT).toContain("AGENTMUX_");
        expect(save().disabled).toBe(true);
    });

    it("starts empty for an agent with nothing stored", async () => {
        getContent.mockResolvedValue(null);
        render(() => <AgentEnvPanel agentId="uid-x" agentName="AgentX" />);
        await waitFor(() => expect(box().disabled).toBe(false));
        expect(box().value).toBe("");
    });

    it("saves an edit exactly as typed, through setagentcontent", async () => {
        getContent.mockResolvedValue(stored("EDITOR=vim\n"));
        setContent.mockResolvedValue(stored("x"));
        render(() => <AgentEnvPanel agentId="uid-x" agentName="AgentX" />);
        await waitFor(() => expect(box().value).toBe("EDITOR=vim\n"));
        fireEvent.input(box(), { target: { value: "# note\nEDITOR=nano  \n\nRUST_LOG=debug" } });
        expect(save().disabled).toBe(false);
        fireEvent.click(save());
        await waitFor(() =>
            expect(setContent).toHaveBeenCalledWith(
                {},
                { agent_id: "uid-x", content_type: "env", content: "# note\nEDITOR=nano  \n\nRUST_LOG=debug" }
            )
        );
        expect(await screen.findByText("Saved. It applies at the agent's next launch.")).toBeInTheDocument();
        expect(save().disabled).toBe(true);
    });

    it("shows the lines the launch would skip, and the reserved names", async () => {
        getContent.mockResolvedValue(stored(""));
        render(() => <AgentEnvPanel agentId="uid-x" agentName="AgentX" />);
        await waitFor(() => expect(box().disabled).toBe(false));
        expect(screen.queryByTestId("agent-env-ignored")).toBeNull();
        fireEvent.input(box(), { target: { value: "OK=1\nnot a setting\nKEY = spaced\nAGENTMUX_AGENT_ID=x" } });
        const ignored = await screen.findByTestId("agent-env-ignored");
        expect(ignored).toHaveTextContent("Line 2: not a setting");
        expect(ignored).toHaveTextContent("Line 3: KEY = spaced");
        expect(ignored).not.toHaveTextContent("OK=1");
        expect(screen.getByTestId("agent-env-reserved")).toHaveTextContent("AGENTMUX_AGENT_ID");
        // Ignored lines don't stop a save: the operator may keep them.
        expect(save().disabled).toBe(false);
    });

    it("reverts to the stored text", async () => {
        getContent.mockResolvedValue(stored("A=1"));
        render(() => <AgentEnvPanel agentId="uid-x" agentName="AgentX" />);
        await waitFor(() => expect(box().value).toBe("A=1"));
        fireEvent.input(box(), { target: { value: "A=2" } });
        fireEvent.click(screen.getByRole("button", { name: "Revert" }));
        await waitFor(() => expect(box().value).toBe("A=1"));
    });

    it("shows a refusal from srv and keeps the edit", async () => {
        getContent.mockResolvedValue(stored(""));
        setContent.mockRejectedValue(new Error("setagentcontent: the store is read-only"));
        render(() => <AgentEnvPanel agentId="uid-x" agentName="AgentX" />);
        await waitFor(() => expect(box().disabled).toBe(false));
        fireEvent.input(box(), { target: { value: "A=1" } });
        fireEvent.click(save());
        expect(await screen.findByRole("alert")).toHaveTextContent("the store is read-only");
        expect(box().value).toBe("A=1");
        expect(save().disabled).toBe(false);
    });

    it("shows a load failure", async () => {
        getContent.mockRejectedValue(new Error("getagentcontent: no such agent"));
        render(() => <AgentEnvPanel agentId="uid-x" agentName="AgentX" />);
        expect(await screen.findByRole("alert")).toHaveTextContent("no such agent");
    });

    it("follows a change made elsewhere, unless there are unsaved edits", async () => {
        getContent.mockResolvedValue(stored("A=1"));
        render(() => <AgentEnvPanel agentId="uid-x" agentName="AgentX" />);
        await waitFor(() => expect(box().value).toBe("A=1"));
        getContent.mockResolvedValue(stored("A=1\nB=2"));
        changed!();
        await waitFor(() => expect(box().value).toBe("A=1\nB=2"));
        fireEvent.input(box(), { target: { value: "mine" } });
        getContent.mockResolvedValue(stored("C=3"));
        changed!();
        await waitFor(() => expect(getContent).toHaveBeenCalledTimes(3));
        expect(box().value).toBe("mine");
    });
});
