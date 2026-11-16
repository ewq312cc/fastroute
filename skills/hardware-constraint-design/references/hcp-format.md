# Hardware Constraint Pack (HCP) v1 — format

An HCP is one YAML (or JSON) file with the facts and constraints of a device, each with the
evidence it comes from. People read it (`fastroute constraints report`), AI models write it
(draft), and the deterministic tools compile it to a Constraint IR and check designs against
that. The schema is `schemas/hcp-v1.schema.json`; `assets/example-device.yaml` is a complete
example for a *fictional* device. `fastroute constraints check pack.yaml` says what is wrong and
where.

Contents: [What a pack holds](#what-a-pack-holds) · [Modifiability](#modifiability) ·
[Evidence](#evidence) · [Units](#units) · [Rules](#rules-and-their-parameters) ·
[Blocks, regions, base design](#blocks-regions-base-design) · [Ids and the link to hcp-manager](#ids-and-the-link-to-hcp-manager) ·
[Versions](#versions-and-compatibility) · [Diagnostic codes](#diagnostic-codes)

## What a pack holds

| Section | Holds |
|---|---|
| `pack` | id, title, pack version, language, creation date, who extracted it (`ai`, `human`, `mixed`) and the review state (`draft`, `reviewed`, `approved`; the last two name who and when) |
| `sources` | the documents the evidence quotes: `source_id`, `document_id`, title, **version**, `level`, URL, retrieval date, SHA-256, and the devices it **applies to** |
| `devices` | vendor, part number, revision, package and every **pin** (`number` is the key; name, class, functions) |
| `peripheral_blocks` | circuits the device needs or allows: decoupling, reset, boot button, programming interface, USB, ... with the components and the pins they connect to |
| `rules` | typed constraints (below), each with a `modifiability`, parameters, conditions (`applies_when`) and evidence |
| `regions` | board outline, mounting holes, connectors, module body, antenna keep-outs, editable areas — polygons or circles in millimetres |
| `base_design` | a fixed base board: what it is bound to (repository commit or file digests), what is immutable, and the **only** regions, blocks and pins an AI may use |

A pack holds **facts and evidence references only**. Vectors, similarity scores, retrieval ranks
and embeddings never appear in it (`vector_field`): they belong to hcp-manager's index, which can
be deleted and rebuilt, and nothing in compiling or validating reads them.

Keys that start with `x_` are free extensions; the compiler ignores them.

## Modifiability

Every rule, block and region says what may happen to it. The default for everything is
**read-only**; only `ai_editable` things on the `base_design` whitelist can be changed by an AI.

| Value | Meaning | Needs |
|---|---|---|
| `fixed` | must not change | evidence |
| `required` | must be present / kept | evidence |
| `configurable` | a choice with limits (the limits are the rule) | evidence |
| `derived` | follows from other rules, not stated by a document | `derived_from` (rule ids) and evidence with `basis: derived` |
| `optional` | allowed, not needed | evidence |
| `ai_editable` | an AI may change it, inside the base design's whitelist | evidence, and an entry in `base_design.editable_regions` |
| `unknown` | the documents do not say | `missing` (what is missing); numbers in `params` are `null`, **never a guess** |
| `conflict` | the documents disagree | `conflict.candidates`: two or more *different* values, each with its evidence |

Conflicts are never resolved by voting or by a "confidence": a conflict stays a conflict until a
person decides, and while it stands no executable rule is made from it.

## Evidence

```yaml
evidence:
  - source_id: SRC-DS            # must exist in sources and apply to the device of the rule
    chunk_id: "CHK-DS@3f2a9c1d:p12.3"   # optional: the text block in hcp-manager (stable, see below)
    locator: { page: 12, section: "6.2 Antenna keep-out", table: "...", figure: "..." }  # page or section
    quote: "Keep the area around the antenna free of copper on all layers."   # at most 400 characters
    basis: explicit              # explicit | derived | observed | pending
    extracted: "2026-11-16"
```

| `basis` | Means | May support |
|---|---|---|
| `explicit` | the document says it in these words | everything |
| `derived` | follows from several statements | `derived` rules |
| `observed` | what a reference board or example does | `configurable`, `optional`, `ai_editable` things and board-level facts (regions, `mechanical_fixed`) — **not** a `fixed`/`required` statement about the chip |
| `pending` | to be confirmed | nothing (a warning); use `unknown` instead |

A `fixed` or `required` rule about the chip itself (`reserved_pin`, `nc_pin`, `fixed_net`,
`exclusive_functions`, `required_block`, `strapping`, `reset_pin`, `boot_pin`, `electrical`,
`high_speed`, `keepout`) needs at least one **explicit** quotation from an **official** source
(`level` starting with `official_`). An open board, a community post, a derivation or a note that
is still pending does not make the chip require anything (`weak_evidence`). A reference board never
overrides the manufacturer's documents: if they differ, that is a `conflict`.

Evidence for a device must come from a source whose `applies_to` lists that device and the right
version: the manual of a similar model is not evidence (`source_not_applicable`).

`quote` is a short excerpt for review and for the check against the cached text; it is not a copy
of the document (mind the licence).

## Units

A numeric parameter carries its unit in its **key**: `width_mm`, `impedance_ohm`, `min_value_nf`,
`current_ma`, `skew_mm`, `min_count`. Accepted suffixes: `mm um mil ohm kohm v mv a ma ua uf nf pf
uh nh hz khz mhz ghz ns ps us ms s deg pct count ratio db`. A number under any other key is
refused (`unknown_unit`). Coordinates are millimetres, x to the right, **y down**, in the board's
frame (`frame: device` regions are in the device's own frame and the device must be named).

## Rules and their parameters

`kind` decides which parameters a rule needs (the schema enforces it; `unknown` and `conflict`
rules may have empty slots).

| `kind` | Subject | Required `params` | Meaning |
|---|---|---|---|
| `reserved_pin` | `device` | `pins` (numbers) | the design must not use these pins |
| `nc_pin` | `device` | `pins` | must be left unconnected |
| `fixed_net` | `device` | `pin`, `net_pattern` | the pin must be on a net matching the pattern (`*`, `?`) |
| `exclusive_functions` | `device` | `functions` (two or more) | cannot be in use together; also one pin cannot carry two functions |
| `required_block` | `block` | – | the block must exist in the design |
| `strapping` | `device`, `pin` | `sampled_at`, `default_level`, `external_drive_allowed` | the pin is read at reset/power-up |
| `reset_pin` | `device`, `pin` | `active_level` | the pin resets the device |
| `boot_pin` | `device`, `pin` | `boot_level` | the pin selects the boot mode |
| `electrical` | – | `quantity`, `condition` (+ values with units) | a limit (supply range, current, ...) |
| `high_speed` | `device` | `nets`, `differential` (+ `impedance_ohm`, `gap_mm`, `skew_mm`, `max_length_mm`, ...) | an interface routed with care |
| `clearance` | – | `clearance_mm` | minimum clearance (fastroute: `.kicad_dru`) |
| `trace_width` | – | `width_mm` | trace width (fastroute: `.kicad_dru`) |
| `keepout` | `region` | `forbid` (`track`, `via`, `component`, `zone`) | nothing of these kinds inside the region |
| `mechanical_fixed` | `region` | – | the mounting hole, connector, outline or module body stays where it is |

`applies_when` holds the conditions in words ("the USB interface is used", "module revision v1.1").
A rule never states a number without its condition when the document gave one.

## Blocks, regions, base design

* **Block** (`peripheral_blocks`): `components` (role, kind, `min_count`, value limits such as
  `min_value_nf` / `value_ohm`) and `connects` (pin → `net_pattern`). A `required` block must be
  found in the design; an `ai_editable` block is one an AI may add, where the base design allows it.
* **Region** (`regions`): `kind` is `board_outline`, `mount_hole` (a `circle`), `connector`,
  `module_body`, `antenna_keepout`, `keepout` or `editable`; `polygon` or `circle` in mm; `layers`.
* **Base design**: for "the base board is fixed, change only the outside". `binding` ties the pack to
  a repository commit and/or file SHA-256 digests (a changed base is detected and needs review);
  `immutable` lists the components, pins, connectors, regions that must not change; each
  `editable_regions` entry names the region, the blocks that may be added there, the pin pool the
  new circuit may use and the operations allowed. Anything not listed is read-only. The draft, the
  request and the checks are in `editable-base.md`.

## Ids and the link to hcp-manager

| Id | Form | Stable because |
|---|---|---|
| `source_id` | `SRC-<NAME>` | chosen once per source in the pack |
| `document_id` | `DOC-<NAME>` | the document in hcp-manager's inventory |
| `chunk_id` | `CHK-<DOC NAME>@<first 8 hex of the document's SHA-256>:p<page>.<n>` | page and ordinal of a block of text in that exact file: rebuilding the index of the same bytes gives the same ids, a changed document gives new ones (so old evidence is seen to be stale) |
| `device_id`, `block_id`, `rule_id`, `region_id` | `DEV-`, `BLK-`, `C-`, `REG-` + name | names in the pack |

The pack stores `source_id` + `locator` + `quote` (+ optionally `chunk_id`); hcp-manager resolves
them back to the original text (`hcp-manager evidence verify`). Nothing else of the index is stored.

## Versions and compatibility

`hcp_version` is `MAJOR.MINOR`. A minor version only **adds** optional properties; the meaning and
type of existing ones never change. The schema is strict (an unknown property is an error, so that
vector fields and typos cannot slip in), therefore a program reads packs up to the minor version it
knows and says so for a newer one (`unsupported_version`) instead of guessing. Put your own data
under `x_...` keys. A new major version may change anything.

**1.1** adds, all optional: `pack.kind` (`chip`, `module`, `board`), `pack.content` (`full` or `metadata_only`), `pack.upstream` (inheritance, below)
and `rule.overrides`. A pack that uses any of them says `hcp_version: "1.1"` (`needs_version_1_1`); a 1.0 pack is read as before.

## Packs of different kinds and inheritance (1.1)

A **chip** pack states what the chip requires. A **module** pack states what the module adds or changes (integrated flash and PSRAM, antenna,
pins brought out, mechanical limits) and may take named things from a chip pack; a **board** pack may take from a module or chip pack. A chip pack
inherits from nothing. Nothing is inherited unless it is listed:

```yaml
pack:
  id: vendor/family/module/variant
  kind: module
  upstream:
    - id: vendor/family/chip/variant     # the exact pack
      version: 0.1.0                      # the exact version: a similar one is never used
      kind: chip
      canonical_sha256: <64 hex>          # of the upstream HCP document, canonical JSON (`hcp-manager packs show` prints it)
      inherit: { devices: [DEV-CHIP], rules: [C-CHIP-GND], blocks: [], regions: [] }
rules:
  - id: C-MODULE-RSVD                     # a rule of the module, on the module's device, from the module's own document
  - id: C-CHIP-RSVD
    overrides: C-CHIP-RSVD                # replaces the inherited rule; needs evidence from a source of THIS pack
```

A pack that inherits is not usable by itself (`upstream_unresolved`): `fastroute constraints resolve` / `hcp-manager packs resolve` merge in the listed
upstream objects (with the sources they cite, so their evidence can still be checked), pin the result to the upstream digests and check it like any pack.
A changed upstream (`upstream_digest_mismatch`) makes the derived pack stale: it is reviewed and sealed again, not adopted. Other refusals:
`upstream_missing`, `upstream_kind_unknown` / `upstream_kind_mismatch` / `upstream_kind_not_allowed`, `inherit_unknown_id`, `override_of_uninherited`,
`override_without_own_evidence`, `inherit_conflict` (the same thing stated with other values and no `overrides`: replace it explicitly or record a `conflict`
rule), `id_clash`, `source_conflict`. A document of the module is not evidence for the chip's pins and the other way round (`source_not_applicable`): that is why
an override is a rule on the module's device.

`pack.content: metadata_only` registers an identity (a device, a vendor or a series that is not covered yet): no pins, rules, blocks, regions or base
design (`metadata_only_with_facts`). Without it a device has at least one pin (`no_pins`).

## Diagnostic codes

`fastroute constraints check` prints `severity code path (object): message`; `--json` the same as
data. The codes are stable.

| Code | Means |
|---|---|
| `parse` | not YAML / JSON |
| `schema` | the structure is wrong (the message says how); the other checks wait until it is right |
| `unsupported_version` | `hcp_version` is not one this program reads |
| `vector_field` | retrieval data (embedding, similarity, score, rank) in a pack |
| `duplicate_id` | an id used twice |
| `duplicate_pin` | a pin number twice, or a name twice where that is not allowed |
| `unknown_reference` | a device, block, region, rule or source that does not exist |
| `unknown_pin` | a pin the device does not have |
| `source_not_applicable` | evidence from a document that does not apply to the device |
| `weak_evidence` | `fixed` / `required` without an official explicit quotation |
| `pending_evidence` (warning) | evidence still to be confirmed |
| `unknown_unit` | a number whose key has no unit |
| `unknown_value` | a number in an `unknown` or `conflict` rule |
| `conflict_state` | a conflict without two different values |
| `derived_basis` / `derived_cycle` | a derived rule without derived evidence / derived from itself |
| `degenerate_region` / `wrong_region_kind` | a polygon without area, a region of the wrong kind for its use |
| `editable_not_whitelisted` | `ai_editable` without an entry in `base_design.editable_regions` |
| `pool_pin_protected` | a pin pool lists a pin that a rule protects (reserved, no-connect, strapping, reset, boot, fixed net) |
| `no_allowed_ops` (warning) | an editable region without `allowed_ops`: nothing may be done there until operations are listed |
| `review_incomplete` | reviewed or approved without who and when |
| `needs_version_1_1` | `pack.kind`, `pack.content`, `pack.upstream` or `rule.overrides` in a pack that says 1.0 |
| `upstream_unresolved` | a pack with `pack.upstream` read without being resolved |
| `no_pins` / `metadata_only_with_facts` | a device without pins in a full pack / facts in a metadata-only pack |
