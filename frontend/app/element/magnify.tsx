// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { JSX } from "solid-js";
import magnifyUrl from "../asset/magnify.svg?url";
import "./magnify.scss";

interface MagnifyIconProps {
    enabled: boolean;
}

export function MagnifyIcon({ enabled }: MagnifyIconProps): JSX.Element {
    return (
        <div class={clsx("magnify-icon", { enabled })}>
            <img src={magnifyUrl} style={{ width: "100%", height: "100%" }} />
        </div>
    );
}
