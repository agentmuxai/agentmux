// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import React, { memo, useCallback, useEffect, useState } from "react";
import { useAtomValue } from "jotai";
import type { ForgeViewModel, ContentTabId, DetailSection } from "./forge-model";
import { CONTENT_TABS, CONTENT_TAB_LABELS, SKILL_TYPES } from "./forge-model";
import "./forge-view.scss";

const PROVIDERS = [
    { id: "claude", label: "Claude Code", cmd: "claude --output-format stream-json" },
    { id: "codex", label: "Codex CLI", cmd: "codex --full-auto" },
    { id: "gemini", label: "Gemini CLI", cmd: "gemini --yolo" },
];

// ── Main view ────────────────────────────────────────────────────────────────

export const ForgeView: React.FC<ViewComponentProps<ForgeViewModel>> = memo(({ model }) => {
    const view = useAtomValue(model.viewAtom);

    if (view === "create" || view === "edit") {
        return <ForgeForm model={model} />;
    }

    if (view === "detail") {
        return <ForgeDetail model={model} />;
    }

    return <ForgeList model={model} />;
});

ForgeView.displayName = "ForgeView";

// ── Agent List ────────────────────────────────────────────────────────────────

const ForgeList: React.FC<{ model: ForgeViewModel }> = memo(({ model }) => {
    const agents = useAtomValue(model.agentsAtom);
    const [showImport, setShowImport] = useState(false);

    return (
        <div className="forge-pane">
            <div className="forge-header">
                <span className="forge-title">Forge</span>
            </div>
            <div className="forge-divider" />
            {agents.length === 0 ? (
                <div className="forge-empty">
                    <span className="forge-empty-icon">&#10022;</span>
                    <span className="forge-empty-label">No agents yet</span>
                    <span className="forge-empty-sub">Create your first agent</span>
                    <button className="forge-new-btn" onClick={() => model.startCreate()}>
                        + New Agent
                    </button>
                    <button className="forge-new-btn forge-import-btn" onClick={() => setShowImport(true)}>
                        Import from Claw
                    </button>
                </div>
            ) : (
                <>
                    <div className="forge-list">
                        {agents.map((agent) => (
                            <ForgeAgentCard key={agent.id} agent={agent} model={model} />
                        ))}
                    </div>
                    <div className="forge-list-footer">
                        <button className="forge-new-btn" onClick={() => model.startCreate()}>
                            + New Agent
                        </button>
                        <button className="forge-new-btn forge-import-btn" onClick={() => setShowImport(true)}>
                            Import from Claw
                        </button>
                    </div>
                </>
            )}
            {showImport && <ForgeImportForm model={model} onClose={() => setShowImport(false)} />}
        </div>
    );
});

ForgeList.displayName = "ForgeList";

// ── Agent Card ────────────────────────────────────────────────────────────────

const ForgeAgentCard: React.FC<{ agent: ForgeAgent; model: ForgeViewModel }> = memo(({ agent, model }) => {
    const [confirming, setConfirming] = useState(false);

    const handleDelete = useCallback(async () => {
        if (!confirming) {
            setConfirming(true);
            return;
        }
        setConfirming(false);
        await model.deleteAgent(agent.id);
    }, [confirming, model, agent.id]);

    const handleClick = useCallback(() => {
        model.openDetail(agent);
    }, [model, agent]);

    const handleEdit = useCallback((e: React.MouseEvent) => {
        e.stopPropagation();
        model.startEdit(agent);
    }, [model, agent]);

    const handleDeleteClick = useCallback((e: React.MouseEvent) => {
        e.stopPropagation();
        handleDelete();
    }, [handleDelete]);

    const providerLabel = PROVIDERS.find((p) => p.id === agent.provider)?.label ?? agent.provider;

    return (
        <div className="forge-card" onClick={handleClick}>
            <span className="forge-card-icon">{agent.icon}</span>
            <div className="forge-card-info">
                <span className="forge-card-name">{agent.name}</span>
                <span className="forge-card-provider">{providerLabel}</span>
                {agent.description && <span className="forge-card-desc">{agent.description}</span>}
            </div>
            <div className="forge-card-actions">
                <button className="forge-card-btn" onClick={handleEdit} title="Edit">
                    Edit
                </button>
                <button
                    className={`forge-card-btn forge-card-btn-delete${confirming ? " confirming" : ""}`}
                    onClick={handleDeleteClick}
                    onBlur={() => setConfirming(false)}
                    title={confirming ? "Click again to confirm" : "Delete"}
                >
                    {confirming ? "Sure?" : "\u2715"}
                </button>
            </div>
        </div>
    );
});

ForgeAgentCard.displayName = "ForgeAgentCard";

// ── Detail View ───────────────────────────────────────────────────────────────

const DETAIL_SECTIONS: { id: DetailSection; label: string }[] = [
    { id: "content", label: "Content" },
    { id: "skills", label: "Skills" },
    { id: "history", label: "History" },
];

const ForgeDetail: React.FC<{ model: ForgeViewModel }> = memo(({ model }) => {
    const agent = useAtomValue(model.detailAgentAtom);
    const activeSection = useAtomValue(model.activeSectionAtom);

    if (!agent) return null;

    const providerLabel = PROVIDERS.find((p) => p.id === agent.provider)?.label ?? agent.provider;

    return (
        <div className="forge-pane">
            <div className="forge-detail-header">
                <button className="forge-back-btn" onClick={() => model.closeDetail()}>
                    &larr; Back
                </button>
                <span className="forge-detail-icon">{agent.icon}</span>
                <div className="forge-detail-info">
                    <span className="forge-detail-name">{agent.name}</span>
                    <span className="forge-detail-sub">
                        {providerLabel}
                        {agent.description ? ` \u2022 ${agent.description}` : ""}
                    </span>
                </div>
                <button className="forge-card-btn" onClick={() => model.startEditFromDetail()}>
                    Edit
                </button>
            </div>
            <div className="forge-divider" />
            <div className="forge-section-tabs">
                {DETAIL_SECTIONS.map((s) => (
                    <DetailSectionButton key={s.id} section={s} activeSection={activeSection} model={model} agentId={agent.id} />
                ))}
            </div>
            <div className="forge-section-body">
                {activeSection === "content" && <ForgeContentSection model={model} agentId={agent.id} />}
                {activeSection === "skills" && <ForgeSkillsPanel model={model} agentId={agent.id} />}
                {activeSection === "history" && <ForgeHistoryPanel model={model} agentId={agent.id} />}
            </div>
        </div>
    );
});

ForgeDetail.displayName = "ForgeDetail";

// ── Detail Section Button ────────────────────────────────────────────────────

const DetailSectionButton: React.FC<{
    section: { id: DetailSection; label: string };
    activeSection: DetailSection;
    model: ForgeViewModel;
    agentId: string;
}> = memo(({ section, activeSection, model, agentId }) => {
    const handleClick = useCallback(async () => {
        const { globalStore } = await import("@/app/store/global");
        globalStore.set(model.activeSectionAtom, section.id);
        if (section.id === "skills") {
            await model.loadSkills(agentId);
        } else if (section.id === "history") {
            await model.loadHistory(agentId);
        }
    }, [model, section.id, agentId]);

    return (
        <button
            className={`forge-section-tab${activeSection === section.id ? " active" : ""}`}
            onClick={handleClick}
        >
            {section.label}
        </button>
    );
});

DetailSectionButton.displayName = "DetailSectionButton";

// ── Content Section (existing tabs) ──────────────────────────────────────────

const ForgeContentSection: React.FC<{ model: ForgeViewModel; agentId: string }> = memo(({ model, agentId }) => {
    const contentMap = useAtomValue(model.contentAtom);
    const activeTab = useAtomValue(model.activeTabAtom);
    const contentLoading = useAtomValue(model.contentLoadingAtom);

    return (
        <>
            <div className="forge-content-tabs">
                {CONTENT_TABS.map((tab) => (
                    <ContentTabButton key={tab} tab={tab} activeTab={activeTab} model={model} />
                ))}
            </div>
            <div className="forge-content-body">
                {contentLoading ? (
                    <div className="forge-content-loading">Loading...</div>
                ) : (
                    <ContentEditor
                        agentId={agentId}
                        contentType={activeTab}
                        content={contentMap[activeTab]}
                        model={model}
                    />
                )}
            </div>
        </>
    );
});

ForgeContentSection.displayName = "ForgeContentSection";

// ── Content Tab Button ────────────────────────────────────────────────────────

const ContentTabButton: React.FC<{
    tab: ContentTabId;
    activeTab: ContentTabId;
    model: ForgeViewModel;
}> = memo(({ tab, activeTab, model }) => {
    const handleClick = useCallback(async () => {
        const { globalStore } = await import("@/app/store/global");
        globalStore.set(model.activeTabAtom, tab);
    }, [model, tab]);

    return (
        <button
            className={`forge-content-tab${activeTab === tab ? " active" : ""}`}
            onClick={handleClick}
        >
            {CONTENT_TAB_LABELS[tab]}
        </button>
    );
});

ContentTabButton.displayName = "ContentTabButton";

// ── Content Editor ────────────────────────────────────────────────────────────

const ContentEditor: React.FC<{
    agentId: string;
    contentType: string;
    content: ForgeContent | undefined;
    model: ForgeViewModel;
}> = memo(({ agentId, contentType, content, model }) => {
    const saving = useAtomValue(model.contentSavingAtom);
    const [editing, setEditing] = useState(false);
    const [draft, setDraft] = useState("");

    const currentContent = content?.content ?? "";
    const charCount = currentContent.length;

    const handleStartEdit = useCallback(() => {
        setDraft(currentContent);
        setEditing(true);
    }, [currentContent]);

    const handleCancel = useCallback(() => {
        setEditing(false);
        setDraft("");
    }, []);

    const handleSave = useCallback(async () => {
        await model.saveContent(agentId, contentType, draft);
        setEditing(false);
    }, [model, agentId, contentType, draft]);

    if (editing) {
        return (
            <div className="forge-content-editor">
                <textarea
                    className="forge-content-textarea"
                    value={draft}
                    onChange={(e) => setDraft(e.target.value)}
                    autoFocus
                    spellCheck={false}
                />
                <div className="forge-content-editor-footer">
                    <span className="forge-content-charcount">{draft.length} chars</span>
                    <div className="forge-content-editor-actions">
                        <button className="forge-btn-primary" onClick={handleSave} disabled={saving}>
                            {saving ? "Saving..." : "Save"}
                        </button>
                        <button className="forge-btn-secondary" onClick={handleCancel} disabled={saving}>
                            Cancel
                        </button>
                    </div>
                </div>
            </div>
        );
    }

    return (
        <div className="forge-content-display">
            {currentContent ? (
                <pre className="forge-content-pre">{currentContent}</pre>
            ) : (
                <div className="forge-content-empty">No content yet</div>
            )}
            <div className="forge-content-display-footer">
                <span className="forge-content-charcount">
                    {charCount > 0 ? `${charCount} chars` : ""}
                    {content?.updated_at ? ` \u2022 saved` : ""}
                </span>
                <button className="forge-btn-primary" onClick={handleStartEdit}>
                    Edit Content
                </button>
            </div>
        </div>
    );
});

ContentEditor.displayName = "ContentEditor";

// ── Skills Panel ──────────────────────────────────────────────────────────────

const ForgeSkillsPanel: React.FC<{ model: ForgeViewModel; agentId: string }> = memo(({ model, agentId }) => {
    const skills = useAtomValue(model.skillsAtom);
    const loading = useAtomValue(model.skillsLoadingAtom);
    const editingSkill = useAtomValue(model.editingSkillAtom);
    const [showForm, setShowForm] = useState(false);

    const handleNewSkill = useCallback(async () => {
        const { globalStore } = await import("@/app/store/global");
        globalStore.set(model.editingSkillAtom, null);
        setShowForm(true);
    }, [model]);

    const handleEditSkill = useCallback(async (skill: ForgeSkill) => {
        const { globalStore } = await import("@/app/store/global");
        globalStore.set(model.editingSkillAtom, skill);
        setShowForm(true);
    }, [model]);

    const handleCloseForm = useCallback(async () => {
        const { globalStore } = await import("@/app/store/global");
        globalStore.set(model.editingSkillAtom, null);
        setShowForm(false);
    }, [model]);

    if (loading) {
        return <div className="forge-content-loading">Loading skills...</div>;
    }

    if (showForm) {
        return (
            <ForgeSkillForm
                model={model}
                agentId={agentId}
                skill={editingSkill}
                onClose={handleCloseForm}
            />
        );
    }

    return (
        <div className="forge-skills-panel">
            {skills.length === 0 ? (
                <div className="forge-content-empty">No skills yet</div>
            ) : (
                <div className="forge-skills-list">
                    {skills.map((skill) => (
                        <ForgeSkillCard
                            key={skill.id}
                            skill={skill}
                            model={model}
                            onEdit={handleEditSkill}
                        />
                    ))}
                </div>
            )}
            <div className="forge-skills-footer">
                <button className="forge-btn-primary" onClick={handleNewSkill}>
                    + Add Skill
                </button>
            </div>
        </div>
    );
});

ForgeSkillsPanel.displayName = "ForgeSkillsPanel";

// ── Skill Card ────────────────────────────────────────────────────────────────

const ForgeSkillCard: React.FC<{
    skill: ForgeSkill;
    model: ForgeViewModel;
    onEdit: (skill: ForgeSkill) => void;
}> = memo(({ skill, model, onEdit }) => {
    const [confirming, setConfirming] = useState(false);

    const handleDelete = useCallback(async () => {
        if (!confirming) {
            setConfirming(true);
            return;
        }
        setConfirming(false);
        await model.deleteSkill(skill.id);
    }, [confirming, model, skill.id]);

    return (
        <div className="forge-skill-card">
            <div className="forge-skill-card-info">
                <div className="forge-skill-card-top">
                    <span className="forge-skill-card-name">{skill.name}</span>
                    <span className="forge-skill-type-badge">{skill.skill_type}</span>
                </div>
                {skill.trigger && (
                    <span className="forge-skill-card-trigger">/{skill.trigger}</span>
                )}
                {skill.description && (
                    <span className="forge-skill-card-desc">{skill.description}</span>
                )}
            </div>
            <div className="forge-card-actions">
                <button className="forge-card-btn" onClick={() => onEdit(skill)} title="Edit">
                    Edit
                </button>
                <button
                    className={`forge-card-btn forge-card-btn-delete${confirming ? " confirming" : ""}`}
                    onClick={handleDelete}
                    onBlur={() => setConfirming(false)}
                    title={confirming ? "Click again to confirm" : "Delete"}
                >
                    {confirming ? "Sure?" : "\u2715"}
                </button>
            </div>
        </div>
    );
});

ForgeSkillCard.displayName = "ForgeSkillCard";

// ── Skill Form ────────────────────────────────────────────────────────────────

const ForgeSkillForm: React.FC<{
    model: ForgeViewModel;
    agentId: string;
    skill: ForgeSkill | null;
    onClose: () => void;
}> = memo(({ model, agentId, skill, onClose }) => {
    const error = useAtomValue(model.errorAtom);
    const isEdit = skill != null;

    const [name, setName] = useState(skill?.name ?? "");
    const [trigger, setTrigger] = useState(skill?.trigger ?? "");
    const [skillType, setSkillType] = useState(skill?.skill_type ?? "prompt");
    const [description, setDescription] = useState(skill?.description ?? "");
    const [content, setContent] = useState(skill?.content ?? "");

    const handleSubmit = useCallback(async (e: React.FormEvent) => {
        e.preventDefault();
        if (!name.trim()) return;
        if (isEdit) {
            await model.updateSkill({
                id: skill!.id,
                name: name.trim(),
                trigger,
                skill_type: skillType,
                description,
                content,
            });
        } else {
            await model.createSkill({
                agent_id: agentId,
                name: name.trim(),
                trigger,
                skill_type: skillType,
                description,
                content,
            });
        }
        onClose();
    }, [isEdit, model, skill, agentId, name, trigger, skillType, description, content, onClose]);

    return (
        <div className="forge-skill-form">
            <div className="forge-skill-form-header">
                <span className="forge-title-sub">{isEdit ? "Edit Skill" : "New Skill"}</span>
            </div>
            <form onSubmit={handleSubmit}>
                <div className="forge-form-row">
                    <label className="forge-form-label">Name</label>
                    <input
                        className="forge-form-input"
                        value={name}
                        onChange={(e) => setName(e.target.value)}
                        placeholder="Skill name"
                        autoFocus
                        required
                    />
                </div>
                <div className="forge-form-row">
                    <label className="forge-form-label">Trigger</label>
                    <input
                        className="forge-form-input"
                        value={trigger}
                        onChange={(e) => setTrigger(e.target.value)}
                        placeholder="/command-name"
                    />
                </div>
                <div className="forge-form-row">
                    <label className="forge-form-label">Type</label>
                    <select
                        className="forge-form-input forge-form-select"
                        value={skillType}
                        onChange={(e) => setSkillType(e.target.value)}
                    >
                        {SKILL_TYPES.map((t) => (
                            <option key={t} value={t}>{t}</option>
                        ))}
                    </select>
                </div>
                <div className="forge-form-row">
                    <label className="forge-form-label">Description</label>
                    <input
                        className="forge-form-input"
                        value={description}
                        onChange={(e) => setDescription(e.target.value)}
                        placeholder="Brief description"
                    />
                </div>
                <div className="forge-form-row forge-form-row-col">
                    <label className="forge-form-label">Content</label>
                    <textarea
                        className="forge-content-textarea forge-skill-content"
                        value={content}
                        onChange={(e) => setContent(e.target.value)}
                        placeholder="Skill content (prompt, command, etc.)"
                        spellCheck={false}
                    />
                </div>
                {error && <div className="forge-form-error">{error}</div>}
                <div className="forge-form-actions">
                    <button type="submit" className="forge-btn-primary" disabled={!name.trim()}>
                        {isEdit ? "Update" : "Create"}
                    </button>
                    <button type="button" className="forge-btn-secondary" onClick={onClose}>
                        Cancel
                    </button>
                </div>
            </form>
        </div>
    );
});

ForgeSkillForm.displayName = "ForgeSkillForm";

// ── History Panel ─────────────────────────────────────────────────────────────

const ForgeHistoryPanel: React.FC<{ model: ForgeViewModel; agentId: string }> = memo(({ model, agentId }) => {
    const entries = useAtomValue(model.historyAtom);
    const loading = useAtomValue(model.historyLoadingAtom);
    const [searchQuery, setSearchQuery] = useState("");

    const handleSearch = useCallback(async () => {
        if (searchQuery.trim()) {
            await model.searchHistory(agentId, searchQuery.trim());
        } else {
            await model.loadHistory(agentId);
        }
    }, [model, agentId, searchQuery]);

    const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
        if (e.key === "Enter") {
            handleSearch();
        }
    }, [handleSearch]);

    // Group entries by session_date
    const groupedEntries = entries.reduce<Record<string, ForgeHistory[]>>((acc, entry) => {
        const date = entry.session_date;
        if (!acc[date]) acc[date] = [];
        acc[date].push(entry);
        return acc;
    }, {});
    const sortedDates = Object.keys(groupedEntries).sort().reverse();

    return (
        <div className="forge-history-panel">
            <div className="forge-history-search">
                <input
                    className="forge-form-input"
                    value={searchQuery}
                    onChange={(e) => setSearchQuery(e.target.value)}
                    onKeyDown={handleKeyDown}
                    placeholder="Search history..."
                />
                <button className="forge-card-btn" onClick={handleSearch}>
                    Search
                </button>
            </div>
            <div className="forge-history-list">
                {loading ? (
                    <div className="forge-content-loading">Loading history...</div>
                ) : entries.length === 0 ? (
                    <div className="forge-content-empty">No history entries</div>
                ) : (
                    sortedDates.map((date) => (
                        <div key={date} className="forge-history-group">
                            <div className="forge-history-date">{date}</div>
                            {groupedEntries[date].map((entry) => (
                                <div key={entry.id} className="forge-history-entry">
                                    <span className="forge-history-time">
                                        {new Date(entry.timestamp).toLocaleTimeString()}
                                    </span>
                                    <span className="forge-history-text">{entry.entry}</span>
                                </div>
                            ))}
                        </div>
                    ))
                )}
            </div>
        </div>
    );
});

ForgeHistoryPanel.displayName = "ForgeHistoryPanel";

// ── Import Form ───────────────────────────────────────────────────────────────

const ForgeImportForm: React.FC<{ model: ForgeViewModel; onClose: () => void }> = memo(({ model, onClose }) => {
    const importing = useAtomValue(model.importingAtom);
    const error = useAtomValue(model.errorAtom);
    const [workspacePath, setWorkspacePath] = useState("");
    const [agentName, setAgentName] = useState("");

    // Auto-fill agent name from path
    useEffect(() => {
        if (workspacePath && !agentName) {
            const parts = workspacePath.replace(/\\/g, "/").split("/").filter(Boolean);
            if (parts.length > 0) {
                setAgentName(parts[parts.length - 1]);
            }
        }
    }, [workspacePath, agentName]);

    const handleSubmit = useCallback(async (e: React.FormEvent) => {
        e.preventDefault();
        if (!workspacePath.trim() || !agentName.trim()) return;
        await model.importFromClaw(workspacePath.trim(), agentName.trim());
        onClose();
    }, [model, workspacePath, agentName, onClose]);

    return (
        <div className="forge-import-overlay">
            <div className="forge-import-dialog">
                <div className="forge-skill-form-header">
                    <span className="forge-title-sub">Import from Claw</span>
                </div>
                <form onSubmit={handleSubmit}>
                    <div className="forge-form-row">
                        <label className="forge-form-label">Workspace Path</label>
                        <input
                            className="forge-form-input"
                            value={workspacePath}
                            onChange={(e) => setWorkspacePath(e.target.value)}
                            placeholder="~/.claw/workspaces/agent1"
                            autoFocus
                            required
                        />
                    </div>
                    <div className="forge-form-row">
                        <label className="forge-form-label">Agent Name</label>
                        <input
                            className="forge-form-input"
                            value={agentName}
                            onChange={(e) => setAgentName(e.target.value)}
                            placeholder="Agent name"
                            required
                        />
                    </div>
                    {error && <div className="forge-form-error">{error}</div>}
                    <div className="forge-form-actions">
                        <button type="submit" className="forge-btn-primary" disabled={importing || !workspacePath.trim() || !agentName.trim()}>
                            {importing ? "Importing..." : "Import"}
                        </button>
                        <button type="button" className="forge-btn-secondary" onClick={onClose} disabled={importing}>
                            Cancel
                        </button>
                    </div>
                </form>
            </div>
        </div>
    );
});

ForgeImportForm.displayName = "ForgeImportForm";

// ── Create / Edit Form ────────────────────────────────────────────────────────

const ForgeForm: React.FC<{ model: ForgeViewModel }> = memo(({ model }) => {
    const view = useAtomValue(model.viewAtom);
    const editingAgent = useAtomValue(model.editingAgentAtom);
    const loading = useAtomValue(model.loadingAtom);
    const error = useAtomValue(model.errorAtom);

    const isEdit = view === "edit" && editingAgent != null;

    const [name, setName] = useState(editingAgent?.name ?? "");
    const [icon, setIcon] = useState(editingAgent?.icon ?? "\u2726");
    const [provider, setProvider] = useState(editingAgent?.provider ?? "claude");
    const [description, setDescription] = useState(editingAgent?.description ?? "");
    const [workingDirectory, setWorkingDirectory] = useState(editingAgent?.working_directory ?? "");
    const [shell, setShell] = useState(editingAgent?.shell ?? "");
    const [providerFlags, setProviderFlags] = useState(editingAgent?.provider_flags ?? "");
    const [autoStart, setAutoStart] = useState(editingAgent?.auto_start === 1);

    const handleSubmit = useCallback(
        async (e: React.FormEvent) => {
            e.preventDefault();
            if (!name.trim()) return;
            if (isEdit) {
                await model.updateAgent({
                    id: editingAgent!.id,
                    name: name.trim(),
                    icon: icon || "\u2726",
                    provider,
                    description,
                    working_directory: workingDirectory,
                    shell,
                    provider_flags: providerFlags,
                    auto_start: autoStart ? 1 : 0,
                });
            } else {
                await model.createAgent({
                    name: name.trim(),
                    icon: icon || "\u2726",
                    provider,
                    description,
                    working_directory: workingDirectory,
                    shell,
                    provider_flags: providerFlags,
                    auto_start: autoStart ? 1 : 0,
                });
            }
        },
        [isEdit, model, editingAgent, name, icon, provider, description, workingDirectory, shell, providerFlags, autoStart]
    );

    const title = isEdit ? "Edit Agent" : "New Agent";

    return (
        <div className="forge-pane">
            <div className="forge-header">
                <span className="forge-title">
                    Forge&nbsp;/&nbsp;<span className="forge-title-sub">{title}</span>
                </span>
            </div>
            <div className="forge-divider" />
            <form className="forge-form" onSubmit={handleSubmit}>
                <div className="forge-form-row">
                    <label className="forge-form-label">Icon</label>
                    <input
                        className="forge-form-input forge-form-input-icon"
                        value={icon}
                        maxLength={4}
                        onChange={(e) => setIcon(e.target.value)}
                        placeholder="\u2726"
                    />
                </div>

                <div className="forge-form-row">
                    <label className="forge-form-label">Name</label>
                    <input
                        className="forge-form-input"
                        value={name}
                        onChange={(e) => setName(e.target.value)}
                        placeholder="My Agent"
                        autoFocus
                        required
                    />
                </div>

                <div className="forge-form-row forge-form-row-col">
                    <label className="forge-form-label">Provider</label>
                    <div className="forge-form-providers">
                        {PROVIDERS.map((p) => (
                            <label key={p.id} className="forge-form-provider-opt">
                                <input
                                    type="radio"
                                    name="provider"
                                    value={p.id}
                                    checked={provider === p.id}
                                    onChange={() => setProvider(p.id)}
                                />
                                <span className="forge-form-provider-label">{p.label}</span>
                                <span className="forge-form-provider-cmd">{p.cmd}</span>
                            </label>
                        ))}
                    </div>
                </div>

                <div className="forge-form-row">
                    <label className="forge-form-label">Description</label>
                    <input
                        className="forge-form-input"
                        value={description}
                        onChange={(e) => setDescription(e.target.value)}
                        placeholder="Optional description"
                    />
                </div>

                <div className="forge-form-row">
                    <label className="forge-form-label">Working Directory</label>
                    <input
                        className="forge-form-input"
                        value={workingDirectory}
                        onChange={(e) => setWorkingDirectory(e.target.value)}
                        placeholder="e.g. ~/.agentmux/agents/myagent"
                    />
                </div>

                <div className="forge-form-row">
                    <label className="forge-form-label">Shell</label>
                    <select
                        className="forge-form-input forge-form-select"
                        value={shell}
                        onChange={(e) => setShell(e.target.value)}
                    >
                        <option value="">Default</option>
                        <option value="bash">bash</option>
                        <option value="pwsh">pwsh</option>
                        <option value="cmd">cmd</option>
                        <option value="zsh">zsh</option>
                    </select>
                </div>

                <div className="forge-form-row">
                    <label className="forge-form-label">Provider Flags</label>
                    <input
                        className="forge-form-input"
                        value={providerFlags}
                        onChange={(e) => setProviderFlags(e.target.value)}
                        placeholder="Extra CLI arguments"
                    />
                </div>

                <div className="forge-form-row">
                    <label className="forge-form-label forge-form-checkbox-label">
                        <input
                            type="checkbox"
                            checked={autoStart}
                            onChange={(e) => setAutoStart(e.target.checked)}
                        />
                        Auto Start
                    </label>
                </div>

                {error && <div className="forge-form-error">{error}</div>}

                <div className="forge-form-actions">
                    <button type="submit" className="forge-btn-primary" disabled={loading || !name.trim()}>
                        {loading ? "Saving\u2026" : "Save"}
                    </button>
                    <button type="button" className="forge-btn-secondary" onClick={() => model.cancelForm()}>
                        Cancel
                    </button>
                </div>
            </form>
        </div>
    );
});

ForgeForm.displayName = "ForgeForm";
