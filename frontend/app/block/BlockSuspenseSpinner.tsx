// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { BrainSpinner } from "@/app/element/BrainSpinner";
import type { JSX } from "solid-js";
import { logWhileShown } from "./pane-cover-log";

/** The mark a block draws while its view is suspended after the cover lifted, with its time on screen written to the `[pane-cover]` log. */
export function BlockSuspenseSpinner(props: { blockId: string; agent: () => string | undefined }): JSX.Element {
    logWhileShown(props.blockId, "suspense-fallback", props.agent);
    return <BrainSpinner />;
}
