// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A list of the map's nodes that keeps each node's element for as long as its key is in the list.
 *
 * The map's data is rebuilt whenever anything about any agent changes, so an agent is a new object each time with the
 * same key. `<For>` follows the object, and would throw away every node and make it again: the node under a pointer
 * that rests on it goes, and with it the hover label's anchor, the focus and the cursor. Here the element stays and
 * only what it reads from the item changes.
 */

import { createMemo, For, type Accessor, type JSX } from "solid-js";

const sameKeys = (a: readonly string[], b: readonly string[]) => a.length === b.length && a.every((key, i) => key === b[i]);

export function ByKey<T extends { key: string }>(props: {
    each: readonly T[];
    /** One item's node: `item()` is the item now, which changes while the node stays. */
    children: (item: Accessor<T>, key: string) => JSX.Element;
}): JSX.Element {
    const keys = createMemo(() => props.each.map((item) => item.key), [], { equals: sameKeys });
    const byKey = createMemo(() => new Map(props.each.map((item) => [item.key, item] as const)));
    return (
        <For each={keys()}>
            {(key) => {
                // An item that has just gone is read once more before its node does: the last one stands in.
                const item = createMemo<T>((last) => byKey().get(key) ?? last!);
                return props.children(item, key);
            }}
        </For>
    );
}
