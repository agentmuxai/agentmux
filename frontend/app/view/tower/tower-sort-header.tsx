// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// A sortable column header, shared by Tower's tables.

import { Button } from "@/app/element/ui";
import clsx from "clsx";
import type { TowerViewModel } from "./tower-model";
import { nextSort, type SortKey } from "./tower-util";

export function SortHeader(props: {
    model: TowerViewModel;
    key: SortKey;
    label: string;
    numeric?: boolean;
    title?: string;
}) {
    const active = () => props.model.sort().key === props.key;
    return (
        <th
            class={clsx(props.numeric && "tower-num")}
            aria-sort={active() ? (props.model.sort().desc ? "descending" : "ascending") : "none"}
        >
            <Button
                tone="quiet"
                density="compact"
                class="tower-sort"
                title={props.title}
                icon={active() ? (props.model.sort().desc ? "arrow-down" : "arrow-up") : undefined}
                onClick={() => props.model.setSort(nextSort(props.model.sort(), props.key))}
            >
                {props.label}
            </Button>
        </th>
    );
}
