// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0 AND MIT
//
// `keyArray` and `<Key>`, copied from @solid-primitives/keyed 1.5.3
// (https://github.com/solidjs-community/solid-primitives, packages/keyed) with
// one change: a new slot's root no longer closes over the reconcile pass's
// result array. In the original, `addNewItem` writes `list[i] = save.mapped`
// inside `createRoot`, so every live slot kept that pass's whole array — the
// mapped DOM of every row in the list at that moment — reachable from its
// `dispose`. Rows that later left the list stayed alive, detached, for as long
// as any slot born in the same pass lived. See
// docs/specs/SPEC_KEYED_SLOT_RETENTION_FIX_2026_10_12.md.
//
// Not copied: the server-rendering branch (AgentMux renders only in the
// browser) and the package's other exports (unused here).
//
// The original's licence:
//
// MIT License
//
// Copyright (c) 2021 Solid Primitives Working Group
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all
// copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

import {
    $TRACK,
    createMemo,
    createRoot,
    createSignal,
    onCleanup,
    untrack,
    type Accessor,
    type JSX,
    type Setter,
} from "solid-js";

const FALLBACK = Symbol("fallback");

type Slot<T, U> = {
    dispose: () => void;
    setItem?: Setter<T>;
    setIndex?: Setter<number>;
    mapped?: U;
};

function disposeAll(list: Iterable<{ dispose: () => void }>) {
    for (const o of list) o.dispose();
}

/**
 * Reactively maps an array by a key: an item keeps its mapped output (and
 * the reactive scope that built it) for as long as its key stays in the
 * list. Item and index are signals.
 */
export function keyArray<T, U, K>(
    items: Accessor<readonly T[] | undefined | null | false>,
    keyFn: (item: T, index: number) => K,
    mapFn: (v: Accessor<T>, i: Accessor<number>) => U,
    options: { fallback?: Accessor<U> } = {}
): Accessor<U[]> {
    const prev = new Map<K | typeof FALLBACK, Slot<T, U>>();
    onCleanup(() => disposeAll(prev.values()));

    return () => {
        const list = items() || [];
        // Reading $TRACK subscribes to a store array's top level.
        // eslint-disable-next-line @typescript-eslint/no-unused-expressions
        (list as unknown as Record<symbol, unknown>)[$TRACK];
        return untrack(() => {
            // fast path for empty arrays
            if (!list.length) {
                disposeAll(prev.values());
                prev.clear();
                if (!options.fallback) return [];
                const fb = createRoot((dispose) => {
                    prev.set(FALLBACK, { dispose });
                    return options.fallback!();
                });
                return [fb];
            }

            const result: U[] = new Array(list.length);

            // fast path for new create or after fallback
            const fb = prev.get(FALLBACK);
            if (!prev.size || fb) {
                fb?.dispose();
                prev.delete(FALLBACK);
                for (let i = 0; i < list.length; i++) {
                    const item = list[i];
                    addNewItem(result, item, i, keyFn(item, i));
                }
                return result;
            }

            const prevKeys = new Set(prev.keys());
            for (let i = 0; i < list.length; i++) {
                const item = list[i];
                const key = keyFn(item, i);
                prevKeys.delete(key);
                const lookup = prev.get(key);
                if (lookup) {
                    result[i] = lookup.mapped!;
                    lookup.setIndex?.(i);
                    lookup.setItem!(() => item);
                } else addNewItem(result, item, i, key);
            }
            for (const key of prevKeys) {
                prev.get(key)?.dispose();
                prev.delete(key);
            }
            return result;
        });
    };

    function addNewItem(list: U[], item: T, i: number, key: K) {
        // The one change from upstream: the root below must not reference
        // `list` (see the file comment), so its output comes out through
        // `mapped` and is stored after the root returns.
        let mapped!: U;
        createRoot((dispose) => {
            const [getItem, setItem] = createSignal(item);
            const save: Slot<T, U> = { setItem, dispose };
            if (mapFn.length > 1) {
                const [index, setIndex] = createSignal(i);
                save.setIndex = setIndex;
                save.mapped = mapFn(getItem, index);
            } else save.mapped = (mapFn as (v: Accessor<T>) => U)(getItem);
            prev.set(key, save);
            mapped = save.mapped;
        });
        list[i] = mapped;
    }
}

/**
 * Renders `each` keyed by `by`: an item keeps its DOM for as long as its key
 * stays in the list, wherever it moves. Children get the item and its index
 * as signals; `fallback` shows when the list is empty.
 *
 * ```tsx
 * <Key each={items()} by={(item) => item.id} fallback={<div>No items</div>}>
 *   {(item, index) => <div data-index={index()}>{item().name}</div>}
 * </Key>
 * ```
 */
export function Key<T>(props: {
    each?: readonly T[] | null | false;
    by: ((v: T) => unknown) | keyof T;
    fallback?: JSX.Element;
    children: (v: Accessor<T>, i: Accessor<number>) => JSX.Element;
}): JSX.Element {
    const { by } = props;
    return createMemo(
        keyArray(
            () => props.each,
            typeof by === "function" ? by : (v: T) => v[by],
            props.children,
            "fallback" in props ? { fallback: () => props.fallback } : undefined
        )
    ) as unknown as JSX.Element;
}
