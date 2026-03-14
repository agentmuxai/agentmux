// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { createEffect, createSignal, For, Show } from "solid-js";
import type { JSX } from "solid-js";
import * as util from "@/util/util";
import type { ForgeViewModel, ContentTabId, DetailSection } from "./forge-model";
import { CONTENT_TABS, CONTENT_TAB_LABELS, SKILL_TYPES } from "./forge-model";
import "./forge-view.scss";

const PROVIDERS = [
    { id: "claude", label: "Claude Code", cmd: "claude --output-format stream-json" },
    { id: "codex", label: "Codex CLI", cmd: "codex --full-auto" },
    { id: "gemini", label: "Gemini CLI", cmd: "gemini --yolo" },
];

// ── Main view ────────────────────────────────────────────────────────────────

export function ForgeView(props: ViewComponentProps<ForgeViewModel>): JSX.Element {
    const view = props.model.viewAtom;

    return (
        <Show when={view() === "create" || view() === "edit"} fallback={
            <Show when={view() === "detail"} fallback={
                <ForgeList model={props.model} />
            }>
                <ForgeDetail model={props.model} />
            </Show>
        }>
            <ForgeForm model={props.model} />
        </Show>
    );
}

// ── Agent List ────────────────────────────────────────────────────────────────

function ForgeList(props: { model: ForgeViewModel }): JSX.Element {
    const agents = props.model.agentsAtom;
    const [showImport, setShowImport] = createSignal(false);

    const hostAgents = () => agents().filter((a) => a.agent_type === "host");
    const containerAgents = () => agents().filter((a) => a.agent_type === "container");
    const customAgents = () => agents().filter((a) => a.agent_type !== "host" && a.agent_type !== "container");

    return (
        <div class="forge-pane">
            <div class="forge-header">
                <span class="forge-title">Forge</span>
            </div>
            <div class="forge-divider" />
            <Show when={agents().length > 0} fallback={
                <div class="forge-empty">
                    <span class="forge-empty-icon">&#10022;</span>
                    <span class="forge-empty-label">No agents yet</span>
                    <span class="forge-empty-sub">Create your first agent</span>
                    <button class="forge-new-btn" onClick={() => props.model.startCreate()}>
                        + New Agent
                    </button>
                    <button class="forge-new-btn forge-import-btn" onClick={() => setShowImport(true)}>
                        Import from Claw
                    </button>
                </div>
            }>
                <div class="forge-list">
                    <Show when={hostAgents().length > 0}>
                        <div class="forge-group-header">Host Agents</div>
                        <For each={hostAgents()}>{(agent) =>
                            <ForgeAgentCard agent={agent} model={props.model} />
                        }</For>
                    </Show>
                    <Show when={containerAgents().length > 0}>
                        <div class="forge-group-header">Container Agents</div>
                        <For each={containerAgents()}>{(agent) =>
                            <ForgeAgentCard agent={agent} model={props.model} />
                        }</For>
                    </Show>
                    <Show when={customAgents().length > 0}>
                        <div class="forge-group-header">Custom Agents</div>
                        <For each={customAgents()}>{(agent) =>
                            <ForgeAgentCard agent={agent} model={props.model} />
                        }</For>
                    </Show>
                </div>
                <div class="forge-list-footer">
                    <button class="forge-new-btn" onClick={() => props.model.startCreate()}>
                        + New Agent
                    </button>
                    <button class="forge-new-btn forge-import-btn" onClick={() => setShowImport(true)}>
                        Import from Claw
                    </button>
                    <button class="forge-new-btn forge-reseed-btn" onClick={() => props.model.reseedAgents()}>
                        Reset Built-in Agents
                    </button>
                </div>
            </Show>
            <Show when={showImport()}>
                <ForgeImportForm model={props.model} onClose={() => setShowImport(false)} />
            </Show>
        </div>
    );
}

// ── Agent Card ────────────────────────────────────────────────────────────────

function ForgeAgentCard(props: { agent: ForgeAgent; model: ForgeViewModel }): JSX.Element {
    const [confirming, setConfirming] = createSignal(false);

    const handleDelete = async () => {
        if (!confirming()) {
            setConfirming(true);
            return;
        }
        setConfirming(false);
        await props.model.deleteAgent(props.agent.id);
    };

    const handleClick = () => {
        props.model.openDetail(props.agent);
    };

    const handleEdit = (e: MouseEvent) => {
        e.stopPropagation();
        props.model.startEdit(props.agent);
    };

    const handleDeleteClick = (e: MouseEvent) => {
        e.stopPropagation();
        handleDelete();
    };

    const providerLabel = () => PROVIDERS.find((p) => p.id === props.agent.provider)?.label ?? props.agent.provider;
    const typeBadge = () => {
        if (props.agent.agent_type === "host") return "HOST";
        if (props.agent.agent_type === "container") return "CTR";
        return null;
    };

    return (
        <div class="forge-card" onClick={handleClick}>
            <span class="forge-card-icon">{props.agent.icon}</span>
            <div class="forge-card-info">
                <div class="forge-card-name-row">
                    <span class="forge-card-name">{props.agent.name}</span>
                    <Show when={typeBadge()}>
                        <span class={`forge-agent-type-badge forge-agent-type-${props.agent.agent_type}`}>{typeBadge()}</span>
                    </Show>
                </div>
                <span class="forge-card-provider">{providerLabel()}</span>
                <Show when={props.agent.description}>
                    <span class="forge-card-desc">{props.agent.description}</span>
                </Show>
            </div>
            <div class="forge-card-actions">
                <button class="forge-card-btn" onClick={handleEdit} title="Edit">
                    Edit
                </button>
                <button
                    class={`forge-card-btn forge-card-btn-delete${confirming() ? " confirming" : ""}`}
                    onClick={handleDeleteClick}
                    onBlur={() => setConfirming(false)}
                    title={confirming() ? "Click again to confirm" : "Delete"}
                >
                    {confirming() ? "Sure?" : "\u2715"}
                </button>
            </div>
        </div>
    );
}

// ── Detail View ───────────────────────────────────────────────────────────────

const DETAIL_SECTIONS: { id: DetailSection; label: string }[] = [
    { id: "content", label: "Content" },
    { id: "skills", label: "Skills" },
    { id: "history", label: "History" },
];

function ForgeDetail(props: { model: ForgeViewModel }): JSX.Element {
    const agent = props.model.detailAgentAtom;
    const activeSection = props.model.activeSectionAtom;

    return (
        <Show when={agent()}>
            {(agentVal) => {
                const providerLabel = () => PROVIDERS.find((p) => p.id === agentVal().provider)?.label ?? agentVal().provider;
                const detailTypeBadge = () => {
                    if (agentVal().agent_type === "host") return "HOST";
                    if (agentVal().agent_type === "container") return "CTR";
                    return null;
                };
                return (
                    <div class="forge-pane">
                        <div class="forge-detail-header">
                            <button class="forge-back-btn" onClick={() => props.model.closeDetail()}>
                                &larr; Back
                            </button>
                            <span class="forge-detail-icon">{agentVal().icon}</span>
                            <div class="forge-detail-info">
                                <div class="forge-detail-name-row">
                                    <span class="forge-detail-name">{agentVal().name}</span>
                                    <Show when={detailTypeBadge()}>
                                        <span class={`forge-agent-type-badge forge-agent-type-${agentVal().agent_type}`}>{detailTypeBadge()}</span>
                                    </Show>
                                </div>
                                <span class="forge-detail-sub">
                                    {providerLabel()}
                                    {agentVal().description ? ` \u2022 ${agentVal().description}` : ""}
                                </span>
                            </div>
                            <button class="forge-card-btn" onClick={() => props.model.startEditFromDetail()}>
                                Edit
                            </button>
                        </div>
                        <div class="forge-divider" />
                        <div class="forge-section-tabs">
                            <For each={DETAIL_SECTIONS}>{(s) =>
                                <DetailSectionButton section={s} activeSection={activeSection()} model={props.model} agentId={agentVal().id} />
                            }</For>
                        </div>
                        <div class="forge-section-body">
                            <Show when={activeSection() === "content"}>
                                <ForgeContentSection model={props.model} agentId={agentVal().id} />
                            </Show>
                            <Show when={activeSection() === "skills"}>
                                <ForgeSkillsPanel model={props.model} agentId={agentVal().id} />
                            </Show>
                            <Show when={activeSection() === "history"}>
                                <ForgeHistoryPanel model={props.model} agentId={agentVal().id} />
                            </Show>
                        </div>
                    </div>
                );
            }}
        </Show>
    );
}

// ── Detail Section Button ────────────────────────────────────────────────────

function DetailSectionButton(props: {
    section: { id: DetailSection; label: string };
    activeSection: DetailSection;
    model: ForgeViewModel;
    agentId: string;
}): JSX.Element {
    const handleClick = async () => {
        props.model.setActiveSection(props.section.id);
        if (props.section.id === "skills") {
            await props.model.loadSkills(props.agentId);
        } else if (props.section.id === "history") {
            await props.model.loadHistory(props.agentId);
        }
    };

    return (
        <button
            class={`forge-section-tab${props.activeSection === props.section.id ? " active" : ""}`}
            onClick={handleClick}
        >
            {props.section.label}
        </button>
    );
}

// ── Content Section (existing tabs) ──────────────────────────────────────────

function ForgeContentSection(props: { model: ForgeViewModel; agentId: string }): JSX.Element {
    const contentMap = props.model.contentAtom;
    const activeTab = props.model.activeTabAtom;
    const contentLoading = props.model.contentLoadingAtom;

    return (
        <>
            <div class="forge-content-tabs">
                <For each={CONTENT_TABS}>{(tab) =>
                    <ContentTabButton tab={tab} activeTab={activeTab()} model={props.model} />
                }</For>
            </div>
            <div class="forge-content-body">
                <Show when={!contentLoading()} fallback={
                    <div class="forge-content-loading">Loading...</div>
                }>
                    <ContentEditor
                        agentId={props.agentId}
                        contentType={activeTab()}
                        content={contentMap()[activeTab()]}
                        model={props.model}
                    />
                </Show>
            </div>
        </>
    );
}

// ── Content Tab Button ────────────────────────────────────────────────────────

function ContentTabButton(props: {
    tab: ContentTabId;
    activeTab: ContentTabId;
    model: ForgeViewModel;
}): JSX.Element {
    const handleClick = async () => {
        props.model.setActiveTab(props.tab);
    };

    return (
        <button
            class={`forge-content-tab${props.activeTab === props.tab ? " active" : ""}`}
            onClick={handleClick}
        >
            {CONTENT_TAB_LABELS[props.tab]}
        </button>
    );
}

// ── Content Editor ────────────────────────────────────────────────────────────

function ContentEditor(props: {
    agentId: string;
    contentType: string;
    content: ForgeContent | undefined;
    model: ForgeViewModel;
}): JSX.Element {
    const saving = props.model.contentSavingAtom;
    const [editing, setEditing] = createSignal(false);
    const [draft, setDraft] = createSignal("");

    const currentContent = () => props.content?.content ?? "";
    const charCount = () => currentContent().length;

    const handleStartEdit = () => {
        setDraft(currentContent());
        setEditing(true);
    };

    const handleCancel = () => {
        setEditing(false);
        setDraft("");
    };

    const handleSave = async () => {
        await props.model.saveContent(props.agentId, props.contentType, draft());
        setEditing(false);
    };

    return (
        <Show when={editing()} fallback={
            <div class="forge-content-display">
                <Show when={currentContent()} fallback={
                    <div class="forge-content-empty">No content yet</div>
                }>
                    <pre class="forge-content-pre">{currentContent()}</pre>
                </Show>
                <div class="forge-content-display-footer">
                    <span class="forge-content-charcount">
                        {charCount() > 0 ? `${charCount()} chars` : ""}
                        {props.content?.updated_at ? ` \u2022 saved` : ""}
                    </span>
                    <button class="forge-btn-primary" onClick={handleStartEdit}>
                        Edit Content
                    </button>
                </div>
            </div>
        }>
            <div class="forge-content-editor">
                <textarea
                    class="forge-content-textarea"
                    value={draft()}
                    onInput={(e) => setDraft(e.currentTarget.value)}
                    autofocus
                    spellcheck={false}
                />
                <div class="forge-content-editor-footer">
                    <span class="forge-content-charcount">{draft().length} chars</span>
                    <div class="forge-content-editor-actions">
                        <button class="forge-btn-primary" onClick={handleSave} disabled={saving()}>
                            {saving() ? "Saving..." : "Save"}
                        </button>
                        <button class="forge-btn-secondary" onClick={handleCancel} disabled={saving()}>
                            Cancel
                        </button>
                    </div>
                </div>
            </div>
        </Show>
    );
}

// ── Skills Panel ──────────────────────────────────────────────────────────────

function ForgeSkillsPanel(props: { model: ForgeViewModel; agentId: string }): JSX.Element {
    const skills = props.model.skillsAtom;
    const loading = props.model.skillsLoadingAtom;
    const editingSkill = props.model.editingSkillAtom;
    const [showForm, setShowForm] = createSignal(false);

    const handleNewSkill = () => {
        props.model.setEditingSkill(null);
        setShowForm(true);
    };

    const handleEditSkill = (skill: ForgeSkill) => {
        props.model.setEditingSkill(skill);
        setShowForm(true);
    };

    const handleCloseForm = () => {
        props.model.setEditingSkill(null);
        setShowForm(false);
    };

    return (
        <Show when={!loading()} fallback={
            <div class="forge-content-loading">Loading skills...</div>
        }>
            <Show when={!showForm()} fallback={
                <ForgeSkillForm
                    model={props.model}
                    agentId={props.agentId}
                    skill={editingSkill()}
                    onClose={handleCloseForm}
                />
            }>
                <div class="forge-skills-panel">
                    <Show when={skills().length > 0} fallback={
                        <div class="forge-content-empty">No skills yet</div>
                    }>
                        <div class="forge-skills-list">
                            <For each={skills()}>{(skill) =>
                                <ForgeSkillCard
                                    skill={skill}
                                    model={props.model}
                                    onEdit={handleEditSkill}
                                />
                            }</For>
                        </div>
                    </Show>
                    <div class="forge-skills-footer">
                        <button class="forge-btn-primary" onClick={handleNewSkill}>
                            + Add Skill
                        </button>
                    </div>
                </div>
            </Show>
        </Show>
    );
}

// ── Skill Card ────────────────────────────────────────────────────────────────

function ForgeSkillCard(props: {
    skill: ForgeSkill;
    model: ForgeViewModel;
    onEdit: (skill: ForgeSkill) => void;
}): JSX.Element {
    const [confirming, setConfirming] = createSignal(false);

    const handleDelete = async () => {
        if (!confirming()) {
            setConfirming(true);
            return;
        }
        setConfirming(false);
        await props.model.deleteSkill(props.skill.id);
    };

    return (
        <div class="forge-skill-card">
            <div class="forge-skill-card-info">
                <div class="forge-skill-card-top">
                    <span class="forge-skill-card-name">{props.skill.name}</span>
                    <span class="forge-skill-type-badge">{props.skill.skill_type}</span>
                </div>
                <Show when={props.skill.trigger}>
                    <span class="forge-skill-card-trigger">/{props.skill.trigger}</span>
                </Show>
                <Show when={props.skill.description}>
                    <span class="forge-skill-card-desc">{props.skill.description}</span>
                </Show>
            </div>
            <div class="forge-card-actions">
                <button class="forge-card-btn" onClick={() => props.onEdit(props.skill)} title="Edit">
                    Edit
                </button>
                <button
                    class={`forge-card-btn forge-card-btn-delete${confirming() ? " confirming" : ""}`}
                    onClick={handleDelete}
                    onBlur={() => setConfirming(false)}
                    title={confirming() ? "Click again to confirm" : "Delete"}
                >
                    {confirming() ? "Sure?" : "\u2715"}
                </button>
            </div>
        </div>
    );
}

// ── Skill Form ────────────────────────────────────────────────────────────────

function ForgeSkillForm(props: {
    model: ForgeViewModel;
    agentId: string;
    skill: ForgeSkill | null;
    onClose: () => void;
}): JSX.Element {
    const error = props.model.errorAtom;
    const isEdit = () => props.skill != null;

    const [name, setName] = createSignal(props.skill?.name ?? "");
    const [trigger, setTrigger] = createSignal(props.skill?.trigger ?? "");
    const [skillType, setSkillType] = createSignal(props.skill?.skill_type ?? "prompt");
    const [description, setDescription] = createSignal(props.skill?.description ?? "");
    const [content, setContent] = createSignal(props.skill?.content ?? "");

    const handleSubmit = async (e: Event) => {
        e.preventDefault();
        if (!name().trim()) return;
        if (isEdit()) {
            await props.model.updateSkill({
                id: props.skill!.id,
                name: name().trim(),
                trigger: trigger(),
                skill_type: skillType(),
                description: description(),
                content: content(),
            });
        } else {
            await props.model.createSkill({
                agent_id: props.agentId,
                name: name().trim(),
                trigger: trigger(),
                skill_type: skillType(),
                description: description(),
                content: content(),
            });
        }
        props.onClose();
    };

    return (
        <div class="forge-skill-form">
            <div class="forge-skill-form-header">
                <span class="forge-title-sub">{isEdit() ? "Edit Skill" : "New Skill"}</span>
            </div>
            <form onSubmit={handleSubmit}>
                <div class="forge-form-row">
                    <label class="forge-form-label">Name</label>
                    <input
                        class="forge-form-input"
                        value={name()}
                        onInput={(e) => setName(e.currentTarget.value)}
                        placeholder="Skill name"
                        autofocus
                        required
                    />
                </div>
                <div class="forge-form-row">
                    <label class="forge-form-label">Trigger</label>
                    <input
                        class="forge-form-input"
                        value={trigger()}
                        onInput={(e) => setTrigger(e.currentTarget.value)}
                        placeholder="/command-name"
                    />
                </div>
                <div class="forge-form-row">
                    <label class="forge-form-label">Type</label>
                    <select
                        class="forge-form-input forge-form-select"
                        value={skillType()}
                        onInput={(e) => setSkillType(e.currentTarget.value)}
                    >
                        <For each={SKILL_TYPES}>{(t) =>
                            <option value={t}>{t}</option>
                        }</For>
                    </select>
                </div>
                <div class="forge-form-row">
                    <label class="forge-form-label">Description</label>
                    <input
                        class="forge-form-input"
                        value={description()}
                        onInput={(e) => setDescription(e.currentTarget.value)}
                        placeholder="Brief description"
                    />
                </div>
                <div class="forge-form-row forge-form-row-col">
                    <label class="forge-form-label">Content</label>
                    <textarea
                        class="forge-content-textarea forge-skill-content"
                        value={content()}
                        onInput={(e) => setContent(e.currentTarget.value)}
                        placeholder="Skill content (prompt, command, etc.)"
                        spellcheck={false}
                    />
                </div>
                <Show when={error()}>
                    <div class="forge-form-error">{error()}</div>
                </Show>
                <div class="forge-form-actions">
                    <button type="submit" class="forge-btn-primary" disabled={!name().trim()}>
                        {isEdit() ? "Update" : "Create"}
                    </button>
                    <button type="button" class="forge-btn-secondary" onClick={props.onClose}>
                        Cancel
                    </button>
                </div>
            </form>
        </div>
    );
}

// ── History Panel ─────────────────────────────────────────────────────────────

function ForgeHistoryPanel(props: { model: ForgeViewModel; agentId: string }): JSX.Element {
    const entries = props.model.historyAtom;
    const loading = props.model.historyLoadingAtom;
    const [searchQuery, setSearchQuery] = createSignal("");

    const handleSearch = async () => {
        if (searchQuery().trim()) {
            await props.model.searchHistory(props.agentId, searchQuery().trim());
        } else {
            await props.model.loadHistory(props.agentId);
        }
    };

    const handleKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Enter") {
            handleSearch();
        }
    };

    // Group entries by session_date
    const groupedEntries = () => {
        return entries().reduce<Record<string, ForgeHistory[]>>((acc, entry) => {
            const date = entry.session_date;
            if (!acc[date]) acc[date] = [];
            acc[date].push(entry);
            return acc;
        }, {});
    };
    const sortedDates = () => Object.keys(groupedEntries()).sort().reverse();

    return (
        <div class="forge-history-panel">
            <div class="forge-history-search">
                <input
                    class="forge-form-input"
                    value={searchQuery()}
                    onInput={(e) => setSearchQuery(e.currentTarget.value)}
                    onKeyDown={handleKeyDown}
                    placeholder="Search history..."
                />
                <button class="forge-card-btn" onClick={handleSearch}>
                    Search
                </button>
            </div>
            <div class="forge-history-list">
                <Show when={!loading()} fallback={
                    <div class="forge-content-loading">Loading history...</div>
                }>
                    <Show when={entries().length > 0} fallback={
                        <div class="forge-content-empty">No history entries</div>
                    }>
                        <For each={sortedDates()}>{(date) =>
                            <div class="forge-history-group">
                                <div class="forge-history-date">{date}</div>
                                <For each={groupedEntries()[date]}>{(entry) =>
                                    <div class="forge-history-entry">
                                        <span class="forge-history-time">
                                            {new Date(entry.timestamp).toLocaleTimeString()}
                                        </span>
                                        <span class="forge-history-text">{entry.entry}</span>
                                    </div>
                                }</For>
                            </div>
                        }</For>
                    </Show>
                </Show>
            </div>
        </div>
    );
}

// ── Import Form ───────────────────────────────────────────────────────────────

function ForgeImportForm(props: { model: ForgeViewModel; onClose: () => void }): JSX.Element {
    const importing = props.model.importingAtom;
    const error = props.model.errorAtom;
    const [workspacePath, setWorkspacePath] = createSignal("");
    const [agentName, setAgentName] = createSignal("");

    // Auto-fill agent name from path
    createEffect(() => {
        const wp = workspacePath();
        if (wp && !agentName()) {
            const parts = wp.replace(/\\/g, "/").split("/").filter(Boolean);
            if (parts.length > 0) {
                setAgentName(parts[parts.length - 1]);
            }
        }
    });

    const handleSubmit = async (e: Event) => {
        e.preventDefault();
        if (!workspacePath().trim() || !agentName().trim()) return;
        await props.model.importFromClaw(workspacePath().trim(), agentName().trim());
        props.onClose();
    };

    return (
        <div class="forge-import-overlay">
            <div class="forge-import-dialog">
                <div class="forge-skill-form-header">
                    <span class="forge-title-sub">Import from Claw</span>
                </div>
                <form onSubmit={handleSubmit}>
                    <div class="forge-form-row">
                        <label class="forge-form-label">Workspace Path</label>
                        <input
                            class="forge-form-input"
                            value={workspacePath()}
                            onInput={(e) => setWorkspacePath(e.currentTarget.value)}
                            placeholder="~/.claw/workspaces/agent1"
                            autofocus
                            required
                        />
                    </div>
                    <div class="forge-form-row">
                        <label class="forge-form-label">Agent Name</label>
                        <input
                            class="forge-form-input"
                            value={agentName()}
                            onInput={(e) => setAgentName(e.currentTarget.value)}
                            placeholder="Agent name"
                            required
                        />
                    </div>
                    <Show when={error()}>
                        <div class="forge-form-error">{error()}</div>
                    </Show>
                    <div class="forge-form-actions">
                        <button type="submit" class="forge-btn-primary" disabled={importing() || !workspacePath().trim() || !agentName().trim()}>
                            {importing() ? "Importing..." : "Import"}
                        </button>
                        <button type="button" class="forge-btn-secondary" onClick={props.onClose} disabled={importing()}>
                            Cancel
                        </button>
                    </div>
                </form>
            </div>
        </div>
    );
}

// ── Create / Edit Form ────────────────────────────────────────────────────────

function ForgeForm(props: { model: ForgeViewModel }): JSX.Element {
    const view = props.model.viewAtom;
    const editingAgent = props.model.editingAgentAtom;
    const loading = props.model.loadingAtom;
    const error = props.model.errorAtom;

    const isEdit = () => view() === "edit" && editingAgent() != null;

    const [name, setName] = createSignal(editingAgent()?.name ?? "");
    const [icon, setIcon] = createSignal(editingAgent()?.icon ?? "\u2726");
    const [provider, setProvider] = createSignal(editingAgent()?.provider ?? "claude");
    const [description, setDescription] = createSignal(editingAgent()?.description ?? "");
    const [workingDirectory, setWorkingDirectory] = createSignal(editingAgent()?.working_directory ?? "");
    const [shell, setShell] = createSignal(editingAgent()?.shell ?? "");
    const [providerFlags, setProviderFlags] = createSignal(editingAgent()?.provider_flags ?? "");
    const [autoStart, setAutoStart] = createSignal(editingAgent()?.auto_start === 1);

    const handleSubmit = async (e: Event) => {
        e.preventDefault();
        if (!name().trim()) return;
        if (isEdit()) {
            await props.model.updateAgent({
                id: editingAgent()!.id,
                name: name().trim(),
                icon: icon() || "\u2726",
                provider: provider(),
                description: description(),
                working_directory: workingDirectory(),
                shell: shell(),
                provider_flags: providerFlags(),
                auto_start: autoStart() ? 1 : 0,
            });
        } else {
            await props.model.createAgent({
                name: name().trim(),
                icon: icon() || "\u2726",
                provider: provider(),
                description: description(),
                working_directory: workingDirectory(),
                shell: shell(),
                provider_flags: providerFlags(),
                auto_start: autoStart() ? 1 : 0,
            });
        }
    };

    const title = () => isEdit() ? "Edit Agent" : "New Agent";

    return (
        <div class="forge-pane">
            <div class="forge-header">
                <span class="forge-title">
                    Forge&nbsp;/&nbsp;<span class="forge-title-sub">{title()}</span>
                </span>
            </div>
            <div class="forge-divider" />
            <form class="forge-form" onSubmit={handleSubmit}>
                <div class="forge-form-row">
                    <label class="forge-form-label">Icon</label>
                    <input
                        class="forge-form-input forge-form-input-icon"
                        value={icon()}
                        maxLength={4}
                        onInput={(e) => setIcon(e.currentTarget.value)}
                        placeholder="\u2726"
                    />
                </div>

                <div class="forge-form-row">
                    <label class="forge-form-label">Name</label>
                    <input
                        class="forge-form-input"
                        value={name()}
                        onInput={(e) => setName(e.currentTarget.value)}
                        placeholder="My Agent"
                        autofocus
                        required
                    />
                </div>

                <div class="forge-form-row forge-form-row-col">
                    <label class="forge-form-label">Provider</label>
                    <div class="forge-form-providers">
                        <For each={PROVIDERS}>{(p) =>
                            <label class="forge-form-provider-opt">
                                <input
                                    type="radio"
                                    name="provider"
                                    value={p.id}
                                    checked={provider() === p.id}
                                    onInput={() => setProvider(p.id)}
                                />
                                <span class="forge-form-provider-label">{p.label}</span>
                                <span class="forge-form-provider-cmd">{p.cmd}</span>
                            </label>
                        }</For>
                    </div>
                </div>

                <div class="forge-form-row">
                    <label class="forge-form-label">Description</label>
                    <input
                        class="forge-form-input"
                        value={description()}
                        onInput={(e) => setDescription(e.currentTarget.value)}
                        placeholder="Optional description"
                    />
                </div>

                <div class="forge-form-row">
                    <label class="forge-form-label">Working Directory</label>
                    <input
                        class="forge-form-input"
                        value={workingDirectory()}
                        onInput={(e) => setWorkingDirectory(e.currentTarget.value)}
                        placeholder="e.g. ~/.agentmux/agents/myagent"
                    />
                </div>

                <div class="forge-form-row">
                    <label class="forge-form-label">Shell</label>
                    <select
                        class="forge-form-input forge-form-select"
                        value={shell()}
                        onInput={(e) => setShell(e.currentTarget.value)}
                    >
                        <option value="">Default</option>
                        <option value="bash">bash</option>
                        <option value="pwsh">pwsh</option>
                        <option value="cmd">cmd</option>
                        <option value="zsh">zsh</option>
                    </select>
                </div>

                <div class="forge-form-row">
                    <label class="forge-form-label">Provider Flags</label>
                    <input
                        class="forge-form-input"
                        value={providerFlags()}
                        onInput={(e) => setProviderFlags(e.currentTarget.value)}
                        placeholder="Extra CLI arguments"
                    />
                </div>

                <div class="forge-form-row">
                    <label class="forge-form-label forge-form-checkbox-label">
                        <input
                            type="checkbox"
                            checked={autoStart()}
                            onInput={(e) => setAutoStart(e.currentTarget.checked)}
                        />
                        Auto Start
                    </label>
                </div>

                <Show when={error()}>
                    <div class="forge-form-error">{error()}</div>
                </Show>

                <div class="forge-form-actions">
                    <button type="submit" class="forge-btn-primary" disabled={loading() || !name().trim()}>
                        {loading() ? "Saving\u2026" : "Save"}
                    </button>
                    <button type="button" class="forge-btn-secondary" onClick={() => props.model.cancelForm()}>
                        Cancel
                    </button>
                </div>
            </form>
        </div>
    );
}
