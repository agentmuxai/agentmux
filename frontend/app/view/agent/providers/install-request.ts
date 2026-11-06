// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import type { ProviderDefinition } from "./types";

/**
 * The `npm_package` a `resolvecli` request carries. srv reads a non-empty
 * value as "install it if it is missing" and takes what to install from its own
 * registry, so for a CLI AgentMux downloads (`managedInstall`) the id stands in:
 * it is never used as a package name. Empty means a look, not an install.
 */
export function installPackageField(provider: Pick<ProviderDefinition, "id" | "npmPackage" | "managedInstall">): string {
    return provider.npmPackage || (provider.managedInstall ? provider.id : "");
}
