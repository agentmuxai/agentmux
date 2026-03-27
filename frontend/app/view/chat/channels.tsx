// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { CollapsibleMenu } from "@/app/element/collapsiblemenu";
import type { JSX } from "solid-js";

import "./channels.scss";

function Channels(props: { channels: MenuItem[] }): JSX.Element {
    return <CollapsibleMenu className="channel-list" items={props.channels} />;
}

export { Channels };
