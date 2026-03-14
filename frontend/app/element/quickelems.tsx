// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { JSX } from "solid-js";
import "./quickelems.scss";

function CenteredLoadingDiv(): JSX.Element {
    return <CenteredDiv>loading...</CenteredDiv>;
}

function CenteredDiv(props: { children?: JSX.Element }): JSX.Element {
    return (
        <div class="centered-div">
            <div>{props.children}</div>
        </div>
    );
}

export { CenteredDiv, CenteredLoadingDiv };
