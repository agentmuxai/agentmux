# The `.vault` file

**Status:** living — the format of AgentMux's `.vault` export, version 1 (docs/specs/SPEC_VAULT_PANE_AGENTS_USE_SECRETS_UNSEEN_2026_10_19.md §8).

AgentMux's Vault exports to a `.vault` file: an [age](https://age-encryption.org/v1)
file encrypted to a passphrase, whose contents are the JSON below. You don't need
AgentMux to open your own backup:

```sh
age -d backup.vault > vault.json
```

`age` asks for the passphrase. The JSON then holds every secret in the clear, so
handle it like the secrets it is, and delete it when you're done.

## Encryption

- age format version 1, one `scrypt` recipient (a passphrase). Nothing else is
  added: no extra header, no AgentMux-specific wrapper.
- AgentMux asks for a passphrase of at least 8 characters and offers to suggest
  eight random words. Choose a long one: the file can be copied and guessed
  against offline, and the passphrase is all that protects it.
- When it opens a file, AgentMux refuses scrypt work factors above 22 (about
  4 million iterations, a few seconds of work), so a crafted file can't make it hang.
- A file larger than 64 MiB, or with more than 100,000 entries, isn't imported.

## The JSON, format version 1

```json
{
  "format": "agentmux-vault",
  "version": 1,
  "exported_at": 1792500000000,
  "app_version": "0.59.35",
  "entries": [
    {
      "id": "ssn-alex",
      "kind": "ssn",
      "label": "Alex",
      "fields": { "owner": "Alex Doe" },
      "tags": ["tax"],
      "parts": { "value": "123-45-6789" },
      "created_at": 1792400000000,
      "updated_at": 1792400000000
    }
  ]
}
```

| Field | Meaning |
|---|---|
| `format` | Always `"agentmux-vault"`. |
| `version` | `1`. A file with a higher version is refused with a message to update AgentMux. |
| `exported_at` | Unix milliseconds. |
| `app_version` | The AgentMux that wrote it. |
| `entries[].id` | The entry's id, which agents use to name it. Kept on import when it is free, else a new one is made. |
| `entries[].kind` | One of the kinds below. An unknown kind is skipped and listed in the import preview. |
| `entries[].label` | What the person calls it. |
| `entries[].fields` | Plain details (`owner`, `site`, `username`, …), string to string. |
| `entries[].tags` | A list of strings. |
| `entries[].parts` | The secret parts, string to string. Every part a kind has is listed below. |
| `entries[].created_at`, `updated_at` | Unix milliseconds. |

Grants, Activity and the secrets AgentMux keeps for itself (account keys, the MuxBus
sign-in) aren't exported: they belong to one install.

## Kinds

| `kind` | `parts` | `fields` |
|---|---|---|
| `ssn`, `itin`, `tin`, `ein` | `value` | `owner`, `notes` |
| `date-of-birth` | `value` | `owner`, `notes` |
| `passport`, `drivers-licence`, `other-id` | `value` | `owner`, `issuer`, `expires`, `notes` |
| `card` | `number`, `expiry`, `cvc`, `name` | `owner`, `notes` |
| `bank-account` | `account`, `routing` | `owner`, `bank`, `notes` |
| `api-key` | `value` | `service`, `env_name`, `expires`, `notes` |
| `login` | `password`, `totp` | `site`, `username`, `notes` |
| `note` | `text` | none |

The first part of a kind (`value`, `number`, `account`, `password`, `text`) is its main
value and is required, except for a linked entry (below). `totp` holds a one-time-code seed,
stored and exported but not yet used to fill anything. Unknown parts or fields make the entry
be skipped, with the reason shown.

## Linked entries

A linked entry records where a key lives and holds no value
(docs/specs/SPEC_VAULT_LINKED_ENTRIES_FLEET_VAULT_AND_BATCH_IMPORT_2026_11_15.md §2). In a
file it is an entry of an ordinary kind whose `fields` say `"link": "only"` and whose `parts`
are empty:

```json
{
  "id": "api-key-invoice-provider-key",
  "kind": "api-key",
  "label": "Invoice provider key",
  "fields": {
    "link": "only",
    "source_kind": "aws-secret",
    "source_location": "prod/invoice-sync",
    "source_key": "api.token",
    "source_region": "eu-west-1",
    "readers": "invoice-sync (lambda): at run time",
    "rotated": "2026-10-01"
  },
  "tags": [],
  "parts": {}
}
```

- `source_kind` and `source_location` are required: a link says where the key lives.
  `source_key`, `source_region` and `source_profile` narrow it. Any entry may carry these
  `source_*` fields, to say where a copy of its value is kept.
- `readers` (who or what reads the key, one line each), `rotated` (the date the key last
  changed, `YYYY-MM-DD`) and `copies` (other places it is kept, one `kind: location#key` line
  each) say more of it. `readers` and `copies` may be on an entry that holds its value too.
- A linked entry with a part, or with no place, is skipped on import, with the reason shown.
- When it was last checked isn't in the file: Activity belongs to one install.

The export dialog's **Linked entries only** writes a `.vault` file of the linked entries and
nothing else, so no value is in it. It is encrypted to a passphrase like any other, and is a
way to show on another computer which keys exist and where they live.

## What counts as a duplicate on import

An incoming entry duplicates one already in the Vault when:

- for a linked entry, the kind and the place match (`source_kind`, `source_location`,
  `source_key` and `source_region`);
- for a `login`, the site (compared without case or a trailing `/`) and the username match;
- for any other kind, the kind and the main value match.

You then choose, for the whole import: **skip** the duplicates, **replace** the entries
you have (they keep their ids), or **keep both**.

## Other services' exports

AgentMux also imports files other password managers write, which are plaintext: see the Vault spec's section 8.8 for the list. They are not Vault files and are not encrypted, so the pane offers to delete one once it is imported.
