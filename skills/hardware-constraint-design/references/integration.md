# Tools, versions and failure branches

The Skill calls two programs through their command lines. It does not import their code and keeps no state
of its own; `scripts/hcp_cli.py` is the only glue.

## Finding the programs

Order: `--fastroute` / `--manager`, then `FASTROUTE_BIN` / `HCP_MANAGER_BIN`, then `PATH`, then
`target/release` and `target/debug` of the repository that contains the Skill. `hcp_cli.py doctor` prints
what it found:

```json
{"fastroute": {"path": "...", "version": "fastroute 0.1.14", "ok": true},
 "hcp_manager": {"interface": "1", "capabilities": {"keyword_search": true, "vector_index": true, "hybrid_search": true, "pdf_text": true, "network_fetch": true}, "ok": true},
 "workflows": {"local_pack_workflow": true, "document_research": true, "vector_search": true}}
```

Exit: 0 both usable, 5 hcp-manager missing or incompatible (the local workflow still works), 6 fastroute
missing or without the `constraints` commands the script needs.

## Version negotiation

- **hcp-manager**: `hcp-manager info --json` returns `interface` (a string). This Skill speaks `"1"`. A different
  value is refused with the reason; a minor version of the application (`version`) does not matter. `capabilities`
  says what exists: `keyword_search`, `vector_index`, `hybrid_search`, `embedded_store`, `embedding_providers`,
  `semantic_model_bundled` (false: the default provider is lexical), `remote_embedding_by_default` (false), `pdf_text`,
  `network_fetch`. Changes of the interface are additive within `"1"`: new commands and new JSON fields, never a removed or
  renamed one. Use the vectors only when the capability is `true` and `index status` says `ready`, and treat the result as a
  lead either way.
- **fastroute**: the `constraints` subcommands that the script uses must be listed by `constraints --help`
  (`check`, `compile`, `report`, `validate`, `extract-kicad`, `export-fastroute`, `edit-request`, `check-edit`).
  The formats carry their own versions (`pack_version`, `ir_version`, `snapshot_version`, `draft_version`);
  a program refuses a document of a major version it does not know (exit 3).

## What each program is asked for

| Need | Command | Program |
|---|---|---|
| documents for a device, best first | `research resolve --device PART` | hcp-manager |
| fetch and pin digests | `research sync [DOC...]` (`--accept` only on a person's say) | hcp-manager |
| derived index (blocks and vectors), rebuildable | `research index build` / `rebuild` / `status` | hcp-manager |
| back to the original of a hit | `research chunk show CHUNK_ID` | hcp-manager |
| embedding provider, what would leave the machine | `research embedding show` / `test` | hcp-manager |
| find passages | `research search WORDS --device PART [--version V] [--source-level official] [--mode keyword\|vector\|hybrid]` | hcp-manager |
| quotations are in the documents | `research verify PACK` | hcp-manager |
| make, list, show, export, import a pack | `research packs create\|list\|show\|export\|import` | hcp-manager |
| structure, references, evidence form | `check PACK` | fastroute |
| compile, report | `compile`, `report` | fastroute |
| validate a design, export to the router | `validate`, `export` | fastroute |
| fixed base board | `edit-request`, `check-edit` | fastroute |

The knowledge pack, its sources and the base design are referred to by digest. A request, a draft and a report
each name the exact pack (`sha256`) and base binding they are for; nothing means "latest".

## Exit codes

`0` ok - `1` a violation or failure - `2` usage - `3` invalid input (a pack with errors, a draft that is not a draft,
a missing file) - `4` output refused (it would overwrite an input) - `5` ok, but something needs a person or was not
checked - `6` (script only) a required program is missing.

Exit 5 is a result to report, not an error: say what needs the person.

## The status file

`pipeline PACK --out DIR` writes `pipeline-status.json`:

```json
{"format": "hcp-skill-pipeline", "exit": 5,
 "steps": [{"step": "check", "status": "ok"}, {"step": "evidence", "status": "skipped", "note": "..."}],
 "needs_human": ["1 rule(s) are unknown ...", "the pack is a draft ..."]}
```

Step statuses: `ok`, `needs_human`, `skipped` (a program or an input was not there), `failed`. `needs_human` lists
the confirmation points: unknown and conflicting rules, a draft pack, evidence that was not compared with the
documents, rules that were not checked. Present them to the person; do not close them yourself.

## Offline and local work

Everything after `sync` works without a network. `check`, `compile`, `report`, `validate`, `export`, `check-edit`
need neither the network, nor an AI model, nor a vector index, nor hcp-manager: the same pack, design and
program give the same answer. Without hcp-manager, work from documents the user gives you (a file, a page), cite
them in the pack with their version, and mark the evidence as not verified until `research verify` can run.
