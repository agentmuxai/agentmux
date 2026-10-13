# ts-rs drops serde attributes it can't parse

**Status:** active — the upgrade, the binding review, the rename test and the CI gate in review. The problem and options are AgentX's, from #4802; this is the plan for carrying out its recommended option D. Written 2026-10-13.
**Related:** `SPEC_RPC_BINDINGS_CODEGEN_2026_09_07.md` §3.2 assumes ts-rs's `serde-compat` feature honours `skip_serializing_if`. With ts-rs 10.1 it does not.

## The problem (#4802)

We pin `ts-rs = { version = "10", features = ["serde-compat", "serde-json-impl"] }` in `crates/srv/Cargo.toml`, which resolves to 10.1.0.

- **The parse fails whole.** ts-rs 10.1 parses each `#[serde(...)]` attribute as a unit. One item it doesn't know, `skip_serializing_if`, fails the parse, and then every item in that attribute is ignored. A `task package` build prints `failed to parse serde attribute` 218 times, with no file or line.
- **Absent fields come out required.** A field serde leaves out when empty is generated as `x: T | null` unless someone adds `#[ts(optional)]` by hand. 209 of the 219 such fields have it; about 10 don't (#4802 §3 lists them).
- **A rename in the same attribute is lost.** ts-rs then emits the Rust field name, and nothing fails. This caused the "My Agents empty" incident (`backend/storage/identities.rs:83`). Three hand `#[ts(rename)]`s guard against it today, and `check-rpc-codegen-hygiene` now demands one beside every `serde(rename)`.

## What newer ts-rs changes

From #4802 §4:
- **11.0:** `skip_serializing_if` together with `default` makes the field optional, which covers about 170 of the 218. It also adds `#[ts(optional_fields)]`.
- **12.0:** comma-separated serde attributes are parsed properly, so a `rename` beside another item survives.
- **12.0, breaking:** a `HashMap` whose key is not an enum is generated as `{ [key in K]: V }`.

## Plan

1. **Upgrade on a branch.** Set `ts-rs = { version = "12", … }` and fix whatever the derives stop accepting. Build, and count the warnings left.
2. **Regenerate the bindings** (`cargo test -p agentmux-srv export_bindings`) and the stubs and event map that read them (`gen-rpc-stubs`, `gen-mux-events`).
3. **Review the diff by kind**, and record each in the PR:
   - **Fields turning optional** (`x?: T`). Expected where serde skips the field. Each is checked against its frontend readers, since a reader may now need to handle `undefined`; `tsc` will point at most of them.
   - **`HashMap` shapes** (`{ [key in K]: V }`). Each frontend reader is checked with `tsc`.
   - **Anything else** is a regression, to be explained or fixed.
4. **Clear what still warns.**
   - A `skip_serializing_if` on an `Option` without `default` gets `default`, which is harmless there, or `#[ts(optional)]`.
   - Anything that can't be fixed goes on a short allow-list, with its reason.
5. **Prove renames survive.** A test asserts the generated key equals the serde wire name for the three known renames, alongside `flow_node_type_field_is_named_type_on_the_wire`. Then:
   - the hand `#[ts(rename)]`s that 12 makes redundant are removed;
   - `check-rpc-codegen-hygiene`'s rule demanding a `ts(rename)` beside each `serde(rename)` is dropped, since the parser now carries renames itself and the test pins it.
6. **Gate the warning.** A CI step fails the build when `failed to parse serde attribute` appears, apart from the allow-list. A test struct with `#[serde(rename = "x", skip_serializing_if = "Option::is_none")]` proves the build stays clean and the binding says `x?:`.
7. **Check everything:** the full srv suite, `tsc`, the full frontend suite, and every codegen gate.

## Open questions

These come from #4802 and are answered on the branch:
1. Does 12 still warn on `skip_serializing_if` without `default`? Step 4 handles either answer.
2. Does any frontend code index a changed `HashMap` type with a non-string key?
3. Should the hygiene rule for `ts(rename)` stay as a belt-and-braces check? The plan drops it once the rename test exists.

## Not in scope

- **Option A of #4802** (`no-serde-warnings`) on its own: it hides the one signal we have.
- **Typing more events:** `SPEC_TYPED_EVENTS_EVERY_PATH_2026_10_13.md`.
