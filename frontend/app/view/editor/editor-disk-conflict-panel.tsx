// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The question under the tab strip when the file of a tab with unsaved edits changed on disk: Reload or Keep mine. A panel in the pane, never a window modal. */

import type { JSX } from "solid-js";
import { Show } from "solid-js";
import { Button } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import type { EditorViewModel } from "./editor-model";

export function EditorDiskConflictPanel(props: { model: EditorViewModel }): JSX.Element {
    const model = props.model;
    const tab = () => model.activeTabAtom();
    return (
        <Show when={tab()}>
            {(tb) => (
                <div class="editor-disk-conflict" role="alert" data-zoom-scope="chrome">
                    <span class="editor-disk-conflict-text">
                        {t("editor.diskConflict.message", { name: tb().displayName ?? tb().filePath.split(/[\\/]/).pop() })}
                    </span>
                    <span class="editor-disk-conflict-actions">
                        <Button density="compact" tone="danger" onClick={() => void model.diskConflicts.reload(tb())}>
                            {t("editor.diskConflict.reload")}
                        </Button>
                        <Button density="compact" onClick={() => void model.diskConflicts.keep(tb())}>
                            {t("editor.diskConflict.keepMine")}
                        </Button>
                    </span>
                </div>
            )}
        </Show>
    );
}
