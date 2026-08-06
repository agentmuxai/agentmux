// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { ErrorBoundary } from "@/app/element/errorboundary";
import { createContentBlockPlugin } from "@/app/element/markdown-contentblock-plugin";
import { transformBlocks } from "@/app/element/markdown-util";
import { ALIGN_CLASS_REGEX, rehypeAlignToClass } from "@/app/element/rehype-align-to-class";
import remarkMermaidToTag from "@/app/element/remark-mermaid-to-tag";
import { TableBlock } from "@/app/element/table-block";
import { boundNumber, useAtomValueSafe, cn } from "@/util/util";
import { markEnd, markStart } from "@/perf";
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
import { rehypeLinkify } from "./rehype-linkify";
import { Code, CodeBlock } from "./markdown-codeblock";
import { MarkdownImg, MarkdownSource, WaveBlock } from "./markdown-media";
import { Mermaid, MermaidErrorFallback } from "./markdown-mermaid";
import "./markdown.scss";

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
    /** When false, skip the (expensive) syntax-highlighting rehype plugin
     *  while keeping every other plugin (sanitize, etc.). Read reactively so
     *  streaming callers can defer highlighting until the content settles. */
    highlight?: boolean;
    fontSizeOverride?: number;
    fixedFontSizeOverride?: number;
};

const Markdown = (props: MarkdownProps) => {
    // `text` is read via props.text inside the resolvedText memo so
    // streaming markdown (where the parent passes a continuously
    // growing string) re-renders. Other props are mostly static (atom
    // refs, style, class) — destructure those for terseness without
    // losing reactivity on the one prop that needs it.
    // (codex P1 on PR #786 — the streaming buffer keeps MarkdownBlock
    // mounted across token deltas, but Markdown's own destructuring
    // froze the captured `text` value at first mount.)
    const {
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
    } = props;
    const [focusedHeading, setFocusedHeading] = createSignal<string | null>(null);

    let contentsEl!: HTMLDivElement;
    let tocEl!: HTMLDivElement;
    let contentsOs: OverlayScrollbars | null = null;
    let tocOs: OverlayScrollbars | null = null;

    const [idPrefix] = createSignal<string>(crypto.randomUUID());

    // `useAtomValueSafe(textAtom)` must be called INSIDE the memo, not hoisted
    // to a plain outer `const`. `textAtom` (e.g. the editor's `() => liveDoc()`)
    // is a live reactive getter — calling it once at component-setup time
    // (before setup, before any effect has had a chance to seed the real
    // value) froze the memo's only input, so it never recomputed again for
    // the lifetime of this component instance. On a freshly-mounted markdown
    // preview this snapshot was almost always "" (the seed value hasn't
    // landed yet), producing a permanently blank preview — "fixed" only by
    // remounting the component (e.g. toggling Source → Preview), which
    // re-captures a fresh (by-then-correct) snapshot. Reading the accessor
    // here makes the memo properly track its underlying signal.
    const resolvedText = createMemo(() => useAtomValueSafe<string>(textAtom as any) ?? props.text ?? "");
    const showToc = createMemo(() => useAtomValueSafe(showTocAtom) ?? false);

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
        table: (props: any) => <TableBlock>{props.children}</TableBlock>,
        thead: (props: any) => <thead class="border-b border-border bg-white/[0.03]">{props.children}</thead>,
        tbody: (props: any) => <tbody>{props.children}</tbody>,
        tr: (props: any) => <tr class="border-b border-border/40 last:border-0">{props.children}</tr>,
        th: (props: any) => {
            // Spread sanitizer-survived attributes (colspan, rowspan,
            // scope) so raw HTML tables retain their structure. Codex
            // P2 on PR #754. Override className so alignment classes
            // from rehypeAlignToClass + the cell's typography classes
            // both apply.
            const { children, className, ...rest } = props;
            return (
                <th
                    {...rest}
                    class={cn(
                        "px-3 py-2 text-left text-xs font-semibold uppercase tracking-wide text-primary",
                        className,
                    )}
                >
                    {children}
                </th>
            );
        },
        td: (props: any) => {
            const { children, className, ...rest } = props;
            return (
                <td
                    {...rest}
                    class={cn("px-3 py-2 text-sm text-secondary", className)}
                >
                    {children}
                </td>
            );
        },
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
        markStart("markdown-render");
        const tocRef: TocItem[] = [];
        const tocRefObj = { current: tocRef };

        const rehypePlugins: any[] = rehype
            ? [
                  rehypeRaw,
                  ...((props.highlight ?? true) ? [rehypeHighlight] : []),
                  rehypeAlignToClass,
                  rehypeLinkify,
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
                              th: [
                                  ...(defaultSchema.attributes?.th || []),
                                  ["className", ALIGN_CLASS_REGEX],
                              ],
                              td: [
                                  ...(defaultSchema.attributes?.td || []),
                                  ["className", ALIGN_CLASS_REGEX],
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
            const element = toJsxRuntime(hast as any, {
                jsx: jsx as any,
                jsxs: jsxs as any,
                Fragment: Fragment as any,
                passKeys: false,
                components: markdownComponents as any,
            }) as JSX.Element;
            // `tocRef` is local to this memo run, so the toc reflects only the
            // current text — it no longer accumulates stale/duplicate headings
            // across re-renders (issue #789).
            return { element, toc: tocRefObj.current };
        } catch (e) {
            console.error("Markdown render error:", e);
            return { element: <pre>{txt}</pre>, toc: [] as TocItem[] };
        } finally {
            markEnd("markdown-render", `len=${txt.length}`);
        }
    });

    // Reactive TOC, derived from the render memo. Re-derives whenever the text
    // changes so streaming markdown keeps the TOC in sync, instead of the old
    // non-reactive array that stayed stuck on the first heading set.
    const tocItems = createMemo<TocItem[]>(() => renderedMarkdown().toc);

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
                        {renderedMarkdown().element}
                    </div>
                }
            >
                <div class={cn("content", contentClassName)} ref={contentsEl}>
                    {/* OverlayScrollbars (below, onMount) restructures contentsEl's
                        DOM once, moving whatever children exist AT INIT TIME into
                        its generated .os-viewport. On first open, liveDoc/textAtom
                        is often still "" when this initial move happens (the real
                        content lands a beat later via a reactive update), which
                        orphaned Solid's insertion anchor for {renderedMarkdown().element}
                        outside the new viewport — a permanently blank preview until
                        something (e.g. a Source/Preview toggle) fully remounted this
                        component. A single, never-replaced wrapper div is the node
                        OverlayScrollbars moves; Solid's reactive updates always target
                        children of THIS stable node, so they keep landing correctly
                        regardless of OverlayScrollbars' init timing. */}
                    <div class="markdown-content-inner">
                        {renderedMarkdown().element}
                    </div>
                </div>
            </Show>
            <Show when={showToc() && tocItems().length > 0}>
                <div class="toc mt-1" ref={tocEl}>
                    <div class="toc-inner">
                        <h4 class="font-bold">Table of Contents</h4>
                        {tocItems().map((item) => (
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
            <Show when={showToc() && tocItems().length === 0}>
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
