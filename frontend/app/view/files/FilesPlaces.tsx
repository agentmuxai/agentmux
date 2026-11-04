// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The Files pane's sidebar: Places, Drives, WSL, Remote hosts and the agents' folders. */

import { Button, IconButton } from "@/app/element/ui";
import { openRemotesInPane } from "@/app/view/remotes/open-remotes";
import { For, Show, type JSX } from "solid-js";
import { readableAgentColor, type FilesModel } from "./files-model";

export function FilesPlaces(props: { model: FilesModel }): JSX.Element {
    const model = props.model;
    return (
        <Show when={model.showSidebar()}>
            <nav class="files-places" aria-label="Places">
                <div class="files-places-heading">Places</div>
                <For each={model.places().filter((p) => p.kind !== "drive" && p.kind !== "wsl")}>
                    {(p) => (
                        <Button variant="row" density="compact" class="files-place" title={p.path} onClick={() => void model.openOn("", p.path)}>
                            <i class={`fa fa-${p.kind === "home" ? "house" : "folder"}`} />
                            <span>{p.label}</span>
                        </Button>
                    )}
                </For>
                <Show when={model.places().some((p) => p.kind === "drive")}>
                    <div class="files-places-heading">Drives</div>
                    <For each={model.places().filter((p) => p.kind === "drive")}>
                        {(p) => (
                            <Button variant="row" density="compact" class="files-place" title={p.path} onClick={() => void model.openOn("", p.path)}>
                                <i class="fa fa-hard-drive" />
                                <span>{p.label}</span>
                            </Button>
                        )}
                    </For>
                </Show>
                <Show when={model.places().some((p) => p.kind === "wsl")}>
                    <div class="files-places-heading">WSL</div>
                    <For each={model.places().filter((p) => p.kind === "wsl")}>
                        {(p) => (
                            <Button variant="row" density="compact" class="files-place" title={p.path} onClick={() => void model.openOn("", p.path)}>
                                <i class="fa-brands fa-linux" />
                                <span>{p.label}</span>
                            </Button>
                        )}
                    </For>
                </Show>
                <Show when={model.remotes().length > 0}>
                    <div class="files-places-heading files-places-heading-with-action">
                        <span>Remote</span>
                        <IconButton
                            icon="gear"
                            label="Manage remotes…"
                            density="compact"
                            class="files-places-heading-action"
                            onClick={() =>
                                void openRemotesInPane(model.blockId, model.connection() || undefined).catch(
                                    (e) => console.log("could not open Remotes", e)
                                )
                            }
                        />
                    </div>
                    <For each={model.remotes()}>
                        {(host) => (
                            <Button
                                variant="row"
                                density="compact"
                                class={model.connection() === host.name ? "files-place files-place-active" : "files-place"}
                                title={`Your home folder on ${host.name}, over SSH`}
                                onClick={() => void model.openOn(host.name, "~")}
                            >
                                <i class="fa fa-server" style={host.color ? { color: host.color } : undefined} />
                                <span>{host.label}</span>
                            </Button>
                        )}
                    </For>
                </Show>
                <Show when={model.agents().length > 0}>
                    <div class="files-places-heading">Agents</div>
                    <For each={model.agents()}>
                        {(a) => (
                            <Button variant="row" density="compact" class="files-place" title={a.path} onClick={() => void model.openOn("", a.path)}>
                                <i class="fa fa-robot" />
                                <span style={a.color ? { color: readableAgentColor(a.color) } : undefined}>{a.name}</span>
                            </Button>
                        )}
                    </For>
                </Show>
            </nav>
        </Show>
    );
}
