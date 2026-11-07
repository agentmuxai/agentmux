// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * One script of a computer-use skill, to read and approve (docs/specs/SPEC_PACKS_2026_11_05.md §9). A script is
 * never written to an agent's folder until a person approves it here; approving pins the publisher's key.
 */

// reactive-coverage: one-shot a script's text is read when the person opens its Review panel and read again when its hash changes; nothing else changes it

import { createResource, createSignal, For, Show, type JSX } from "solid-js";
import { Button, Pill } from "@/app/element/ui";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { ScriptInfo } from "@/types/rpc/ScriptInfo";
import type { SkillFileInfo } from "@/types/rpc/SkillFileInfo";
import type { SkillApp } from "@/types/rpc/SkillApp";
import { describeSignature, scriptState } from "./skill-app-model";
import "./skill-app.scss";

/** `open` and `onToggle` belong to the caller: approving refreshes the file list, which rebuilds each row, and the panel must stay as it was. */
export function ScriptReview(p: {
    skillId: string;
    file: SkillFileInfo;
    info: ScriptInfo | undefined;
    open: boolean;
    onToggle: () => void;
}): JSX.Element {
    const open = () => p.open;
    const [busy, setBusy] = createSignal(false);
    const [error, setError] = createSignal("");
    // Keyed on the hash too: a script replaced by an update is read again, never shown stale.
    const [source] = createResource(
        () => (open() ? p.file.sha256 : null),
        async (sha) => (sha ? (await RpcApi.SkillCatalogAppFileCommand(TabRpcClient, { skill_id: p.skillId, path: p.file.path })).text : ""),
    );

    async function setApproved(approved: boolean): Promise<void> {
        setBusy(true);
        setError("");
        try {
            await RpcApi.SkillCatalogAppScriptCommand(TabRpcClient, { skill_id: p.skillId, path: p.file.path, approved, sha256: p.file.sha256 });
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        } finally {
            setBusy(false);
        }
    }

    return (
        <>
            <Pill tone={p.file.approved ? "ok" : "warn"} title={p.file.approved ? undefined : "Never given to an agent until you approve it"}>
                {scriptState(p.file)}
            </Pill>
            <Button variant="link" aria-expanded={open()} onClick={p.onToggle}>
                {open() ? "Hide" : "Review"}
            </Button>
            <Show when={open()}>
                <div class="skill-app-script" role="group" aria-label={`Script ${p.file.path}`}>
                    <Show when={p.info?.purpose}>
                        <p class="skill-app-line">{p.info?.purpose}</p>
                    </Show>
                    <p class="skill-app-line skill-app-muted">
                        Needs: {p.info && p.info.needs.length > 0 ? p.info.needs.join(", ") : "nothing listed"}
                    </p>
                    <p class="skill-app-line skill-app-muted">SHA-256 {p.file.sha256}</p>
                    <pre class="skill-app-source" tabIndex={0}>
                        {source.loading ? "Loading…" : source.error ? "Could not read the script." : source()}
                    </pre>
                    <Show when={error()}>
                        <p class="skill-app-line skill-app-warn" role="alert">
                            {error()}
                        </p>
                    </Show>
                    <Show
                        when={p.file.approved}
                        fallback={
                            <>
                                <p class="skill-app-line skill-app-muted">
                                    Approving lets agents that use this skill receive and run this exact script.
                                </p>
                                <Button tone="accent" busy={busy()} onClick={() => void setApproved(true)}>
                                    Approve script
                                </Button>
                            </>
                        }
                    >
                        <Button tone="danger" busy={busy()} onClick={() => void setApproved(false)}>
                            Revoke approval
                        </Button>
                    </Show>
                </div>
            </Show>
        </>
    );
}

/** What an import preview says about a computer-use skill before it is imported: who signed it and which scripts it carries. */
export function ImportAppFacts(p: { app: SkillApp }): JSX.Element {
    const signature = () => describeSignature(p.app.signature);
    return (
        <div class="skill-import-app">
            <p class={`skill-app-line${signature().tone === "warn" ? " skill-app-warn" : ""}`}>{signature().text}</p>
            <Show when={p.app.scripts.length > 0}>
                <p class="skill-app-line skill-app-muted">
                    Scripts are stored, not run: no agent gets one until you approve it in the skill's detail.
                </p>
                <ul class="skill-app-list" aria-label="Scripts">
                    <For each={p.app.scripts}>
                        {(s) => (
                            <li>
                                {s.path} <span class="skill-app-muted">{s.purpose}{s.needs.length > 0 ? ` · needs ${s.needs.join(", ")}` : ""}</span>
                            </li>
                        )}
                    </For>
                </ul>
            </Show>
        </div>
    );
}
