---
name: hardware-constraint-design
description: Use before answering or changing anything that depends on what a specific chip, module or board requires, instead of answering from memory - pin usability, reserved and strapping pins (BOOT, EN/RESET, GPIO0, flash/PSRAM), circuits a design guide demands, USB or other high-speed routing, antenna keep-outs, mounting holes, and what may or may not change on an open-source board or core-module carrier. Turns datasheets, design guides and reference boards into a Hardware Constraint Pack (page-level evidence, each rule fixed, required, configurable, unknown or conflict), validates a KiCad PCB against it offline, feeds the fastroute router, and limits an AI to whitelisted regions of a fixed base board. Triggers include - list what this part requires with page numbers, check my schematic or PCB against the datasheet, can I use this GPIO, what must I not touch, add circuits to this open board, 引脚能不能用, 按手册检查, 提取硬件约束, 哪些能改, 天线禁布区. Not for running the router, general electronics theory or unrelated code.
---

# Hardware constraint design

Turn "what does this part require?" into a **pack**: a versioned YAML file in which every rule says what is
required, how binding it is (fixed, required, configurable, ai_editable, unknown, conflict) and where the
documents say so (document, version, page or section, the quoted words). Deterministic programs then check
the pack, compile it, validate a design against it and hand the executable part to the router. You do the
reading and the judgement; the programs do everything that must give the same answer twice.

## Who does what

| | Does | Never |
|---|---|---|
| **you (this Skill)** | choose the documents, read the original pages, write rules with evidence, draft edits | state a number from memory, from a similar part, or from a search score; keep a cache or an index |
| **hcp-manager** (independent application) | document inventory, download, cache and digests, the embedded index with keyword, vector and hybrid search, evidence check, pack store, the packs that ship with the software (verify, install offline, resolve inheritance) | judge what a document means |
| **fastroute constraints** | schema, check, compile, report, validate, export, edit-request, check-edit | use an AI model, the network or a vector index |

`scripts/hcp_cli.py` is the one command line over the two programs; it finds them, checks their versions,
forwards your command and says what to do when something is missing. Start with `python3 scripts/hcp_cli.py doctor`.

## Workflow

1. **Pin down the device and the task.** Exact part number, revision, package or module variant (the `-U`,
   `-N8`, `R2` suffix matters). The task is one of: (a) build or review a pack for a device, (b) check a design
   against a pack, (c) extend a fixed base board. Ask only if the part or variant is ambiguous.
2. **Research** (`hcp_cli.py research ...`, details in `references/document-research.md`). First see whether the software ships a pack for the exact
   device (`research packs builtin list`; `install` puts it, and the chip or module packs it inherits from, into the workspace; a pack that inherits is
   used after `resolve`). A shipped pack is a starting point that a person reviewed, not a reason to skip the evidence check. `resolve` lists the
   documents for the exact device, best source first (official manual and errata, then guides and reference designs,
   then dev kits, open boards, community); `sync` fetches and pins digests; `index build` (blocks and vectors); `search` filters by device and
   version *before* ranking (`--mode keyword|vector|hybrid`). A search hit is a lead: `chunk show ID` and the page are
   the original; read it before you use it.
3. **Extract** (`references/constraint-extraction.md`; topic guides: `boot-reset.md`, `high-speed.md`,
   `mechanical.md`; `hcp_cli.py extract templates` lists rule templates). Write the pack from `assets/example-device.yaml`;
   format in `references/hcp-format.md`. Rules about the chip need an `explicit` quotation from an official
   document. What a reference board does is `observed` and makes a `configurable` or `optional` rule. When the documents do not say,
   write the rule as `unknown` with what is missing; when they disagree, `conflict` with both values and sources.
4. **Check and compile**: `check PACK` (exit 3 = fix the pack), `compile PACK -o ir.json`, `report PACK -o report.md`
   (Chinese by default, `--lang en`), `research verify PACK` (every quotation is in the cited document at the cited
   version). `pipeline PACK --out DIR [--kicad-pcb BOARD]` runs all of it and writes `pipeline-status.json`.
5. **Confirm with the person** before anything is called reviewed: show the report's lists of what cannot change,
   what must stay, what may be configured, and above all the unknown and conflicting items. Review status, name and
   date are theirs to give.
6. **Design present?** `validate --pack PACK --kicad-pcb BOARD` (or `--design snapshot.json`). Report it as it is:
   violations with rule, object, actual and required value, evidence; and the rules that were *not* checked. It is not
   KiCad's DRC and not an electrical review. `export --pack PACK --kicad-pcb BOARD -o DIR` writes what fastroute
   can execute (pairs, length matching, clearance, track width) and lists what it cannot.
7. **Fixed base board** (`references/editable-base.md`): `edit-request` gives the whitelist; you answer with a
   draft (changes in whitelisted regions, blocks, pins, operations - or `stop` with what is missing); `check-edit`
   accepts or rejects it and writes the before/after report. The draft never overwrites the project.

## Rules that do not bend

- **Unknown stays unknown.** No typical value, no "same as the similar chip", no number from training data.
- **Authority comes from the pack, not from search.** A keyword or vector hit never adds a region, block, pin or
  operation to what an AI may edit. If the requirement does not fit the whitelist, stop and report; do not widen it.
- **Official, explicit evidence for `fixed` and `required`.** Mark evidence `derived` or `pending` honestly.
- **A pass is only as strong as what was checked.** Say how many rules were unchecked, unsupported, unknown or in
  conflict (exit 5), and which.
- **Similar models are different documents.** A document for another variant is not evidence for this device.

## When something is missing

| Situation | What you do |
|---|---|
| `doctor` says fastroute is missing or too old (exit 6) | Stop and tell the user how to build it (`cargo build --release -p fastroute`); nothing can be checked without it |
| hcp-manager missing, incompatible (`interface` is not `"1"`) or offline | The local workflow still runs (check, compile, report, validate). Work from the pack and the documents the user supplies, mark the evidence **not verified**, list the quotations for a person to compare |
| No inventory / index missing or stale | `research init`, `sources import|add`, `sync`, `index build` - the index is derived and rebuildable |
| No usable vectors (provider failed, configuration changed, remote not authorised) | `--mode hybrid` says so and is a keyword search. The default vectors are lexical; never depend on a vector result, and never authorise a remote provider on the user's behalf |
| Document not found, or only a community source | Rule stays `unknown`; ask the user for the official document; say what you looked at |
| Documents disagree | `conflict`; show both; ask which applies |
| `check-edit` rejects, or says review | Show the findings and the report. Rejected: do not "fix" by widening authority - change the draft or `stop`. Review (base or pack changed): the person decides whether the pack is still right |

## Files

| Read when | File |
|---|---|
| writing or reading a pack, diagnostic codes | `references/hcp-format.md` |
| choosing and reading documents, source priority, locking versions | `references/document-research.md` |
| turning text into rules: wording of evidence, what is explicit, derived, unknown | `references/constraint-extraction.md` |
| a part with strapping, reset, boot or download-mode pins | `references/boot-reset.md` |
| USB, differential pairs, length matching, RF | `references/high-speed.md` |
| antenna keep-out, outline, mounting holes, connectors | `references/mechanical.md` |
| an open board or core module that must stay as it is | `references/editable-base.md` |
| tools, versions, failure branches, the status file | `references/integration.md` |
| the JSON Schemas (pack, IR, design snapshot, edit draft and request, inventory) | `schemas/` |
| a worked example and rule templates | `assets/example-device.yaml`, `assets/example-sources.yaml`, `assets/example-edit-draft.json`, `assets/rule-templates.yaml` |
