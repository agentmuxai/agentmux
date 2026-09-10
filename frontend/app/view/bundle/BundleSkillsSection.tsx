// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * BundleSkillsSection — the "Skills" section of the Bundle editor's detail
 * view. Mirrors BundleMcpSection.tsx — see its doc comment for the
 * bind/unbind-has-no-effect-vs-addPrivate-is-functional reasoning and the
 * flat-list-not-nested-PrimitiveListDetail layout choice.
 */

import { createSignal, For, onCleanup, Show, type JSX } from "solid-js";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import { BundleSkillModel } from "./bundle-skill-model";
import "./BundlePrimitiveSection.scss";

interface BundleSkillsSectionProps {
    bundleId: string;
}

export const BundleSkillsSection = (props: BundleSkillsSectionProps): JSX.Element => {
    const model = new BundleSkillModel(props.bundleId);
    onCleanup(() => model.dispose());

    const [newName, setNewName] = createSignal("");
    const [newContent, setNewContent] = createSignal("");

    const handleAdd = (e: Event) => {
        e.preventDefault();
        const name = newName().trim();
        if (!name) return;
        // See BundleMcpSection.tsx's identical comment — addPrivate never
        // rejects, only clear the form on reported success. reagentx P1 on
        // PR #2647.
        void model.addPrivate(name, newContent()).then((ok) => {
            if (!ok) return;
            setNewName("");
            setNewContent("");
        });
    };

    return (
        <div class="bundle-primitive-section">
            <h4 class="bundle-primitive-section-title">Skills</h4>
            <Show when={model.errorAtom()}>
                <div class="bundle-primitive-section-error">{model.errorAtom()}</div>
            </Show>
            <Show
                when={model.skillsAtom().length > 0}
                fallback={<p class="bundle-primitive-section-empty">No skills yet</p>}
            >
                <ul class="bundle-primitive-section-list">
                    <For each={model.skillsAtom()}>
                        {(skill) => (
                            <li class="bundle-primitive-section-item">
                                <span class="bundle-primitive-section-item-name">{skill.name}</span>
                                <Show when={!skill.is_global}>
                                    <span class="bundle-primitive-section-item-badge is-private">private</span>
                                </Show>
                                <Show
                                    when={skill.bound_to_bundle}
                                    fallback={
                                        <Show when={skill.is_global}>
                                            <button
                                                type="button"
                                                class="bundle-primitive-section-btn"
                                                onClick={() => void model.bind(skill.id)}
                                            >
                                                Bind
                                            </button>
                                        </Show>
                                    }
                                >
                                    <button
                                        type="button"
                                        class="bundle-primitive-section-btn"
                                        onClick={() => void model.unbind(skill.id)}
                                    >
                                        {skill.is_global ? "Unbind" : "Remove"}
                                    </button>
                                </Show>
                            </li>
                        )}
                    </For>
                </ul>
            </Show>
            <p class="bundle-primitive-section-caveat">
                Binding an existing global skill here has no effect on a spawned agent
                — it's already applied to every agent regardless of bundle binding. Add a
                new private skill below to give this bundle its own skill.
            </p>
            <form class="bundle-primitive-section-add" onSubmit={handleAdd}>
                <input
                    type="text"
                    class="bundle-primitive-section-add-input"
                    placeholder="Skill name"
                    value={newName()}
                    onInput={(e) => setNewName(e.currentTarget.value)}
                    onContextMenu={showTextInputContextMenu}
                />
                <textarea
                    class="bundle-primitive-section-add-textarea"
                    placeholder="Skill content / prompt"
                    rows={3}
                    value={newContent()}
                    onInput={(e) => setNewContent(e.currentTarget.value)}
                    onContextMenu={showTextInputContextMenu}
                />
                <button
                    type="submit"
                    class="bundle-primitive-section-add-btn"
                    disabled={model.addingAtom() || !newName().trim()}
                >
                    {model.addingAtom() ? "Adding…" : "+ Add private skill"}
                </button>
            </form>
        </div>
    );
};

BundleSkillsSection.displayName = "BundleSkillsSection";
