// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { buildConfigFiles, deriveSlug, renderSkillMd, uniqueSkillSlug } from "./agent-config-builder";

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

describe("renderSkillMd", () => {
    it("wraps content in YAML frontmatter with name + description", () => {
        const md = renderSkillMd("Deploy Checklist", "Runs the checklist", "1. Test\n2. Deploy");
        expect(md).toBe(
            '---\nname: "Deploy Checklist"\ndescription: "Runs the checklist"\n---\n\n1. Test\n2. Deploy',
        );
    });

    it("escapes YAML/JSON special characters via JSON.stringify", () => {
        const md = renderSkillMd('Weird: "Name"', "Has a colon: and \"quotes\"", "body");
        const frontmatter = md.split("\n---\n\n")[0];
        // Must round-trip as exactly two lines (name + description) -- no
        // characters leaked out of the quoted scalar onto a new line.
        expect(frontmatter.split("\n")).toHaveLength(3); // "---", name line, description line
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
        expect(skillFile!.content).toContain('name: "Deploy Checklist"');
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
