// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { ErrorBoundary } from "@/app/element/errorboundary";
import { createContentBlockPlugin } from "@/app/element/markdown-contentblock-plugin";
import { transformBlocks } from "@/app/element/markdown-util";
import { findSafeSplitPoint } from "@/app/element/markdown-incremental";

/**
 * Deterministic work counters for the streaming-render invariants. Not a
 * profiler: these exist so a test can assert the SHAPE of the work (does
 * per-commit parse input stay bounded? is the processor built once?) without
 * measuring time. Reset via `__resetMarkdownRenderStats()` in tests.
 */
export const __markdownRenderStats = {
    /** Render-memo runs — one per committed text change. */
    commits: 0,
    /** Total characters handed to `processor.parse` across all commits. */
    parsedChars: 0,
    /** Times the unified processor + plugin chain has been constructed. */
    processorBuilds: 0,
};

export function __resetMarkdownRenderStats(): void {
    __markdownRenderStats.commits = 0;
    __markdownRenderStats.parsedChars = 0;
    __markdownRenderStats.processorBuilds = 0;
}
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
import { MarkdownImg, MarkdownSource, MuxBlock } from "./markdown-media";
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
    /** When true, skip OverlayScrollbars entirely and let `.content` use its
     *  plain CSS `overflow` — a real native/webkit scrollbar, styled by the
     *  same universal `*::-webkit-scrollbar` rule (app.scss) every other
     *  native scroll surface (CodeMirror, etc.) already uses, instead of
     *  OverlayScrollbars' JS-rendered auto-hide overlay track. No effect
     *  when `scrollable` is false. */
    nativeScrollbar?: boolean;
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
        nativeScrollbar = false,
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
        if (!heading) return;
        // In OverlayScrollbars mode the real scrolling element is its
        // generated viewport, not contentsEl itself; in nativeScrollbar mode
        // contentsEl IS the scrolling element (plain CSS overflow, no
        // OverlayScrollbars restructuring ever happened).
        const viewport = contentsOs ? contentsOs.elements().viewport : contentsEl;
        if (!viewport) return;
        const el = document.getElementById(idPrefix() + heading.slice(1));
        if (el) {
            const headingRect = el.getBoundingClientRect();
            const viewportRect = viewport.getBoundingClientRect();
            viewport.scrollBy({ top: headingRect.top - viewportRect.top });
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
        thead: (props: any) => <thead class="border-b border-border bg-hover">{props.children}</thead>,
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
        waveblock: (props: any) => <MuxBlock {...props} blockmap={contentBlocksMap()} />,
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

    // Counts WORK, not milliseconds — deliberately.
    //
    // The regression these guard against is algorithmic (re-parsing the whole
    // message per streaming commit, and rebuilding the plugin chain to do it).
    // A wall-clock assertion for that is flaky on shared CI runners to the
    // point of being theater — the position this repo already took in
    // Discussion #1161, and the reason these are counters instead. Counts are
    // deterministic, machine-independent, and fail for a comprehensible
    // reason. See docs/analysis/ANALYSIS_AGENT_PANE_TYPING_UNDER_LOAD_2026_09_22.md
    // and markdown-render-work.test.tsx, which asserts on them.
    //
    // Cost when nobody is looking: three integer adds per render.
    // eslint-disable-next-line
    const stats = __markdownRenderStats;

    // Stable across commits, deliberately: the processor below closes over
    // these once, and each render run refreshes their CONTENTS rather than
    // rebinding them. That is what lets one processor outlive a commit.
    const tocRef: TocItem[] = [];
    const blocksRef = new Map<string, any>();

    /**
     * Already-parsed prefix of the current message: the part far enough from
     * the end that no further streamed text can change how it parses. Holds
     * the hast children and toc entries it produced, so growth only ever costs
     * a parse of the NEW bytes. Null means "nothing frozen — parse it whole".
     */
    let frozen: {
        end: number;
        source: string;
        highlight: boolean;
        children: any[];
        toc: TocItem[];
    } | null = null;

    // The unified processor used to be constructed INSIDE the render memo, so
    // the entire plugin chain was rebuilt on every streaming commit — ~11x a
    // second per streaming pane, measured at roughly 4ms of the per-commit
    // cost (docs/analysis/ANALYSIS_AGENT_PANE_TYPING_UNDER_LOAD_2026_09_22.md
    // §6.5). It depends only on `highlight` and the slug prefix, never on the
    // text, so it is memoized on those and reused. This matters more, not
    // less, once per-commit parse cost comes down: at ~1ms of parse, a 4ms
    // rebuild would dominate what it is rebuilding.
    const processor = createMemo(() => {
        stats.processorBuilds++;
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
            [RemarkFlexibleToc, { tocRef }],
            [createContentBlockPlugin, { blocks: blocksRef }],
        ];

        return unified()
            .use(remarkParse)
            .use(remarkPlugins as any)
            .use(remarkRehype as any, { allowDangerousHtml: true })
            .use(rehypePlugins as any);
    });

    const renderedMarkdown = createMemo(() => {
        const txt = transformedText();
        stats.commits++;
        markStart("markdown-render");

        // Refresh the contents the memoized processor's plugins read at
        // transform time. `tocRef` must be CLEARED, not replaced — it is now
        // shared with the processor, and carrying entries across runs would
        // reintroduce the duplicate/stale-heading accumulation fixed in
        // issue #789.
        blocksRef.clear();
        for (const [k, v] of contentBlocksMap()) blocksRef.set(k, v);

        const highlight = props.highlight ?? true;

        /**
         * Parse ONE independent segment. `tocRef` is cleared per call, not per
         * commit, because a commit may now run this more than once and each
         * segment's headings have to be collected separately before the next
         * call wipes them.
         */
        const runSegment = (src: string): { children: any[]; toc: TocItem[] } => {
            tocRef.length = 0;
            stats.parsedChars += src.length;
            const hast: any = processor().runSync(processor().parse(src));
            return { children: hast.children ?? [], toc: tocRef.slice() };
        };

        try {
            // Incremental path: freeze the part of the message that can no
            // longer change meaning and parse only the trailing open block, so
            // each byte is parsed once instead of once per commit. Falls back
            // to a whole-document parse whenever a split cannot be PROVEN safe
            // (see markdown-incremental.ts) — that fallback is just today's
            // behavior, so the worst case is unchanged.
            const splitAt = findSafeSplitPoint(txt);

            let children: any[];
            let toc: TocItem[];

            if (splitAt <= 0) {
                frozen = null;
                const whole = runSegment(txt);
                children = whole.children;
                toc = whole.toc;
            } else {
                // The cache is only valid if this really is the same document
                // growing. A non-append edit (history restore, switching
                // messages) or a highlight flip invalidates it — the frozen
                // hast was produced by a different processor in that case.
                if (frozen && (frozen.highlight !== highlight || !txt.startsWith(frozen.source))) {
                    frozen = null;
                }
                if (!frozen || splitAt > frozen.end) {
                    const from = frozen ? frozen.end : 0;
                    const seg = runSegment(txt.slice(from, splitAt));
                    frozen = {
                        end: splitAt,
                        source: txt.slice(0, splitAt),
                        highlight,
                        children: frozen ? frozen.children.concat(seg.children) : seg.children,
                        toc: frozen ? frozen.toc.concat(seg.toc) : seg.toc,
                    };
                }
                const tail = runSegment(txt.slice(splitAt));
                children = frozen.children.concat(tail.children);
                toc = frozen.toc.concat(tail.toc);
            }

            const element = toJsxRuntime({ type: "root", children } as any, {
                jsx: jsx as any,
                jsxs: jsxs as any,
                Fragment: Fragment as any,
                passKeys: false,
                components: markdownComponents as any,
            }) as JSX.Element;
            // `toc` is assembled from per-segment copies above, never the live
            // `tocRef` — that array is cleared on the next `runSegment` call,
            // so handing it out would let a consumer observe it empty later.
            // Assembling per segment is also what preserves issue #789's
            // guarantee that the toc reflects only the current text.
            return { element, toc };
        } catch (e) {
            console.error("Markdown render error:", e);
            // A failed incremental parse must not leave a poisoned cache
            // behind for the next commit to build on.
            frozen = null;
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
        if (scrollable && !nativeScrollbar && contentsEl) {
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
