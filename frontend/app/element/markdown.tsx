// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { CopyButton } from "@/app/element/copybutton";
import { writeText as clipboardWriteText } from "@/util/clipboard";
import { ErrorBoundary } from "@/app/element/errorboundary";
import { createContentBlockPlugin } from "@/app/element/markdown-contentblock-plugin";
import {
    MarkdownContentBlockType,
    resolveRemoteFile,
    resolveSrcSet,
    transformBlocks,
} from "@/app/element/markdown-util";
import remarkMermaidToTag from "@/app/element/remark-mermaid-to-tag";
import { boundNumber, useAtomValueSafe, cn } from "@/util/util";
import clsx from "clsx";
import { toJsxRuntime } from "hast-util-to-jsx-runtime";
import { OverlayScrollbars } from "overlayscrollbars";
import { createEffect, createMemo, createSignal, onCleanup, onMount, Show } from "solid-js";
import type { JSX } from "solid-js";
import { Fragment, jsx, jsxs } from "solid-js/h/jsx-runtime";
import { unified } from "unified";
import rehypeHighlight from "rehype-highlight";
import rehypeRaw from "rehype-raw";
import rehypeSanitize, { defaultSchema } from "rehype-sanitize";
import rehypeSlug from "rehype-slug";
import RemarkFlexibleToc, { TocItem } from "remark-flexible-toc";
import remarkGfm from "remark-gfm";
import remarkParse from "remark-parse";
import remarkRehype from "remark-rehype";
import { openLink } from "../store/global";
import { IconButton } from "./iconbutton";
import "./markdown.scss";

let mermaidInitialized = false;
let mermaidInstance: any = null;

const initializeMermaid = async () => {
    if (!mermaidInitialized) {
        const mermaid = await import("mermaid");
        mermaidInstance = mermaid.default;
        mermaidInstance.initialize({
            startOnLoad: false,
            theme: "dark",
            securityLevel: "strict",
        });
        mermaidInitialized = true;
    }
};

const Link = ({
    setFocusedHeading,
    props,
}: {
    props: JSX.AnchorHTMLAttributes<HTMLAnchorElement>;
    setFocusedHeading: (href: string) => void;
}) => {
    const onClick = (e: MouseEvent) => {
        e.preventDefault();
        const href = (props as any).href as string;
        if (!href) return;
        if (href.startsWith("#")) {
            setFocusedHeading(href);
        } else {
            openLink(href);
        }
    };
    return (
        <a href={(props as any).href} onClick={onClick}>
            {(props as any).children}
        </a>
    );
};

const Heading = ({ props, hnum }: { props: JSX.HTMLAttributes<HTMLHeadingElement>; hnum: number }) => {
    return (
        <div id={(props as any).id} class={clsx("heading", `is-${hnum}`)}>
            {(props as any).children}
        </div>
    );
};

let mermaidRenderCount = 0;

const Mermaid = ({ chart }: { chart: string }) => {
    let ref!: HTMLDivElement;
    const [svgContent, setSvgContent] = createSignal<string | null>(null);
    const [error, setError] = createSignal<string | null>(null);

    onMount(() => {
        let cancelled = false;
        const renderMermaid = async () => {
            try {
                setError(null);
                setSvgContent(null);

                await initializeMermaid();
                if (cancelled || !mermaidInstance) {
                    return;
                }

                // Normalize the chart text
                const normalizedChart = chart
                    .replace(/<br\s*\/?>/gi, "\n")
                    .replace(/\r\n?/g, "\n")
                    .replace(/\n+$/, "");

                const id = `mermaid-${++mermaidRenderCount}`;
                const { svg } = await mermaidInstance.render(id, normalizedChart);
                if (!cancelled) {
                    setSvgContent(svg);
                }
            } catch (err: any) {
                console.error("Error rendering mermaid diagram:", err);
                if (!cancelled) {
                    setError(err.message || String(err));
                }
            }
        };

        renderMermaid();
        onCleanup(() => {
            cancelled = true;
        });
    });

    return (
        <Show
            when={!error()}
            fallback={
                <div class="mermaid error">
                    <div style={{ color: "var(--error-color, #f44)", "margin-bottom": "8px" }}>
                        Failed to render diagram
                    </div>
                    <pre style={{ "white-space": "pre-wrap", opacity: 0.7, "font-size": "0.85em" }}>{chart}</pre>
                </div>
            }
        >
            <Show
                when={svgContent()}
                fallback={<div class="mermaid">Loading diagram...</div>}
            >
                <div class="mermaid" ref={ref} innerHTML={svgContent()} />
            </Show>
        </Show>
    );
};

const MermaidErrorFallback = ({ error, chart }: { error?: Error; chart: string }) => (
    <div class="mermaid error">
        <div style={{ color: "var(--error-color, #f44)", "margin-bottom": "8px" }}>Failed to render diagram</div>
        <pre style={{ "white-space": "pre-wrap", opacity: 0.7, "font-size": "0.85em" }}>{chart}</pre>
    </div>
);

const Code = ({ className = "", children }: { className?: string; children: any }) => {
    if (/\blanguage-mermaid\b/.test(className)) {
        const text = Array.isArray(children) ? children.join("") : String(children ?? "");
        return (
            <ErrorBoundary fallback={<MermaidErrorFallback chart={text} />}>
                <Mermaid chart={text} />
            </ErrorBoundary>
        );
    }
    return <code class={className}>{children}</code>;
};

type CodeBlockProps = {
    children: any;
    onClickExecute?: (cmd: string) => void;
};

const CodeBlock = ({ children, onClickExecute }: CodeBlockProps) => {
    const getTextContent = (children: any): string => {
        if (typeof children === "string") {
            return children;
        } else if (Array.isArray(children)) {
            return children.map(getTextContent).join("");
        } else if (children && children.props && children.props.children) {
            return getTextContent(children.props.children);
        }
        return "";
    };

    const handleCopy = async (e: MouseEvent) => {
        let textToCopy = getTextContent(children);
        textToCopy = textToCopy.replace(/\n$/, "");
        await clipboardWriteText(textToCopy);
    };

    const handleExecute = (e: MouseEvent) => {
        let textToCopy = getTextContent(children);
        textToCopy = textToCopy.replace(/\n$/, "");
        if (onClickExecute) {
            onClickExecute(textToCopy);
        }
    };

    return (
        <pre class="codeblock">
            {children}
            <div class="codeblock-actions">
                <CopyButton onClick={handleCopy} title="Copy" />
                {onClickExecute && (
                    <IconButton
                        decl={{
                            elemtype: "iconbutton",
                            icon: "regular@square-terminal",
                            click: handleExecute,
                        }}
                    />
                )}
            </div>
        </pre>
    );
};

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

type MarkdownProps = {
    text?: string;
    textAtom?: (() => string) | (() => Promise<string>);
    showTocAtom?: () => boolean;
    style?: JSX.CSSProperties;
    class?: string;
    contentClass?: string;
    onClickExecute?: (cmd: string) => void;
    resolveOpts?: MarkdownResolveOpts;
    scrollable?: boolean;
    rehype?: boolean;
    fontSizeOverride?: number;
    fixedFontSizeOverride?: number;
};

const Markdown = ({
    text,
    textAtom,
    showTocAtom,
    style,
    class: className,
    contentClass: contentClassName,
    resolveOpts,
    fontSizeOverride,
    fixedFontSizeOverride,
    scrollable = true,
    rehype = true,
    onClickExecute,
}: MarkdownProps) => {
    const textAtomValue = useAtomValueSafe<string>(textAtom as any);
    const tocItems: TocItem[] = [];
    const showToc = useAtomValueSafe(showTocAtom) ?? false;
    const [focusedHeading, setFocusedHeading] = createSignal<string | null>(null);

    let contentsEl!: HTMLDivElement;
    let tocEl!: HTMLDivElement;
    let contentsOs: OverlayScrollbars | null = null;
    let tocOs: OverlayScrollbars | null = null;

    const [idPrefix] = createSignal<string>(crypto.randomUUID());

    const resolvedText = createMemo(() => textAtomValue ?? text ?? "");

    const transformedOutput = createMemo(() => transformBlocks(resolvedText()));
    const transformedText = createMemo(() => transformedOutput().content);
    const contentBlocksMap = createMemo(() => transformedOutput().blocks);

    createEffect(() => {
        const heading = focusedHeading();
        if (heading && contentsOs) {
            const { viewport } = contentsOs.elements();
            const el = document.getElementById(idPrefix() + heading.slice(1));
            if (el) {
                const headingRect = el.getBoundingClientRect();
                const viewportRect = viewport.getBoundingClientRect();
                viewport.scrollBy({ top: headingRect.top - viewportRect.top });
            }
        }
    });

    const markdownComponents: Record<string, any> = {
        a: (props: any) => <Link props={props} setFocusedHeading={setFocusedHeading} />,
        p: (props: any) => <div class="paragraph" {...props} />,
        h1: (props: any) => <Heading props={props} hnum={1} />,
        h2: (props: any) => <Heading props={props} hnum={2} />,
        h3: (props: any) => <Heading props={props} hnum={3} />,
        h4: (props: any) => <Heading props={props} hnum={4} />,
        h5: (props: any) => <Heading props={props} hnum={5} />,
        h6: (props: any) => <Heading props={props} hnum={6} />,
        img: (props: any) => <MarkdownImg props={props} resolveOpts={resolveOpts} />,
        source: (props: any) => <MarkdownSource props={props} resolveOpts={resolveOpts} />,
        code: Code,
        pre: (props: any) => <CodeBlock children={props.children} onClickExecute={onClickExecute} />,
        waveblock: (props: any) => <WaveBlock {...props} blockmap={contentBlocksMap()} />,
        mermaidblock: (props: any) => {
            const getTextContent = (children: any): string => {
                if (typeof children === "string") return children;
                if (Array.isArray(children)) return children.map(getTextContent).join("");
                if (children && typeof children === "object" && children.props?.children)
                    return getTextContent(children.props.children);
                return String(children || "");
            };
            const chartText = getTextContent(props.children);
            return (
                <ErrorBoundary fallback={<MermaidErrorFallback chart={chartText} />}>
                    <Mermaid chart={chartText} />
                </ErrorBoundary>
            );
        },
    };

    const renderedMarkdown = createMemo(() => {
        const txt = transformedText();
        const tocRef: TocItem[] = [];
        const tocRefObj = { current: tocRef };

        const rehypePlugins: any[] = rehype
            ? [
                  rehypeRaw,
                  rehypeHighlight,
                  () =>
                      rehypeSanitize({
                          ...defaultSchema,
                          attributes: {
                              ...defaultSchema.attributes,
                              span: [
                                  ...(defaultSchema.attributes?.span || []),
                                  ["className", /^hljs-./],
                                  ["srcset"],
                                  ["media"],
                                  ["type"],
                              ],
                              waveblock: [["blockkey"]],
                          },
                          tagNames: [
                              ...(defaultSchema.tagNames || []),
                              "span",
                              "waveblock",
                              "picture",
                              "source",
                              "mermaidblock",
                          ],
                      }),
                  () => rehypeSlug({ prefix: idPrefix() }),
              ]
            : [];

        const remarkPlugins: any[] = [
            remarkMermaidToTag,
            remarkGfm,
            [RemarkFlexibleToc, { tocRef: tocRefObj.current }],
            [createContentBlockPlugin, { blocks: contentBlocksMap() }],
        ];

        const processor = unified()
            .use(remarkParse)
            .use(remarkPlugins as any)
            .use(remarkRehype as any, { allowDangerousHtml: true })
            .use(rehypePlugins as any);

        try {
            const mdast = processor.parse(txt);
            const hast = processor.runSync(mdast);
            // Update tocItems after processing
            tocRefObj.current.forEach((item) => tocItems.push(item));
            return toJsxRuntime(hast as any, {
                jsx: jsx as any,
                jsxs: jsxs as any,
                Fragment: Fragment as any,
                passKeys: false,
                components: markdownComponents as any,
            }) as JSX.Element;
        } catch (e) {
            console.error("Markdown render error:", e);
            return <pre>{txt}</pre>;
        }
    });

    onMount(() => {
        if (scrollable && contentsEl) {
            contentsOs = OverlayScrollbars(contentsEl, { scrollbars: { autoHide: "leave" } });
            onCleanup(() => contentsOs?.destroy());
        }
    });

    const mergedStyle = createMemo((): JSX.CSSProperties => {
        const s: Record<string, any> = { ...(style ?? {}) };
        if (fontSizeOverride != null) {
            s["--markdown-font-size"] = `${boundNumber(fontSizeOverride, 6, 64)}px`;
        }
        if (fixedFontSizeOverride != null) {
            s["--markdown-fixed-font-size"] = `${boundNumber(fixedFontSizeOverride, 6, 64)}px`;
        }
        return s;
    });

    return (
        <div class={clsx("markdown", className)} style={mergedStyle() as any}>
            <Show
                when={scrollable}
                fallback={
                    <div class={cn("content non-scrollable", contentClassName)}>
                        {renderedMarkdown()}
                    </div>
                }
            >
                <div class={cn("content", contentClassName)} ref={contentsEl}>
                    {renderedMarkdown()}
                </div>
            </Show>
            <Show when={showToc && tocItems.length > 0}>
                <div class="toc mt-1" ref={tocEl}>
                    <div class="toc-inner">
                        <h4 class="font-bold">Table of Contents</h4>
                        {tocItems.map((item) => (
                            <a
                                class="toc-item"
                                style={{ "--indent-factor": item.depth } as any}
                                onClick={() => setFocusedHeading(item.href)}
                            >
                                {item.value}
                            </a>
                        ))}
                    </div>
                </div>
            </Show>
            <Show when={showToc && tocItems.length === 0}>
                <div class="toc mt-1">
                    <div class="toc-inner">
                        <h4 class="font-bold">Table of Contents</h4>
                        <div class="toc-item toc-empty text-secondary" style={{ "--indent-factor": 2 } as any}>
                            No sub-headings found
                        </div>
                    </div>
                </div>
            </Show>
        </div>
    );
};

export { Markdown };
