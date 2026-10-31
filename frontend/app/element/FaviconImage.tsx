// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A site's favicon, or a generic globe when there is none to show: no URL, or one that fails to
 * load (a 404, a broken site, a blocked image). A broken image would otherwise leave a blank slot.
 * The failure is remembered per URL, so the next favicon that arrives gets its own try.
 */

import { createSignal, Show, type JSX } from "solid-js";
import { makeIconClass } from "@/util/util";

export function FaviconImage(props: {
    src: string | undefined;
    /** Class for the `<img>`; the globe carries `favicon-globe`. */
    class?: string;
    size?: number;
    onLoad?: (e: Event & { currentTarget: HTMLImageElement }) => void;
}): JSX.Element {
    const [failedSrc, setFailedSrc] = createSignal<string | undefined>();
    const showable = () => !!props.src && failedSrc() !== props.src;
    return (
        <Show when={showable()} fallback={<i class={`${makeIconClass("globe", true)} favicon-globe`} aria-hidden="true" />}>
            <img
                class={props.class}
                src={props.src}
                alt=""
                aria-hidden="true"
                width={props.size}
                height={props.size}
                onLoad={props.onLoad}
                onError={() => setFailedSrc(props.src)}
            />
        </Show>
    );
}
