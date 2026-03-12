// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { IconButton } from "@/app/element/iconbutton";
import { atoms, WOS } from "@/store/global";
import { RpcApi } from "@/store/wshclientapi";
import { TabRpcClient } from "@/store/wshrpcutil";
import { fireAndForget, isBlank, makeIconClass } from "@/util/util";
import clsx from "clsx";
import type { JSX } from "solid-js";
import { createSignal, Show } from "solid-js";
import "./titlebar.scss";

interface TitleBarProps {
    blockId: string;
    blockMeta: MetaType;
    title?: string;
    icon?: string;
    color?: string;
    onTitleChange?: (newTitle: string) => void;
}

function TitleBar({ blockId, blockMeta, title, icon, color, onTitleChange }: TitleBarProps): JSX.Element {
    const [isEditing, setIsEditing] = createSignal(false);
    const [localTitle, setLocalTitle] = createSignal(title || "");
    const fullConfig = atoms.fullConfigAtom();

    // Check if pane labels are enabled
    const paneLabelSettings = fullConfig?.settings?.["pane-labels"];
    const isEnabled = paneLabelSettings?.enabled ?? false;
    const displayMode = paneLabelSettings?.["display-mode"] ?? "always";
    const showIcons = paneLabelSettings?.["show-icons"] ?? true;
    const maxLength = paneLabelSettings?.["max-length"] ?? 50;

    // Check if this specific pane has labels hidden
    const hideOverride = blockMeta?.["pane-title:hide"];

    const [isHovered, setIsHovered] = createSignal(false);

    // Don't render if disabled globally or hidden for this pane
    if (!isEnabled || hideOverride) {
        return null;
    }

    // Handle display mode
    if (displayMode === "never") return null;

    const handleSave = () => {
        setIsEditing(false);
        const trimmedTitle = localTitle().trim();
        if (trimmedTitle !== title) {
            fireAndForget(async () => {
                await RpcApi.SetMetaCommand(TabRpcClient, {
                    oref: WOS.makeORef("block", blockId),
                    meta: { "pane-title": trimmedTitle } as any,
                });
            });
            onTitleChange?.(trimmedTitle);
        }
    };

    const handleKeyDown = (e: KeyboardEvent) => {
        if (e.key === "Enter") {
            e.preventDefault();
            handleSave();
        } else if (e.key === "Escape") {
            e.preventDefault();
            setLocalTitle(title || "");
            setIsEditing(false);
        }
    };

    const displayTitle = () => {
        const t = localTitle();
        return t.length > maxLength ? t.slice(0, maxLength) + "..." : t;
    };
    const effectiveIcon = icon || blockMeta?.["pane-title:icon"];
    const effectiveColor = color || blockMeta?.["pane-title:color"];

    return (
        <Show when={displayMode !== "on-hover" || isHovered()}>
            <div
                class={clsx("pane-title-bar", { "is-editing": isEditing(), "is-hovered": isHovered() })}
                onMouseEnter={() => setIsHovered(true)}
                onMouseLeave={() => setIsHovered(false)}
            >
                <Show when={showIcons && effectiveIcon && !isBlank(effectiveIcon)}>
                    <div class="pane-title-icon" style={{ color: effectiveColor }}>
                        <i class={makeIconClass(effectiveIcon, false, { defaultIcon: "square" })} />
                    </div>
                </Show>
                <Show
                    when={isEditing()}
                    fallback={
                        <span
                            class="pane-title-text"
                            onClick={() => setIsEditing(true)}
                            title={localTitle().length > maxLength ? localTitle() : undefined}
                        >
                            {displayTitle() || "Untitled Pane"}
                        </span>
                    }
                >
                    <input
                        class="pane-title-input"
                        value={localTitle()}
                        onInput={(e) => setLocalTitle((e.target as HTMLInputElement).value)}
                        onBlur={handleSave}
                        onKeyDown={handleKeyDown}
                        maxLength={maxLength}
                        autofocus
                        placeholder="Enter pane title..."
                    />
                </Show>
                <Show when={isHovered() && !isEditing()}>
                    <IconButton
                        className="pane-title-edit-btn"
                        decl={{ elemtype: "iconbutton", icon: "pencil", click: () => setIsEditing(true) }}
                    />
                </Show>
            </div>
        </Show>
    );
}

export { TitleBar };
