// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Connectors' list (docs/specs/SPEC_CONNECTORS_ONE_LIST_2026_11_19.md): one card per connector, Connected first, then
// Available. The cards reflow with the pane's width: one column when thin, up to four when wide (§4.7). A card that is not
// connected has Connect, which opens the add section under its row of cards; a connected one opens its accounts there.
// It replaces the Accounts section's two lists, the add gallery and the Connected cards. Its section id stays `accounts`.

import { createUniqueId, Show, type Accessor, type JSX } from "solid-js";

import { Button, CardGrid, EmptyState, FilterInput, SectionHeader } from "@/app/element/ui";
import { t } from "@/app/i18n/i18n";
import { AddAccountSection } from "@/app/view/accounts/AddAccountSection";
import { OtherAgentsSection } from "@/app/view/accounts/OtherAgentsSection";
import { AccountForm } from "@/app/view/identity/identity-account-form";
import { AccountDeleteNotice, DeleteAccountConfirm } from "@/app/view/identity/identity-accounts-tab";
import { ConnectorCard } from "./connector-card";
import { ConnectorDetails } from "./connector-details";
import type { ConnectorEntry } from "./connector-entries";
import { useConnectorsState, type ConnectorsState } from "./connectors-state";
import "@/app/view/identity/identity-view.scss";

/** At most this many columns, each at most this wide, however wide the pane is (§4.7). */
const MAX_COLUMNS = 4;
const MAX_CARD_WIDTH = 380;
/** A card's own minimum: a column is never narrower, so the pane drops a column instead. */
const MIN_CARD_WIDTH = 270;

/** One group of cards (Connected or Available): its heading and its grid, which opens the details under a card's row. */
function Group(props: {
    s: ConnectorsState;
    title: string;
    detailId: string;
    items: Accessor<ConnectorEntry[]>;
    renderDetail: (e: ConnectorEntry) => JSX.Element;
}): JSX.Element {
    const s = props.s;
    return (
        <Show when={props.items().length > 0}>
            <SectionHeader variant="group" class="accounts-connected-heading" title={props.title} />
            <CardGrid
                class="connectors-grid"
                ariaLabel={props.title}
                items={props.items()}
                keyOf={(e) => e.key}
                minCardWidth={MIN_CARD_WIDTH}
                maxColumns={MAX_COLUMNS}
                maxCardWidth={MAX_CARD_WIDTH}
                selectedKey={s.openIn(props.items)}
                detailId={props.detailId}
                detailLabel={(e) => t("connectors.detailsLabel", { name: e.name })}
                focusDetails
                onClose={s.close}
                renderCard={(e) => (
                    <ConnectorCard
                        entry={e}
                        open={s.openKey() === e.key}
                        flash={s.flash().has(e.key)}
                        detailId={props.detailId}
                        onToggle={() => s.toggle(e)}
                        onReconnect={() => s.reconnect(e)}
                    />
                )}
                renderDetail={props.renderDetail}
            />
        </Show>
    );
}

export function ConnectorsList(): JSX.Element {
    const s = useConnectorsState();
    // Panel ids are per pane: two Connectors panes in one window must not share an id (aria-controls points at it).
    const uid = createUniqueId();
    return (
        <div class="identity-view accounts-manager">
            <div class="identity-body accounts-manager-body">
                <section class="ui-card-section ui-density-compact" aria-label={t("connectors.title")}>
                    <SectionHeader title={t("connectors.title")} count={s.connectedCount() > 0 ? s.connectedCount() : undefined}>
                        <FilterInput
                            class="ui-card-section-filter"
                            placeholder={t("connectors.filter")}
                            value={s.filter()}
                            onInput={s.setFilter}
                        />
                    </SectionHeader>

                    <AccountDeleteNotice model={s.model} />

                    <Group
                        s={s}
                        title={t("connectors.group.connected")}
                        detailId={`connectors-${uid}-connected`}
                        items={s.connected}
                        renderDetail={(e) => (
                            <ConnectorDetails
                                entry={() => s.entryOf(e.key) ?? e}
                                model={s.model}
                                muxbus={s.muxbus}
                                usedBy={s.usedBy}
                                adding={s.adding}
                                setAdding={s.setAdding}
                            />
                        )}
                    />
                    <Group
                        s={s}
                        title={t("connectors.group.available")}
                        detailId={`connectors-${uid}-available`}
                        items={s.available}
                        renderDetail={(e) => (
                            <AddAccountSection tile={e.tile} model={s.model} muxbus={s.muxbus} onClose={s.close} />
                        )}
                    />
                    <Show when={s.filter() && s.shownCount() === 0}>
                        <EmptyState>{t("connectors.noMatch", { query: s.filter() })}</EmptyState>
                    </Show>

                    <p class="connectors-foot">
                        {t("connectors.foot.prompt")}{" "}
                        <Button variant="link" onClick={s.openCustom}>
                            {t("connectors.foot.custom")}
                        </Button>
                    </p>
                    <OtherAgentsSection />
                </section>
            </div>

            <Show when={s.model.pendingDeleteAtom()} keyed>
                {(account) => <DeleteAccountConfirm model={s.model} account={account} />}
            </Show>

            {/* Editing an account is the pane's form; adding one opens in the add section. */}
            <Show when={s.model.formOpenAtom() && s.model.editingAccountAtom()}>
                <AccountForm model={s.model} />
            </Show>
        </div>
    );
}

ConnectorsList.displayName = "ConnectorsList";
