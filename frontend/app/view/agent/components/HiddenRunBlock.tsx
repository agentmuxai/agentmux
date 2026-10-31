// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The one line standing for a run of rows the pane's view hides (virtualization/verbosity.ts): what
 * is in it ("12 tool calls, 3 thoughts"), a warning part when tool calls failed, and a click that
 * opens the run in place. The open state is the run's pin, kept with the pane like other pins.
 */

import { Show, type JSX } from "solid-js";
import { Button } from "@/app/element/ui/Button";
import type { HiddenRunNode } from "../types";
import { hiddenRunLabel } from "../virtualization/verbosity";
import "./HiddenRunBlock.scss";

interface HiddenRunBlockProps {
    node: HiddenRunNode;
    onToggle: () => void;
}

export const HiddenRunBlock = (props: HiddenRunBlockProps): JSX.Element => (
    <div class="hidden-run" data-hidden-run={props.node.id}>
        <Button
            variant="link"
            icon={props.node.open ? "chevron-down" : "chevron-right"}
            aria-expanded={props.node.open}
            onClick={props.onToggle}
        >
            {hiddenRunLabel(props.node.counts)}
            <Show when={props.node.failed > 0}>
                <span class="hidden-run-failed">{`, ${props.node.failed} failed`}</span>
            </Show>
            {props.node.open ? "" : " hidden"}
        </Button>
    </div>
);
