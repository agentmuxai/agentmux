// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { JSX } from "solid-js";

import { WardenJobsManager } from "@/app/view/warden-jobs/warden-jobs-manager";
import type { WardenViewModel } from "./warden-model";
import "./warden-view.scss";

export function WardenView(props: { model: WardenViewModel }): JSX.Element {
    const model = props.model;
    // Ctrl+scroll and Ctrl/Cmd +/-/0 zoom this pane through the shared handlers
    // (app.tsx, keymodel.ts): the `paneZoom` capability is all it needs.
    return (
        // warden-container carries container-type so the Jobs manager inside can answer
        // @container warden queries. A container element cannot respond to its own container query.
        <div class="warden-container">
            <div class="warden-view" style={{ zoom: model.zoomAtom() }}>
                <WardenJobsManager visible={model.shown()} onFailingCount={model.setJobsFailing} />
            </div>
        </div>
    );
}

WardenView.displayName = "WardenView";
