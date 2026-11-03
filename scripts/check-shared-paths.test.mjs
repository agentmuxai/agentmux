// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { check } from "./check-shared-paths.mjs";

const entry = (over = {}) => ({
  name: "shared stores",
  paths: ["store.db"],
  why: "follows the person",
  gate: "user_version",
  lock: "SQLite WAL",
  users: ["crates/srv/src/registry/paths.rs"],
  ...over,
});

describe("check-shared-paths", () => {
  it("passes when every user is named by an entry", () => {
    const { errors } = check({ entries: [entry()] }, ["crates/srv/src/registry/paths.rs"]);
    expect(errors).toEqual([]);
  });

  it("fails for a file that reaches the shared folder and no entry names", () => {
    const { errors } = check({ entries: [entry()] }, ["crates/srv/src/registry/paths.rs", "crates/srv/src/backend/new_feature.rs"]);
    expect(errors).toHaveLength(1);
    expect(errors[0]).toContain("new_feature.rs");
  });

  it("fails for an entry that names a file that no longer reaches the shared folder", () => {
    const { errors } = check({ entries: [entry({ users: ["crates/srv/src/gone.rs"] })] }, []);
    expect(errors.join("\n")).toContain("gone.rs");
  });

  it("fails for an entry with no gate, lock or why", () => {
    for (const field of ["gate", "lock", "why"]) {
      const { errors } = check({ entries: [entry({ [field]: "  " })] }, ["crates/srv/src/registry/paths.rs"]);
      expect(errors.join("\n")).toContain(`has no ${field}`);
    }
  });

  it("matches a glob for a whole folder of users", () => {
    const e = entry({ users: ["crates/srv/src/migrations/*.rs"] });
    const { errors } = check({ entries: [e] }, ["crates/srv/src/migrations/runner.rs", "crates/srv/src/migrations/m0011_x.rs"]);
    expect(errors).toEqual([]);
  });
});
