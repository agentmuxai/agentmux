// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { JSX } from "solid-js";
import "./quickelems.scss";

function CenteredDiv(props: { children?: JSX.Element }): JSX.Element {
    return (
        <div class="centered-div">
            <div>{props.children}</div>
        </div>
    );
}

export { CenteredDiv };
