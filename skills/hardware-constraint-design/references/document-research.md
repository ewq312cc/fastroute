# Document research: from a part number to an evidence list

The point of this step is a list of documents that are **for exactly this device and version**,
ordered by how much they can be trusted, with a digest for each so that anybody can fetch the same
bytes again. Searching, caching and indexing belong to `hcp-manager`; this Skill calls it and never
keeps a second copy of the documents or an index of its own.

Contents: [1. Confirm the device](#1-confirm-the-device-before-searching) ·
[2. Source priority](#2-source-priority) · [3. Find the documents](#3-find-the-documents) ·
[4. Register and cache](#4-register-and-cache-them) · [5. Search the text](#5-search-the-text-keyword-first) ·
[6. Conflicts and gaps](#6-conflicts-and-gaps) · [7. When the app is missing, offline or has no index](#7-when-the-app-is-missing-offline-or-has-no-index) ·
[8. Checklist](#8-checklist)

## 1. Confirm the device before searching

Write down, from the user's board or request: **vendor, exact part number (with the suffix that
changes the product: memory size, antenna, temperature grade), package, silicon/module revision,
and the document versions in the user's hands**. If any of it is unclear, ask. Similar names are
different products ("-1" and "-1U", "N8" and "N16R8"): their pins, reserved pins and keep-outs
differ. **Never use the document of a similar model because the right one was not found.**

## 2. Source priority

| Rank | Source (`level`) | Use |
|---|---|---|
| 1 | Official datasheet / manual of this version (`official_manual`), errata (`official_errata`) | the only basis for `fixed` / `required` statements about the chip |
| 2 | Official hardware design guide, official reference design (`official_guide`, `official_reference_design`) | recommended circuits and layout; says "should" rather than "must" more often |
| 3 | Official dev board schematics (`official_devkit`) | what the manufacturer's own board does: `observed` |
| 4 | Open board with a version or commit (`open_board`) | what one design does: `observed`; useful for editable areas and mechanics |
| 5 | Community posts and forums (`community`) | leads only: find the official sentence they point to, or leave it out |

An open board **never overrides** an official statement. If the board does something the manual
forbids (or the other way round), write both down as a `conflict`; do not pick one.
`hcp-manager sources resolve` already sorts this way and says in each line why a document is
listed but not usable (wrong version, superseded, the link is dead, changed since it was cached).

## 3. Find the documents

Search the manufacturer's site first (documentation / support pages of the exact product), then
the manufacturer's repositories and design-guideline pages, then the board makers. Useful queries
(replace the names): `"<part number> datasheet"`, `"<part number> hardware design guidelines"`,
`"<part number> errata"`, `"<part number> reference design schematic"`, `"<part number> pin
definitions strapping"`, `"<part number> layout antenna keep-out"`. Open the document and read its
revision history: the version string you register must be the one printed in the document, not the
one in the URL.

Record, for each document: URL, **version**, retrieval date, which exact model(s) and revision(s)
it covers, and its licence note. Do not copy large parts of a document into the pack (mind the
licence): quote only the sentence that supports a rule.

## 4. Register and cache them

```sh
hcp-manager --workspace ws init
hcp-manager --workspace ws sources add --id DOC-ESP32S3-DS --title "…datasheet" \
    --level official_manual --version v1.6 --url https://… --applies-to ESP32-S3-WROOM-1
hcp-manager --workspace ws sources sync            # fetches, pins the SHA-256, never overwrites silently
hcp-manager --workspace ws sources resolve --device ESP32-S3-WROOM-1 --version v1.6
```

Or write an inventory file (`assets/example-sources.yaml`, schema `schemas/sources.schema.json`)
and `sources import` it. `sync` caches under the digest. If a document at the URL has changed since
it was pinned, `sync` reports `changed`, keeps the old copy and does **not** adopt the new one
(`sync --accept` is a decision: afterwards rebuild the index and run `evidence verify` because
quoted text may have moved). A dead link keeps the cached copy and is reported, never hidden.

`sources resolve` prints what is **missing** (for example "no usable official manual") and lists
documents of similar models as *excluded*. If the official manual of this exact version is missing,
stop and tell the user: rules about the chip stay `unknown` and the list of gaps is the result.

## 5. Search the text (keyword first)

```sh
hcp-manager --workspace ws index build
hcp-manager --workspace ws --json search "strapping pins boot mode" --device ESP32-S3-WROOM-1 --version v1.6
```

* The search **filters before it ranks**: only documents for the given device, revision, version
  and source level are searched, so a similar model's text cannot be found by accident.
* Results come back with document, version, page, section and a block id. Read the **original
  text** of every candidate (open the page) before you use it. A hit, its rank or its score is
  never evidence; a quotation you have read is.
* `--mode hybrid` is accepted for the day a vector index exists; today it answers with keyword
  search and says so (`degraded`). Nothing depends on it.

Put what you read into the pack as evidence (`source_id`, page/section, a short quotation, `basis`)
and then check it against the documents:

```sh
hcp-manager --workspace ws evidence verify pack.yaml    # source, digest, version, block id, quotation on the page
```

## 6. Conflicts and gaps

* Two documents disagree (manual v1.4 vs errata, or manual vs board): write a `conflict` rule with
  both values and their evidence. No voting, no "most recent wins" unless a document says it does
  (an errata that explicitly supersedes a sentence is an `explicit` basis for the newer value).
* A value is not in the documents: `unknown` with `missing:` saying what is missing. Never fill it
  with a typical value; "impedance 50 ohm" in a forum is not the manual's 90 ohm.
* A document is for a different version than the user's silicon: do not use it for `fixed` or
  `required`; list it as a gap or ask the user.

## 7. When the app is missing, offline or has no index

| State | What to do |
|---|---|
| `hcp-manager` is not installed | say so; ask the user to install it or to provide the documents and the inventory; do not simulate it. Packs that already exist can still be checked, compiled and validated (`fastroute constraints ...`) |
| no network | `sources sync` reports `unreachable`; work from the cached copies (`index build`, `search` work offline) or from documents the user provides with `sources add --file` |
| index missing or stale | `hcp-manager index build` (or `rebuild`); `index status` says what is stale. PDFs need `pdftotext`; without it the status says the PDF has no text |
| no vector index | keyword search is the normal path; the evidence requirements do not change |

## 8. Checklist

- [ ] The exact device, revision and document versions are written down.
- [ ] Every document in the list has a version, a digest and a level; none is for a similar model.
- [ ] The missing kinds of source are listed (`sources resolve`), not papered over.
- [ ] Every quotation in the pack was read in the original and passes `evidence verify`.
- [ ] Community and open-board material is marked `observed` or left out.
