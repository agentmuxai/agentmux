// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The row of chips that picks which kinds of entry the Audit lists.

import { For, type JSX } from "solid-js";

import { Button } from "@/app/element/ui";
import { KIND_CHIPS } from "./audit-shared";

export const AuditKindChips = (props: { kinds: ReadonlySet<string>; onToggle: (kind: string) => void }): JSX.Element => (
    <div class="audit-kind-chips" role="group" aria-label="Kinds of entry">
        <For each={KIND_CHIPS}>
            {(chip) => (
                <Button tone="quiet" density="compact" pressed={props.kinds.has(chip.kind)} onClick={() => props.onToggle(chip.kind)}>
                    {chip.label}
                </Button>
            )}
        </For>
    </div>
);

AuditKindChips.displayName = "AuditKindChips";
