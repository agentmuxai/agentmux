// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The agents × skills grid (Memory → Skills). See SkillMatrixModel. A change reaches an agent's
 * files at its next launch.
 */

import { createEffect, For, Show, type JSX } from "solid-js";
import { Button } from "@/app/element/ui";
import type { SkillCatalogModel } from "./skill-model";
import "./skill-matrix.scss";
import { CELL_TEXT, cellAction, cellState, SkillMatrixModel } from "./skill-matrix-model";

export function SkillMatrix(props: { catalog: SkillCatalogModel }): JSX.Element {
    const matrix = new SkillMatrixModel();
    // Asked again when the catalog or the agent list changes (a bind, a reach switch, an import).
    createEffect(() => {
        props.catalog.revisionAtom();
        void matrix.load(props.catalog.agentsAtom());
    });
    const agents = () => props.catalog.agentsAtom().filter((a) => matrix.cellsAtom().has(a.id));
    return (
        <div class="skill-matrix" data-testid="skill-matrix">
            <h3 class="agent-primitive-modal-name">Agents and skills</h3>
            <p class="agent-primitive-modal-global-note">
                What each agent gets. A change reaches the agent's files at its next launch. Opting out cancels a skill
                that reaches every agent or new agents, for that agent only.
            </p>
            <Show when={matrix.errorAtom()}>
                <div class="agent-primitive-modal-error">{matrix.errorAtom()}</div>
            </Show>
            <Show
                when={props.catalog.skillsAtom().length > 0 && agents().length > 0}
                fallback={
                    <div class="agent-primitive-modal-list-empty">
                        {matrix.loadingAtom() ? "Reading what each agent gets…" : "No skills or agents yet"}
                    </div>
                }
            >
                <div class="skill-matrix-scroll">
                    <table class="skill-matrix-table">
                        <thead>
                            <tr>
                                <th scope="col" class="skill-matrix-corner">
                                    Agent
                                </th>
                                <For each={props.catalog.skillsAtom()}>
                                    {(skill) => (
                                        <th scope="col" title={skill.name}>
                                            <span class="skill-matrix-skill-name">{skill.name}</span>
                                        </th>
                                    )}
                                </For>
                            </tr>
                        </thead>
                        <tbody>
                            <For each={agents()}>
                                {(agent) => (
                                    <tr>
                                        <th scope="row" class="skill-matrix-agent" title={agent.name}>
                                            {agent.name}
                                        </th>
                                        <For each={props.catalog.skillsAtom()}>
                                            {(skill) => {
                                                const state = () => cellState(skill, matrix.cellsAtom().get(agent.id)?.get(skill.id));
                                                return (
                                                    <td>
                                                        <Button
                                                            density="compact"
                                                            class="skill-matrix-cell"
                                                            data-state={state()}
                                                            disabled={state() === "bundle"}
                                                            title={`${cellAction(state())}: ${agent.name} and ${skill.name}`}
                                                            aria-label={`${agent.name}, ${skill.name}: ${CELL_TEXT[state()] || "not given"}. ${cellAction(state())}`}
                                                            onClick={() => void matrix.toggle(agent, skill, state(), props.catalog.agentsAtom())}
                                                        >
                                                            {CELL_TEXT[state()] || "—"}
                                                        </Button>
                                                    </td>
                                                );
                                            }}
                                        </For>
                                    </tr>
                                )}
                            </For>
                        </tbody>
                    </table>
                </div>
            </Show>
        </div>
    );
}
