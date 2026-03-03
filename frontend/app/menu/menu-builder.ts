// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

/**
 * MenuBuilder - Composable context menu builder with inheritance support
 *
 * Usage:
 *   const menu = new MenuBuilder()
 *     .add({ label: "Action", click: () => {} })
 *     .separator()
 *     .merge(parentMenu);
 *
 *   menu.show(event);
 */

export class MenuBuilder {
    private items: ContextMenuItem[] = [];

    /**
     * Add a single menu item
     */
    add(item: ContextMenuItem): this {
        this.items.push(item);
        return this;
    }

    /**
     * Add multiple menu items
     */
    addAll(items: ContextMenuItem[]): this {
        this.items.push(...items);
        return this;
    }

    /**
     * Add a separator
     */
    separator(): this {
        this.items.push({ type: "separator" });
        return this;
    }

    /**
     * Add a submenu
     */
    submenu(label: string, builder: MenuBuilder | ContextMenuItem[]): this {
        const submenuItems = Array.isArray(builder) ? builder : builder.build();
        this.items.push({
            label,
            submenu: submenuItems,
        });
        return this;
    }

    /**
     * Merge another menu builder's items
     * @param other - Another MenuBuilder or array of items
     * @param position - Where to insert: 'before' or 'after' current items
     */
    merge(other: MenuBuilder | ContextMenuItem[], position: "before" | "after" = "after"): this {
        const otherItems = Array.isArray(other) ? other : other.build();

        if (position === "before") {
            this.items = [...otherItems, ...this.items];
        } else {
            this.items = [...this.items, ...otherItems];
        }

        return this;
    }

    /**
     * Merge parent menu items (convenience method for merge with 'before')
     */
    mergeParent(parent: MenuBuilder | ContextMenuItem[]): this {
        return this.merge(parent, "before");
    }

    /**
     * Add a section with label separator
     */
    section(label: string): this {
        this.items.push({ label, type: "separator" });
        return this;
    }

    /**
     * Conditionally add an item
     */
    addIf(condition: boolean, item: ContextMenuItem | (() => ContextMenuItem)): this {
        if (condition) {
            const menuItem = typeof item === "function" ? item() : item;
            this.items.push(menuItem);
        }
        return this;
    }

    /**
     * Build the final menu items array
     */
    build(): ContextMenuItem[] {
        return this.items;
    }


    /**
     * Get the number of items (excluding separators)
     */
    get length(): number {
        return this.items.filter((item) => item.type !== "separator").length;
    }

    /**
     * Check if menu is empty
     */
    get isEmpty(): boolean {
        return this.length === 0;
    }

    /**
     * Clear all items
     */
    clear(): this {
        this.items = [];
        return this;
    }
}
