# Hardware Constraint Packs (HCP)

One file says what a part requires, where the documents say so, and how binding each statement is. People and AI models read it as a
report; deterministic programs compile it and check a design against it; fastroute gets the part of it that a router can use. There is
no second copy of the rules: the report, the validator and the router input all come from the same pack.

```text
official manuals / design guides / open boards
              |  (download, digest, cache, keyword search, evidence check)
        hcp-manager  ........  documents, versions, derived text (rebuildable)
              |  a person or an AI reads the original pages and writes
   Hardware Constraint Pack (YAML)   <- the single source of facts, with evidence
              |  fastroute constraints compile (deterministic)
   Constraint IR (versioned JSON, evidence kept, per-rule "what can be executed")
        +-----+------------------+------------------+
        |                        |                  |
   readable report         validator            fastroute export
   (zh / en)          (offline, no AI)     (--pairs, --tune, .kicad_dru)
                             |
                 edit drafts for a fixed base board:
                 check-edit (whitelist + the same rules)
```

## Who may do what

| | Does | Never |
|---|---|---|
| **hcp-manager** (`apps/hcp-manager`, [HCP_MANAGER.md](HCP_MANAGER.md)) | inventory of documents, download and cache by digest, source priority, the embedded index (text blocks, vectors), keyword / vector / hybrid search filtered by device and version *before* ranking, evidence verification, pack store (create, list, show, import, export) | judge what a document means; produce rules; send customer text to a remote service unless explicitly authorised |
| **the AI / the Skill** (`skills/hardware-constraint-design`) | choose documents, read the pages, write rules with quotations, draft edits | state a number from memory; widen what it may edit; decide that a check passed |
| **`fastroute constraints`** (`crates/fr-constraint`) | schema check, compile, validate, report, export, edit-request, check-edit | use a model, the network or a vector index |
| **a person** | confirms the pack (review status, name, date), decides unknown and conflicting items | |

The facts are the pack and the original documents. The index is derived and can be deleted; a search hit is a lead, not an authority,
and nothing in a pack, an IR or a verdict depends on a search result.

## Where things are

| Path | What |
|---|---|
| `skills/hardware-constraint-design/` | the Skill: `SKILL.md`, `references/` (format, research, extraction, boot/reset, high-speed, mechanical, editable base, integration), `schemas/`, `scripts/hcp_cli.py`, `assets/` (example pack, inventory, edit draft, rule templates). `skills/hardware-constraint-design.skill` is its package |
| `crates/fr-constraint/` | the library: schema interpreter, pack checks, compiler, validator, KiCad reader, fastroute export, edit boundary, reports |
| `crates/fastroute/src/constraints_cmd.rs` | `fastroute constraints ...` |
| `apps/hcp-manager/` | the independent application (command line plus the library `hcp_manager`); its description: [HCP_MANAGER.md](HCP_MANAGER.md) |
| `hardware-packs/espressif/esp32-s3-wroom-1-n8r8/` | the example pack written from locked official documents, with its acceptance record |
| `docs/CLI.md` | the command reference (`fastroute constraints`) |
| `docs/hcp-pack-release.md`, `hardware-packs/schemas/`, `hardware-packs/tools/` | packs that ship with the software: layout `vendor/family/device/variant/version`, manifest, catalogue, release directory, offline install |

## Commands

`fastroute constraints` (offline, deterministic; exit codes 0 pass, 5 passed but something is unchecked / needs review, 1 violation or
rejection, 3 invalid input, 4 output refused, 2 usage):

| Command | |
|---|---|
| `schema [--ir\|--snapshot\|--edit-draft\|--edit-request]` | the published JSON Schemas |
| `resolve PACK --upstream FILE... -o FLAT` | a pack that inherits (`hcp_version` 1.1: module from chip, board from module or chip) merged with its pinned upstream packs |
| `check PACK` | structure, references, evidence, units, whitelist consistency |
| `compile PACK -o IR` | the Constraint IR (same bytes every time) |
| `report PACK -o FILE --lang zh\|en` | what cannot change / must stay / may change / evidence / gaps |
| `validate (--ir\|--pack) (--design\|--kicad-pcb)` | required blocks, reserved and strapping pins, fixed nets, reset and boot, keep-outs, mechanical items, the high-speed limits it can check |
| `extract-kicad BOARD` | a design snapshot from a `.kicad_pcb`, read-only |
| `export-fastroute --ir ... -o DIR` | what the router can execute, and a list of what it cannot |
| `edit-request`, `check-edit` | a fixed base board that an AI may edit only inside a whitelist |

`hcp-manager` (exit codes as above): `init`, `info` (the interface contract, `interface: "1"`), `sources add|import|list|resolve|sync|check`,
`index build|rebuild|status|prune`, `search` (`--device`, `--revision`, `--version`, `--source-level`, `--mode keyword|vector|hybrid`), `chunk show`,
`embedding show|test`, `evidence verify`, `packs list|show|create|import|export|resolve`, `packs builtin list|show|verify|install`, `release seal|index|build`.
The full list and the contract: [HCP_MANAGER.md](HCP_MANAGER.md); shipping packs with a release and installing them offline: [hcp-pack-release.md](hcp-pack-release.md).

`skills/hardware-constraint-design/scripts/hcp_cli.py`: `doctor`, `research ...` (to hcp-manager), `extract`, `check`, `compile`,
`report`, `validate`, `export`, `edit-request`, `check-edit`, `pipeline` (one run, one status file with the points a person must confirm).
It finds the two programs, negotiates their versions and turns a missing program, an empty inventory or a missing index into the next step.

## Formats (all versioned, all with a published schema)

| Format | Schema | Written by |
|---|---|---|
| pack, `hcp_version: "1.0"` | `schemas/hcp-v1.schema.json` | people and AI |
| Constraint IR, `ir_version` | `schemas/constraint-ir-v1.schema.json` | `compile` (integrity hash: an edited IR is refused) |
| design snapshot, `snapshot_version` | `schemas/design-snapshot-v1.schema.json` | `extract-kicad`, any tool |
| document inventory | `schemas/sources.schema.json` | hcp-manager |
| edit request / draft | `schemas/edit-request-v1.schema.json`, `schemas/edit-draft-v1.schema.json` | `edit-request` / an AI |

Reference: `skills/hardware-constraint-design/references/hcp-format.md` (pack, evidence, units, diagnostic codes) and `editable-base.md`
(the base board, the whitelist, the draft and its finding codes).

## What the checks guarantee, and what they do not

* The same pack, program version and design give the same bytes. No clock, network, model or index is read.
* `unknown` and `conflict` rules never run and never pass: they are listed with what is missing. A result with any of them, or with a rule the
  validator cannot check, is **exit 5**, not a clean pass.
* `fixed` and `required` statements about the chip need an official, explicit quotation; what a reference board does is `observed` and can
  only support board-level facts and configurable rules.
* Evidence from a document that does not apply to the device (a similar model) is refused (`source_not_applicable`).
* The validator is **not** KiCad's DRC and not an electrical review; the two results are separate. It has no electrical or impedance model.
* The router can use differential-pair spacing and skew, length-matching groups, clearance and track width (through `.kicad_dru`); keep-outs,
  impedance, maximum length and layer limits are checked by the validator, not handed to the router, and `export-fastroute` says so.

## Acceptance of the EPIC (issue #8)

| Criterion | Status | Implemented by / verified by |
|---|---|---|
| A person can read what cannot change, what must stay, what may change and where the evidence is | done | `constraints report` (zh / en), `reports/pack-report.zh.md`; tests in `fr-constraint/tests/extraction.rs` and `esp32s3_pack.rs` |
| An AI can generate modifications only inside the authorised regions | done | `edit-request` / `check-edit` (#17); `fr-constraint/tests/edit.rs`, `constraints_cli.rs`, `esp32s3_pack.rs` (rejections at every boundary, a changed base needs review, a draft is never applied to the project) |
| The validator checks, offline, required blocks, reserved pins, strapping conflicts, mechanical keep-outs and the supported high-speed rules | done | `fr-constraint/tests/validate.rs`, `special.rs`; ten mistaken ESP32-S3 boards fail at the expected rule |
| fastroute consumes the compiled constraints that apply; unsupported rules are reported | done | `export-fastroute` (pairs, tune, `.kicad_dru`) and its report; the pairs file is accepted by the router (`constraints_cli.rs`) |
| Any key rule traces to the pack and the original | done | evidence is kept in the IR and every finding; `hcp-manager evidence verify` (document, version, digest, words on the page): 84 of 84 for the ESP32-S3 pack |
| Documents and packs are managed without the Skill; the index (blocks and vectors) can be deleted and rebuilt (#18) | done | `apps/hcp-manager` (`tests/manager.rs`, `tests/vector.rs`); `esp32s3_documents.rs` (delete and rebuild: byte-identical answers, with vectors too); the Skill keeps no cache, download routine or index of its own |
| Vector index and hybrid retrieval, kept apart from the deterministic path (#19): filter by device / version / source level before ranking; a changed source invalidates the old index; different devices and versions never mix; a hit goes back to document, page and section; similarity is never evidence | done, with a lexical default | `tests/vector.rs` (invalidation, delete and rebuild, version isolation, similar-model false recall, offline degradation, remote refused without authorisation), `esp32s3_documents.rs` (84 quotations, 12 paraphrases, isolation cases), `esp32s3_pack.rs` (a recalled block changes no verdict); `fr-constraint` and `fastroute` do not link the manager or the database (a test reads `Cargo.lock`) |
| Without a vector service or a network, search degrades to keywords; the compile and the validation still run | done | `--mode hybrid` says it fell back (`degraded`); the offline test runs with an empty PATH and a dead proxy |
| Example pack, schema checks, negative tests and the `.skill` package | done | `hardware-packs/espressif/esp32-s3-wroom-1-n8r8/` and its `acceptance.md`; `skill_cli.rs` keeps `skills/hardware-constraint-design.skill` equal to the directory; the package passes skill-creator's validation |

### Not part of this work

* **The default vectors are lexical, and no semantic model was bundled or tested.** The built-in `hashing` provider helps with spelling variants
  and parts of words; on the ESP32-S3 documents it does **not** find more than the keywords (84 quotations, first hit / first three: keyword
  67 / 84, vector 46 / 62, hybrid 66 / 84; 12 questions in other words, first three: keyword 9, vector 8, hybrid 8; a small sample). A real
  embedding model can be plugged in (`command` or `http` provider), but none was run here. The remote path (`http` to a non-local host) was
  exercised only against a mock server, never against a real service.
* **No local HTTP API, no approximate-nearest-neighbour store.** hcp-manager is a command line plus a library; the vector store is a flat scan
  over the filtered candidates behind a `VectorIndex` trait, which is enough for the size of a device workspace and has not been measured on
  a large corpus.
* **Skill triggering is only partly measured.** With skill-creator's trigger evaluation (ten prompts, five that should trigger and five near
  misses, one or two runs each) the near misses never triggered and the real hardware prompts triggered in about 20 to 30 % of the runs. Rewording
  the description did not change that in a measurable way on this sample; it is not a statement that the Skill triggers reliably.
* Things a person has to confirm are listed in each pack's `acceptance.md` (for the ESP32-S3 pack: the antenna keep-out read from a figure,
  the review of the whole draft pack, the strapping policy, the unversioned guideline pages).
