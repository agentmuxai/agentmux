// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Run test on a computer-use skill's detail (docs/specs/SPEC_PACKS_2026_11_05.md §7, §8). It does not run anything:
 * it files a normal task for the agent the person picks, who reads the skill's `test.json`, drives the app with
 * `Desktop` under the person's own grant, and records the result, so the grant prompt and the audit trail are the
 * usual ones.
 */

import { createSignal, Show, type JSX } from "solid-js";
import { Button, Select } from "@/app/element/ui";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { AgentDefinition } from "@/types/rpc/AgentDefinition";
import type { AppSkillHere } from "@/types/rpc/AppSkillHere";
import type { SkillApp } from "@/types/rpc/SkillApp";
import { appPlatform, hasTestTask, testTask } from "./skill-app-model";
import "./skill-app.scss";

export function RunTest(p: { skillId: string; app: SkillApp; here: AppSkillHere | undefined; agents: AgentDefinition[] }): JSX.Element {
    const platform = appPlatform();
    const [open, setOpen] = createSignal(false);
    const [agent, setAgent] = createSignal("");
    const [busy, setBusy] = createSignal(false);
    const [error, setError] = createSignal("");
    const [filed, setFiled] = createSignal("");
    // Not installed here: nothing to run the test against, and the reason is said instead of a button that fails.
    const missing = () => p.here?.found === false;

    async function file(): Promise<void> {
        setBusy(true);
        setError("");
        setFiled("");
        try {
            const task = testTask(p.app, p.skillId, platform, p.here);
            const res = await RpcApi.KanbanActCommand(TabRpcClient, { action: "create", title: task.title, payload: task.payload, agent: agent(), review: false });
            setFiled(`Task${res.num ? ` #${res.num}` : ""} filed for ${agent()}: it starts when they take it. The result is recorded under Tests.`);
            setOpen(false);
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        } finally {
            setBusy(false);
        }
    }

    return (
        <Show when={hasTestTask(p.app, platform)}>
            <div class="skill-app-run-test">
                <Button density="compact" disabled={missing()} aria-expanded={open()} onClick={() => setOpen(!open())}>
                    Run test
                </Button>
                <Show when={missing()}>
                    <span class="skill-app-muted"> {p.app.name} is not found on this computer: install it first.</span>
                </Show>
                <Show when={open()}>
                    <div class="skill-app-line" role="group" aria-label="Run test">
                        <p class="skill-app-muted">
                            Files a task for the agent you pick. It does not run the test itself: that agent drives {p.app.name} with Desktop under your
                            grant for it, so your prompt and the audit log are the usual ones.
                        </p>
                        <Select
                            density="compact"
                            ariaLabel="Agent to run the test"
                            value={agent()}
                            onChange={setAgent}
                            options={[{ value: "", label: "Choose an agent" }, ...p.agents.map((a) => ({ value: a.name, label: a.name }))]}
                        />{" "}
                        <Button tone="accent" density="compact" busy={busy()} disabled={!agent()} onClick={() => void file()}>
                            File the task
                        </Button>
                    </div>
                </Show>
                <Show when={filed()}>
                    <p class="skill-app-line" role="status">
                        {filed()}
                    </p>
                </Show>
                <Show when={error()}>
                    <p class="skill-app-line skill-app-warn" role="alert">
                        {error()}
                    </p>
                </Show>
            </div>
        </Show>
    );
}
