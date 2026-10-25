# Built-in job templates

Each folder here is a job template that ships with AgentMux, published as
`agentmux.<name>` (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §5.4, §5.5).
srv compiles them in: add every new folder, with each of its files, to
`BUILTINS` in `crates/srv/src/backend/job_templates/mod.rs` (a test fails
otherwise). The operator still approves each one once before a job can use it.

A template folder holds:

- `template.json`: `templateVersion`, `id`, `name`, `version`, `kind` (`job` or
  `cloud`), `description`, `params` (a JSON Schema object: strings, with `enum`
  or `format: "credential"` for a Vault entry, numbers, booleans and arrays of
  strings), the default `trigger`, `action` and `policy`, and the declared
  `outputs` and `secrets`;
- `brief.md`: the agent's brief, with `{{param}}` placeholders;
- `README.md`, optional.

Built-in templates aren't signed. A template from anyone else is signed with
`npx -p @agentmuxai/widget-sdk agentmux-widget sign <folder> --key <key-file>`.

Generic templates only: a project's own templates belong in its private
catalog.
