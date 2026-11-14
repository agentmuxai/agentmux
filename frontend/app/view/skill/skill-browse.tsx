// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Memory → Skills → Browse (docs/specs/SPEC_SKILLS_REPOSITORY_2026_11_05.md §2): the public catalog of
 * computer-use skills, with what is installed here, and Install or Update. Installing offers, never approves:
 * the skill arrives with its scripts unapproved, and an MCP pointer is only shown. The catalog is the one
 * thing here that needs the network; when it is down the view says so and nothing else is affected.
 */

// reactive-coverage: one-shot the remote catalog changes only when its maintainers publish, so it is read when Browse opens, again after an install changes Memory's own list, and on Try again

import { createResource, createSignal, For, Show, type JSX } from "solid-js";
import { Button, Pill } from "@/app/element/ui";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import type { AppSkillCatalogItem } from "@/types/rpc/AppSkillCatalogItem";
import { appHereLine, appPlatform, catalogStatus, forAnotherOs, osNames, pillTone } from "./skill-app-model";
import type { SkillCatalogModel } from "./skill-model";
import "./skill-app.scss";
import "./skill-browse.scss";

/** The button, or the line that stands for it, for what is installed here against the catalog's version. */
function installAction(item: AppSkillCatalogItem): { label: string; note?: string } | null {
    switch (item.state) {
        case "not_installed":
            return { label: "Install" };
        case "update":
            return { label: `Update to ${item.entry.version}`, note: `Installed here: ${item.installed_version}` };
        case "changed":
            return { label: "Replace with the catalog's", note: `The same version ${item.entry.version} is installed here with other files` };
        case "newer_here":
            return null;
        default:
            return null;
    }
}

/** The refusal as the person reads it: without the name of the command that raised it. */
function errorText(e: unknown): string {
    return (e instanceof Error ? e.message : String(e)).replace(/^[a-z_.]+: /, "");
}

function BadgePill(p: { item: AppSkillCatalogItem }): JSX.Element {
    return p.item.badge === "agentmux" ? (
        <Pill tone="accent" icon="shield-halved" title={`Signed by the AgentMux key ${p.item.fingerprint}`}>
            AgentMux
        </Pill>
    ) : (
        <Pill tone="neutral" title="Signed by its author's own key">
            Community · {p.item.fingerprint}
        </Pill>
    );
}

/** What an install told the person, kept by the list under the entry's id so it outlives the row being rebuilt. */
type InstallResult = { skillId: string; text: string };

function BrowseItem(p: {
    item: AppSkillCatalogItem;
    done: InstallResult | undefined;
    onDone: (id: string, r: InstallResult | undefined) => void;
    onInstalled: (skillId: string) => void;
    onChanged: () => void;
}): JSX.Element {
    const [busy, setBusy] = createSignal(false);
    const [error, setError] = createSignal("");
    const platform = appPlatform();
    const status = () => catalogStatus(p.item.entry, platform);
    const otherOs = () => forAnotherOs(p.item.entry, platform);
    const action = () => (otherOs() ? null : installAction(p.item));

    async function install(): Promise<void> {
        setBusy(true);
        setError("");
        p.onDone(p.item.entry.id, undefined);
        try {
            const r = await RpcApi.AppSkillsCatalogInstallCommand(TabRpcClient, { id: p.item.entry.id });
            p.onDone(p.item.entry.id, {
                skillId: r.skill_id,
                text: `${r.updated ? "Updated" : "Installed"} ${r.name} ${r.version}. Its scripts are not approved yet: review them in the skill's detail.`,
            });
            p.onChanged();
        } catch (e) {
            setError(errorText(e));
        } finally {
            setBusy(false);
        }
    }

    return (
        <li class="skill-browse-item" aria-label={p.item.entry.name}>
            <h4 class="skill-app-title">
                {p.item.entry.name} <span class="skill-app-muted">{p.item.entry.app.vendor} · version {p.item.entry.version}</span>
            </h4>
            <p class="skill-app-line">
                <BadgePill item={p.item} /> <Pill tone={pillTone(status().tone)}>{status().label}</Pill>
            </p>
            <p class="skill-app-line">{p.item.entry.description}</p>
            <Show when={appHereLine(p.item, platform)}>{(line) => <p class="skill-app-line skill-app-muted">{line()}</p>}</Show>
            <p class="skill-app-line skill-app-muted">
                Works on {osNames(p.item.entry.platforms) || "no platform listed"} · routes: {[...new Set(p.item.entry.routes)].join(", ")}
            </p>
            <Show when={p.item.entry.hasScripts}>
                <p class="skill-app-line skill-app-warn">Carries scripts: you review each one before an agent can use it.</p>
            </Show>
            <Show when={p.item.entry.mcp}>
                <p class="skill-app-line skill-app-muted">
                    Points at the MCP catalog entry {p.item.entry.mcp}. It is installed from the MCP tab, never from here.
                </p>
            </Show>
            <Show
                when={action()}
                fallback={
                    <p class="skill-app-line skill-app-muted">
                        {otherOs()
                            ? `Needs ${osNames(p.item.entry.platforms)}; this computer runs ${osNames([platform])}, so it is not offered here.`
                            : p.item.state === "current"
                              ? `Installed here at version ${p.item.installed_version}.`
                              : `Installed here: ${p.item.installed_version}, newer than the catalog's ${p.item.entry.version}.`}
                    </p>
                }
            >
                {(a) => (
                    <div class="skill-browse-actions">
                        <Button tone="accent" busy={busy()} onClick={() => void install()}>
                            {a().label}
                        </Button>
                        <Show when={a().note}>
                            <span class="skill-app-muted">{a().note}</span>
                        </Show>
                    </div>
                )}
            </Show>
            <Show when={p.done}>
                {(d) => (
                    <p class="skill-app-line" role="status">
                        {d().text}{" "}
                        <Button variant="link" onClick={() => p.onInstalled(d().skillId)}>
                            Open it
                        </Button>
                    </p>
                )}
            </Show>
            <Show when={error()}>
                <p class="skill-app-line skill-app-warn" role="alert">
                    {error()}
                </p>
            </Show>
        </li>
    );
}

/**
 * The catalog, read again whenever Memory's own list changes (an install refreshes it), so what is shown as
 * installed is never behind.
 */
export function SkillBrowse(p: { catalog: SkillCatalogModel; onOpen: (skillId: string) => void }): JSX.Element {
    const [list, { refetch }] = createResource(
        () => p.catalog.revisionAtom(),
        () => RpcApi.AppSkillsCatalogCommand(TabRpcClient, {}),
    );
    // An install refreshes the list, which rebuilds the rows: what it said is kept here, not in the row.
    const [done, setDone] = createSignal<Record<string, InstallResult | undefined>>({});
    // Reading a resource that failed throws, so the list is read only while there is no error.
    const items = () => (list.error ? undefined : list());
    return (
        <div class="agent-primitive-modal-form skill-browse">
            <h3 class="agent-primitive-modal-name">Browse computer-use skills</h3>
            <p class="skill-app-line skill-app-muted">
                The AgentMux catalog is checked against a key built into AgentMux. Installing adds a skill for you to review: its scripts
                stay unapproved until you approve them.
            </p>
            <Show when={list.error}>
                <div class="agent-primitive-modal-error" role="alert">
                    {errorText(list.error)}
                </div>
                <p class="skill-app-line skill-app-muted">Nothing else depends on the catalog: your installed skills work as they are.</p>
                <div class="skill-browse-actions">
                    <Button density="compact" onClick={() => void refetch()}>
                        Try again
                    </Button>
                </div>
            </Show>
            <Show when={list.loading && !items()}>
                <p class="skill-app-line skill-app-muted">Reading the catalog…</p>
            </Show>
            <Show when={items()}>
                {(l) => (
                    <ul class="skill-browse-list" aria-label="Skills in the catalog">
                        <For each={l().items} fallback={<li class="skill-app-muted">The catalog has no skills yet.</li>}>
                            {(item) => (
                                <BrowseItem
                                    item={item}
                                    done={done()[item.entry.id]}
                                    onDone={(id, r) => setDone((all) => ({ ...all, [id]: r }))}
                                    onInstalled={p.onOpen}
                                    onChanged={() => void p.catalog.refresh()}
                                />
                            )}
                        </For>
                    </ul>
                )}
            </Show>
        </div>
    );
}
