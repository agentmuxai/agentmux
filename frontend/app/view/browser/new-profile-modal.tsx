// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * New profile: a name and a colour, then a tab in the new profile opens in
 * the pane it was asked from (docs/specs/SPEC_BROWSER_PANE_PROFILES_MENU_2026_10_09.md §3).
 * Rendered inside the browser pane, so it locks that pane only.
 */

import { createSignal, Show, type JSX } from "solid-js";
import { Button, Field, TextInput } from "@/app/element/ui";
import { Modal, ModalBody, ModalFooter, ModalHeader } from "@/element/modal";
import { PaletteSwatches } from "@/app/components/palette-swatches";
import { createBrowserProfile, nextProfileColor } from "./browser-profiles";
import type { BrowserProfile } from "@/types/rpc/BrowserProfile";

export function NewProfileModal(props: { onCreated: (profile: BrowserProfile) => void; close: () => void }): JSX.Element {
    const [name, setName] = createSignal("");
    // The next colour in turn, as srv would pick, so most people never choose.
    const [color, setColor] = createSignal(nextProfileColor());
    const [error, setError] = createSignal<string | null>(null);
    const [busy, setBusy] = createSignal(false);

    const create = async () => {
        if (!name().trim() || busy()) return;
        setBusy(true);
        setError(null);
        try {
            const profile = await createBrowserProfile(name().trim(), color());
            props.close();
            props.onCreated(profile);
        } catch (e) {
            setError(e instanceof Error ? e.message : String(e));
        } finally {
            setBusy(false);
        }
    };

    return (
        <Modal open={true} scope="pane" onClose={props.close} size="sm" ariaLabel="New browser profile">
            <ModalHeader title="New profile" />
            <ModalBody>
                <div class="browser-new-profile">
                    <Field label="Name" description="A profile keeps its own sign-ins, cookies and site data.">
                        <TextInput
                            value={name()}
                            autofocus
                            maxLength={40}
                            placeholder="Work"
                            onInput={(e) => setName(e.currentTarget.value)}
                            onKeyDown={(e) => {
                                if (e.key === "Enter") {
                                    e.preventDefault();
                                    void create();
                                }
                            }}
                        />
                    </Field>
                    <Field label="Colour">
                        <PaletteSwatches label="Colour" value={color()} onSelect={(slot) => slot && setColor(slot.hex)} />
                    </Field>
                    <Show when={error()}>
                        <div class="browser-new-profile-error" role="alert">
                            {error()}
                        </div>
                    </Show>
                </div>
            </ModalBody>
            <ModalFooter>
                <Button onClick={props.close}>Cancel</Button>
                <Button tone="accent" busy={busy()} disabled={!name().trim()} onClick={() => void create()}>
                    Create
                </Button>
            </ModalFooter>
        </Modal>
    );
}
