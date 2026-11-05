// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * BundleSkillsSection — the "Skills" section of the Bundle editor's detail
 * view. Binding an existing global skill gives it to every agent with the
 * bundle; adding a private skill gives the bundle a skill of its own (see
 * BundleSkillModel's doc comment).
 *
 * Deliberately a flat inline list, not a nested PrimitiveListDetail — this
 * section already lives inside the bundle's own detail pane (itself one side
 * of BundleManagerBody's list/detail split), and a second full-height
 * single-pane swap nested inside that would fight for the same space.
 */

import { createSignal, onCleanup, Show, type JSX } from "solid-js";
import { showTextInputContextMenu } from "@/app/store/contextmenu";
import { BundleSkillModel } from "./bundle-skill-model";
import "./BundlePrimitiveSection.scss";
import { Button, Card, CardGrid, TextArea, TextInput } from "@/app/element/ui";
import { BundleCategory } from "./bundle-category";
import type { SkillBundleListItem } from "@/types/rpc/SkillBundleListItem";

interface BundleSkillsSectionProps {
    bundleId: string;
}

const unbindWord = (skill: SkillBundleListItem) => (skill.is_global ? "Unbind" : "Remove");

/** One bound skill as a card: its name, description and kind, with Unbind (or Remove, for a private one). */
function skillCard(model: BundleSkillModel, skill: SkillBundleListItem): JSX.Element {
    return (
        <Card
            class="bundle-skill-card"
            icon={<i class="fa-solid fa-wand-magic-sparkles" />}
            title={skill.name}
            subtitle={skill.description || undefined}
            meta={[<span>{skill.is_global ? "Global skill" : "Private to this bundle"}</span>]}
            selected={model.selectedIdAtom() === skill.id}
            onSelect={() => model.setSelectedId(model.selectedIdAtom() === skill.id ? null : skill.id)}
            actions={
                <Button
                    density="compact"
                    class="bundle-primitive-section-btn"
                    onClick={() => void model.unbind(skill.id)}
                >
                    {unbindWord(skill)}
                </Button>
            }
        />
    );
}

/** What opening a bound skill's card shows under it: its content, and where it is edited. */
function skillDetail(skill: SkillBundleListItem): JSX.Element {
    return (
        <div class="bundle-skill-detail">
            <pre class="bundle-skill-content">{skill.content || "(no content)"}</pre>
            <p class="bundle-primitive-section-empty">
                {skill.is_global
                    ? "A global skill: edit it in Memory > Skills."
                    : "A private skill belongs to this bundle: remove it to delete it."}
            </p>
        </div>
    );
}

/** The skills the bundle holds, as cards; or that it holds none. */
function BoundSkills(props: { model: BundleSkillModel }): JSX.Element {
    const model = props.model;
    const bound = () => model.skillsAtom().filter((s) => s.bound_to_bundle);
    return (
        <Show when={bound().length > 0} fallback={<p class="bundle-primitive-section-empty">No skills yet</p>}>
            <CardGrid
                ariaLabel="Skills in this bundle"
                items={bound()}
                keyOf={(skill) => skill.id}
                renderCard={(skill) => skillCard(model, skill)}
                selectedKey={model.selectedIdAtom()}
                renderDetail={skillDetail}
                onClose={() => model.setSelectedId(null)}
            />
        </Show>
    );
}

/** Global skills nothing binds here yet, as chips: one click binds one. */
function BindableSkills(props: { model: BundleSkillModel }): JSX.Element {
    const model = props.model;
    const available = () => model.skillsAtom().filter((s) => s.is_global && !s.bound_to_bundle);
    return (
        <Show when={available().length > 0}>
            <p class="bundle-primitive-section-hint">Bind a global skill:</p>
            <CardGrid
                size="chip"
                ariaLabel="Global skills to bind"
                items={available()}
                keyOf={(skill) => skill.id}
                renderCard={(skill) => (
                    <Card
                        size="chip"
                        class="bundle-skill-chip"
                        icon={<i class="fa-solid fa-plus" />}
                        title={skill.name}
                        subtitle="Bind"
                        aria-label={`Bind ${skill.name}`}
                        onSelect={() => void model.bind(skill.id)}
                    />
                )}
            />
        </Show>
    );
}

/** The form that gives the bundle a skill of its own. */
function AddPrivateSkill(props: { model: BundleSkillModel }): JSX.Element {
    const model = props.model;
    const [newName, setNewName] = createSignal("");
    const [newContent, setNewContent] = createSignal("");

    const handleAdd = (e: Event) => {
        e.preventDefault();
        const name = newName().trim();
        if (!name) return;
        // addPrivate never rejects (errors go to errorAtom), so only clear the
        // form on the reported success; otherwise a failed add would silently
        // wipe what the user typed (PR #2647).
        void model.addPrivate(name, newContent()).then((ok) => {
            if (!ok) return;
            setNewName("");
            setNewContent("");
        });
    };

    return (
        <>
            <p class="bundle-primitive-section-caveat">
                Binding a global skill here gives it to every agent with this bundle. A
                global skill nothing is bound to reaches no agent. Add a new private skill
                below to give this bundle its own skill.
            </p>
            <form class="bundle-primitive-section-add" onSubmit={handleAdd}>
                <TextInput variant="plain"
                    class="bundle-primitive-section-add-input"
                    placeholder="Skill name"
                    value={newName()}
                    onInput={(e) => setNewName(e.currentTarget.value)}
                    onContextMenu={showTextInputContextMenu}
                />
                <TextArea variant="plain"
                    class="bundle-primitive-section-add-textarea"
                    placeholder="Skill content / prompt"
                    rows={3}
                    value={newContent()}
                    onInput={(e) => setNewContent(e.currentTarget.value)}
                    onContextMenu={showTextInputContextMenu}
                />
                <Button
                    type="submit"
                    tone="accent"
                    class="bundle-primitive-section-add-btn"
                    disabled={model.addingAtom() || !newName().trim()}
                >
                    {model.addingAtom() ? "Adding…" : "+ Add private skill"}
                </Button>
            </form>
        </>
    );
}

export const BundleSkillsSection = (props: BundleSkillsSectionProps): JSX.Element => {
    const model = new BundleSkillModel(props.bundleId);
    onCleanup(() => model.dispose());

    return (
        <BundleCategory title="Skills">
            <div class="bundle-primitive-section">
                <Show when={model.errorAtom()}>
                    <div class="bundle-primitive-section-error">{model.errorAtom()}</div>
                </Show>
                <BoundSkills model={model} />
                <BindableSkills model={model} />
                <AddPrivateSkill model={model} />
            </div>
        </BundleCategory>
    );
};

BundleSkillsSection.displayName = "BundleSkillsSection";
