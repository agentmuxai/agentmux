// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * BundleImportPreviewModalPanel — Step 2 of the ABF import flow
 * (docs/specs/SPEC_ABF_IMPORT_UI_PHASE3_2026_08_02.md §4 Step 2 / §4.1).
 *
 * Renders the `bundle.import.preview` response as a selection checklist.
 * All selection state is built here and handed to Step 3 via `onNext` —
 * this panel makes no RPC calls of its own.
 */

import type { JSX } from "solid-js";

import { Button } from "@/element/button";
import { SkillsSection, WidgetsSection } from "./bundle-import-preview-items";
import {
    ContextSection,
    InstructionsSection,
    McpSection,
    NameField,
    RequirementsSection,
    WarningsBanner,
} from "./bundle-import-preview-sections";
import { createPreviewActions, createPreviewState, type BundleImportPreviewProps } from "./bundle-import-preview-state";

export const BundleImportPreviewModalPanel = (props: BundleImportPreviewProps): JSX.Element => {
    const s = createPreviewState(props);
    const act = createPreviewActions(s);
    const { preview } = s;

    return (
        <>
            <header class="modal-panel-header">
                <h2 class="modal-panel-title">Preview &amp; Select</h2>
                <p class="modal-panel-description">
                    Choose what to bring in from <strong>{preview.name}</strong>.
                </p>
            </header>
            <div class="modal-panel-body bundle-import-preview-body">
                <NameField s={s} act={act} />
                <InstructionsSection s={s} act={act} />
                <ContextSection s={s} act={act} />
                <SkillsSection s={s} act={act} />
                <WidgetsSection s={s} />
                <McpSection s={s} act={act} />
                <RequirementsSection s={s} act={act} />
                <WarningsBanner s={s} act={act} />
            </div>
            <footer class="modal-panel-footer">
                <Button onClick={() => props.onCancel()} data-modal-dismiss>
                    Cancel
                </Button>
                <Button onClick={act.next} className="green solid">
                    Next
                </Button>
            </footer>
        </>
    );
};

BundleImportPreviewModalPanel.displayName = "BundleImportPreviewModalPanel";
