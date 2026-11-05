// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * One category of what a bundle holds, as a group of kit Cards under a small heading (Instructions, Skills; Packs
 * will be one more). A category's cards open that item's own editor or details.
 */

import type { JSX } from "solid-js";

import { SectionHeader } from "@/app/element/ui";

export function BundleCategory(props: { title: string; children: JSX.Element }): JSX.Element {
    return (
        <section class="bundle-category">
            <SectionHeader variant="group" level={3} title={props.title} />
            {props.children}
        </section>
    );
}
