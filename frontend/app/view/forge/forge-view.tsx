// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import React, { memo, useCallback, useState } from "react";
import { useAtomValue } from "jotai";
import type { ForgeViewModel } from "./forge-model";
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

    return <ForgeList model={model} />;
});

ForgeView.displayName = "ForgeView";

// ── Agent List ────────────────────────────────────────────────────────────────

const ForgeList: React.FC<{ model: ForgeViewModel }> = memo(({ model }) => {
    const agents = useAtomValue(model.agentsAtom);

    return (
        <div className="forge-pane">
            <div className="forge-header">
                <span className="forge-title">Forge</span>
            </div>
            <div className="forge-divider" />
            {agents.length === 0 ? (
                <div className="forge-empty">
                    <span className="forge-empty-icon">✦</span>
                    <span className="forge-empty-label">No agents yet</span>
                    <span className="forge-empty-sub">Create your first agent</span>
                    <button className="forge-new-btn" onClick={() => model.startCreate()}>
                        + New Agent
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
                    </div>
                </>
            )}
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

    const handleEdit = useCallback(() => {
        model.startEdit(agent);
    }, [model, agent]);

    const providerLabel = PROVIDERS.find((p) => p.id === agent.provider)?.label ?? agent.provider;

    return (
        <div className="forge-card">
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
                    onClick={handleDelete}
                    onBlur={() => setConfirming(false)}
                    title={confirming ? "Click again to confirm" : "Delete"}
                >
                    {confirming ? "Sure?" : "✕"}
                </button>
            </div>
        </div>
    );
});

ForgeAgentCard.displayName = "ForgeAgentCard";

// ── Create / Edit Form ────────────────────────────────────────────────────────

const ForgeForm: React.FC<{ model: ForgeViewModel }> = memo(({ model }) => {
    const view = useAtomValue(model.viewAtom);
    const editingAgent = useAtomValue(model.editingAgentAtom);
    const loading = useAtomValue(model.loadingAtom);
    const error = useAtomValue(model.errorAtom);

    const isEdit = view === "edit" && editingAgent != null;

    const [name, setName] = useState(editingAgent?.name ?? "");
    const [icon, setIcon] = useState(editingAgent?.icon ?? "✦");
    const [provider, setProvider] = useState(editingAgent?.provider ?? "claude");
    const [description, setDescription] = useState(editingAgent?.description ?? "");

    const handleSubmit = useCallback(
        async (e: React.FormEvent) => {
            e.preventDefault();
            if (!name.trim()) return;
            if (isEdit) {
                await model.updateAgent({
                    id: editingAgent!.id,
                    name: name.trim(),
                    icon: icon || "✦",
                    provider,
                    description,
                });
            } else {
                await model.createAgent({
                    name: name.trim(),
                    icon: icon || "✦",
                    provider,
                    description,
                });
            }
        },
        [isEdit, model, editingAgent, name, icon, provider, description]
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
                        placeholder="✦"
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

                {error && <div className="forge-form-error">{error}</div>}

                <div className="forge-form-actions">
                    <button type="submit" className="forge-btn-primary" disabled={loading || !name.trim()}>
                        {loading ? "Saving…" : "Save"}
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
