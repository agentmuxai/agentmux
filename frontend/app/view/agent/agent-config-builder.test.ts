// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { buildConfigFiles, deriveSlug, renderSkillMd, sanitizeTrigger, uniqueSkillSlug } from "./agent-config-builder";

function makeSkill(over: Partial<AgentSkill> = {}): AgentSkill {
    return {
        id: "skill-1",
        agent_id: "agent-1",
        name: "Deploy",
        trigger: "deploy",
        skill_type: "prompt",
        description: "Deploy the app",
        content: "Run: deploy all",
        created_at: 0,
        ...over,
    };
}

describe("deriveSlug", () => {
    it("lowercases and dash-joins", () => {
        expect(deriveSlug("Deploy Checklist")).toBe("deploy-checklist");
    });

    it("collapses runs of non-alphanumeric characters into a single dash", () => {
        // Regression: naive strip-only slugifiers (e.g. slugifyInstanceName)
        // would produce "deploychecklist" here (no dash), diverging from
        // the Rust derive_slug this must mirror exactly.
        expect(deriveSlug("Deploy!!!Checklist")).toBe("deploy-checklist");
    });

    it("falls back to 'agent' for names with no valid characters", () => {
        expect(deriveSlug("!!!")).toBe("agent");
    });

    it("trims to 64 characters", () => {
        const long = "a".repeat(100);
        expect(deriveSlug(long)).toHaveLength(64);
    });
});

describe("sanitizeTrigger", () => {
    it("rejects path traversal and separators", () => {
        // reagent P1, PR #2322
        expect(sanitizeTrigger("../../../../.ssh/authorized_keys")).toBeNull();
        expect(sanitizeTrigger("../evil")).toBeNull();
        expect(sanitizeTrigger("sub/evil")).toBeNull();
        expect(sanitizeTrigger("sub\\evil")).toBeNull();
        expect(sanitizeTrigger("..")).toBeNull();
        expect(sanitizeTrigger(".")).toBeNull();
        expect(sanitizeTrigger("")).toBeNull();
    });

    it("allows an ordinary trigger", () => {
        expect(sanitizeTrigger("deploy")).toBe("deploy");
    });
});

describe("buildConfigFiles — trigger sanitization", () => {
    it("skips a prompt-format skill with a path-traversal trigger", () => {
        // reagent P1, PR #2322: must not materialize a command file outside
        // .claude/commands/, not even under a mangled name -- skip outright.
        const files = buildConfigFiles(
            {},
            [makeSkill({ trigger: "../../../../.ssh/authorized_keys", content: "evil" })],
        );
        expect(files.every((f) => !f.path.includes(".."))).toBe(true);
        expect(files.some((f) => f.path.startsWith(".claude/commands/"))).toBe(false);
    });
});

describe("renderSkillMd", () => {
    it("wraps content in YAML frontmatter with slug (as name) + description", () => {
        // First arg is the SLUG (matching the parent directory per the Agent
        // Skills spec), not the raw display name -- reagent P1 on #2322.
        const md = renderSkillMd("deploy-checklist", "Runs the checklist", "1. Test\n2. Deploy");
        expect(md).toBe(
            '---\nname: "deploy-checklist"\ndescription: "Runs the checklist"\n---\n\n1. Test\n2. Deploy',
        );
    });

    it("escapes YAML/JSON special characters via JSON.stringify", () => {
        const md = renderSkillMd("weird-name", "Has a colon: and \"quotes\"", "body");
        const frontmatter = md.split("\n---\n\n")[0];
        // Must round-trip as exactly two lines (name + description) -- no
        // characters leaked out of the quoted scalar onto a new line.
        expect(frontmatter.split("\n")).toHaveLength(3); // "---", name line, description line
    });

    it("falls back to a placeholder for an empty description", () => {
        const md = renderSkillMd("deploy-checklist", "", "body");
        expect(md).toContain('description: "No description provided."');
    });

    it("falls back to a placeholder for a whitespace-only description", () => {
        const md = renderSkillMd("deploy-checklist", "   ", "body");
        expect(md).toContain('description: "No description provided."');
    });

    it("truncates a description longer than 1024 characters", () => {
        const md = renderSkillMd("deploy-checklist", "x".repeat(2000), "body");
        const descLine = md.split("\n").find((l) => l.startsWith("description: "))!;
        const inner = descLine.slice("description: ".length + 1, -1); // strip quotes
        expect(inner.length).toBeLessThanOrEqual(1024);
    });
});

describe("uniqueSkillSlug", () => {
    it("appends -2, -3 for colliding slugs in call order", () => {
        const used = new Set<string>();
        expect(uniqueSkillSlug("Deploy Checklist", used)).toBe("deploy-checklist");
        expect(uniqueSkillSlug("Deploy!!!Checklist", used)).toBe("deploy-checklist-2");
        expect(uniqueSkillSlug("Deploy   Checklist", used)).toBe("deploy-checklist-3");
    });

    it("does not collide with a pre-existing -2 suffix", () => {
        const used = new Set<string>(["deploy-checklist", "deploy-checklist-2"]);
        expect(uniqueSkillSlug("Deploy!!Checklist", used)).toBe("deploy-checklist-3");
    });

    it("replaces underscores with hyphens (Agent Skills name grammar has no underscores)", () => {
        // Codex P1, PR #2322: deriveSlug (shared with agent role-slugs) keeps
        // underscores, which is spec-invalid for an Agent Skills `name`.
        const used = new Set<string>();
        expect(uniqueSkillSlug("code_review", used)).toBe("code-review");
    });

    it("keeps the suffixed slug within the 64-character spec max", () => {
        // Codex P2, PR #2322: a 64-char base plus "-2" was previously 66 chars.
        const used = new Set<string>();
        const long = "a".repeat(100);
        const first = uniqueSkillSlug(long, used);
        expect(first).toHaveLength(64);
        const second = uniqueSkillSlug(long, used);
        expect(second.length).toBeLessThanOrEqual(64);
        expect(second.endsWith("-2")).toBe(true);
    });
});

describe("buildConfigFiles — skill materialization", () => {
    it("writes prompt-format skills as .claude/commands/<trigger>.md (default)", () => {
        const files = buildConfigFiles({}, [makeSkill()]);
        expect(files.find((f) => f.path === ".claude/commands/deploy.md")).toBeDefined();
        expect(files.some((f) => f.path.startsWith(".claude/skills/"))).toBe(false);
    });

    it("writes agent-skill-format skills as .claude/skills/<slug>/SKILL.md", () => {
        const files = buildConfigFiles({}, [
            makeSkill({ skill_type: "agent-skill", trigger: "", name: "Deploy Checklist" }),
        ]);
        const skillFile = files.find((f) => f.path === ".claude/skills/deploy-checklist/SKILL.md");
        expect(skillFile).toBeDefined();
        // name is the slug, not the raw display name (reagent P1 on #2322).
        expect(skillFile!.content).toContain('name: "deploy-checklist"');
        expect(skillFile!.content).not.toContain('name: "Deploy Checklist"');
        expect(files.some((f) => f.path.startsWith(".claude/commands/"))).toBe(false);
    });

    it("dedupes SKILL.md paths for names that collide after slugification", () => {
        // reagent P1 (PR #2322): distinct skills must not silently overwrite
        // each other's SKILL.md when their names slugify to the same value.
        const files = buildConfigFiles({}, [
            makeSkill({ skill_type: "agent-skill", trigger: "", name: "Deploy Checklist", content: "one" }),
            makeSkill({ skill_type: "agent-skill", trigger: "", name: "Deploy!!!Checklist", content: "two" }),
        ]);
        const skillPaths = files.filter((f) => f.path.startsWith(".claude/skills/")).map((f) => f.path);
        expect(new Set(skillPaths).size).toBe(2);
        expect(skillPaths).toContain(".claude/skills/deploy-checklist/SKILL.md");
        expect(skillPaths).toContain(".claude/skills/deploy-checklist-2/SKILL.md");
    });

    it("skips skills with empty content regardless of format", () => {
        const files = buildConfigFiles({}, [makeSkill({ content: "" })]);
        expect(files.some((f) => f.path.includes("deploy"))).toBe(false);
    });
});
