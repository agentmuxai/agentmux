// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import clsx from "clsx";
import { JSX } from "solid-js";
import magnifySvg from "../asset/magnify.svg?raw";
import "./magnify.scss";

interface MagnifyIconProps {
    enabled: boolean;
}

export function MagnifyIcon(props: MagnifyIconProps): JSX.Element {
    return (
        <div class={clsx("magnify-icon", { enabled: props.enabled })} innerHTML={magnifySvg} />
    );
}
