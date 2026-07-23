// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { MarkdownContentBlockType, resolveRemoteFile, resolveSrcSet } from "@/app/element/markdown-util";
import { createSignal, onMount, Show } from "solid-js";
import type { JSX } from "solid-js";

const MarkdownSource = ({
    props,
    resolveOpts,
}: {
    props: JSX.SourceHTMLAttributes<HTMLSourceElement> & {
        srcSet?: string;
        media?: string;
    };
    resolveOpts: MarkdownResolveOpts;
}) => {
    const [resolvedSrcSet, setResolvedSrcSet] = createSignal<string>((props as any).srcSet ?? "");
    const [resolving, setResolving] = createSignal<boolean>(true);

    onMount(() => {
        const resolvePath = async () => {
            const resolved = await resolveSrcSet((props as any).srcSet, resolveOpts);
            setResolvedSrcSet(resolved);
            setResolving(false);
        };
        resolvePath();
    });

    return (
        <Show when={!resolving()}>
            <source srcset={resolvedSrcSet()} media={(props as any).media} />
        </Show>
    );
};

interface WaveBlockProps {
    blockkey: string;
    blockmap: Map<string, MarkdownContentBlockType>;
}

const WaveBlock = (props: WaveBlockProps) => {
    const { blockkey, blockmap } = props;
    const block = blockmap.get(blockkey);
    if (block == null) {
        return null;
    }
    const sizeInKB = Math.round((block.content.length / 1024) * 10) / 10;
    const displayName = block.id.replace(/^"|"$/g, "");
    return (
        <div class="waveblock">
            <div class="wave-block-content">
                <div class="wave-block-icon">
                    <i class="fas fa-file-code"></i>
                </div>
                <div class="wave-block-info">
                    <span class="wave-block-filename">{displayName}</span>
                    <span class="wave-block-size">{sizeInKB} KB</span>
                </div>
            </div>
        </div>
    );
};

const MarkdownImg = ({
    props,
    resolveOpts,
}: {
    props: JSX.ImgHTMLAttributes<HTMLImageElement>;
    resolveOpts: MarkdownResolveOpts;
}) => {
    const src = (props as any).src as string;
    const srcSet = (props as any).srcSet as string;

    const [resolvedSrc, setResolvedSrc] = createSignal<string | null>(src);
    const [resolvedSrcSet, setResolvedSrcSet] = createSignal<string | null>(srcSet ?? null);
    const [resolvedStr, setResolvedStr] = createSignal<string | null>(null);
    const [resolving, setResolving] = createSignal<boolean>(true);

    onMount(() => {
        if (src?.startsWith("data:image/")) {
            setResolving(false);
            setResolvedSrc(src);
            setResolvedStr(null);
            return;
        }
        if (resolveOpts == null) {
            setResolving(false);
            setResolvedSrc(null);
            setResolvedStr(`[img:${src}]`);
            return;
        }

        const resolveFn = async () => {
            const [rSrc, rSrcSet] = await Promise.all([
                resolveRemoteFile(src, resolveOpts),
                resolveSrcSet(srcSet, resolveOpts),
            ]);

            setResolvedSrc(rSrc);
            setResolvedSrcSet(rSrcSet);
            setResolvedStr(null);
            setResolving(false);
        };
        resolveFn();
    });

    return (
        <Show when={!resolving()}>
            <Show when={resolvedStr() != null} fallback={
                <Show when={resolvedSrc() != null} fallback={<span>[img]</span>}>
                    <img {...(props as any)} src={resolvedSrc()} srcset={resolvedSrcSet()} />
                </Show>
            }>
                <span>{resolvedStr()}</span>
            </Show>
        </Show>
    );
};

export { MarkdownSource, MarkdownImg, WaveBlock };
