# Acceptance record: ESP32-S3-WROOM-1-N8R8 example pack (issue #16)

Date of the run: 2026-11-16. Programs: this repository's `fastroute` and `hcp-manager` 0.1.14, KiCad 8.0.9 (`kicad-cli`).
The review status of the pack is **draft**: no person has reviewed it.

## What was locked (nothing filled in from memory)

| Document | Version | Level | SHA-256 (first 16) |
|---|---|---|---|
| ESP32-S3-WROOM-1 & WROOM-1U Datasheet (53 pages) | v1.8 | official manual | `27d71971da07c280` |
| ESP32-S3 Series Datasheet (87 pages) | v2.2 | official manual | `2d5a7cb7fd559d8d` |
| ESP32-S3 Hardware Design Guidelines, Schematic Checklist (web page) | latest, retrieved 2026-11-16 | official guide | `6cc621dfa1caec2c` |
| ESP32-S3 Hardware Design Guidelines, PCB Layout Design (web page) | latest, retrieved 2026-11-16 | official guide | `92318f21920eb2fc` |
| ESP32-S3-WROOM-2 Datasheet (a similar module; in the inventory on purpose, not cited) | v1.7 | official manual | `71701928d6251965` |
| Notes of the example carrier board (made up for this example) | example-1 | open board | `9b7e0bae770efa77` |

The 41 pins are Table 3-1 of the module datasheet, read by `tools/extract_pin_table.py` (`--check` compares them again). Every other
number is in a quotation of the page it comes from: **84 pieces of evidence, all verified** by `hcp-manager evidence verify` (document,
version, digest, and the words on the page). What the documents do not give stays `unknown` (three rules).

## Result of the legal example

`fastroute constraints validate` on `example/carrier.kicad_pcb`: **pass, exit 5** (nothing violated; not everything could be checked).
24 rules: 18 passed, 0 failed, 2 not checked, 1 not supported, 3 unknown.

| Not checked / not supported / unknown | Why |
|---|---|
| `C-ESP-USB` (unchecked) | the 90 Ω ±10% impedance needs the stack-up; the validator has no impedance model |
| `PINMUX-…` (unchecked) | a KiCad board carries no pin assignments |
| `C-ESP-ELECTRICAL-VDD33` (unsupported) | no electrical model: the 3.0 to 3.6 V and 0.5 A limits are reported, not checked |
| `C-ESP-USB-SKEW`, `C-ESP-USB-GAP`, `C-ESP-EN-RC-VALUES` (unknown) | the guidelines say "equal lengths" without a number, give no spacing, and the final EN values depend on the project's supply ramp |

The carrier is also clean in **KiCad's own DRC** (`kicad-cli pcb drc`: no error; warnings only for the missing library table, a dangling via
and unrouted connections), checked by a test when `kicad-cli` 8 or newer is there. The two results are separate things.

## Negative examples (each fails at the expected rule, with that rule's evidence)

| Mistake | Rule that rejects it | Test |
|---|---|---|
| the supply capacitor of the power entrance is missing | `C-ESP-NEED-POWER` (error) | `each_mistake_fails_at_the_expected_rule…` |
| GPIO0 tied to ground, or to the supply (BOOT conflict) | `C-ESP-BOOT-IO0` (+ `C-ESP-NEED-BOOT` for the supply) | same |
| IO36 (octal PSRAM) used | `C-ESP-RESERVED-PSRAM` | same |
| EN left floating | `C-ESP-EN` (+ `C-ESP-NEED-RESET`) | same |
| a ground pin on the supply | `C-ESP-GND-40` | same |
| a track or a component in the antenna keep-out | `C-ESP-ANT-KEEPOUT` | same |
| a mounting hole moved, the outline changed | `C-ESP-MOUNT-H2`, `C-ESP-OUTLINE` | same |
| edits outside the free area, reserved / strapping / unlisted pins, an unauthorised block, copper across the border, a protected net, an operation or region that is not on the list | `check-edit` rejects (exit 1) with `outside_region`, `crosses_region`, `reserved_pin_used`, `pin_not_in_pool`, `unauthorized_block`, `op_not_allowed`, `region_not_editable`, `protected_net` | `every_kind_of_overreach_is_rejected…` |
| a block that a vector search "recalled" | `unauthorized_block`, same verdict with or without the recall; a draft that claims it is `widened_authorization` | `a_block_that_a_vector_search_recalled…` |
| the open board changed after the pack was made | `needs_review` (exit 5), the board is never written | `a_changed_base_board_needs_a_review…` |
| evidence from the WROOM-2 datasheet for a WROOM-1 fact | `fastroute constraints check` exit 3, `source_not_applicable` | `evidence_from_the_datasheet_of_a_similar_module…` |

The legal draft (a status LED with its resistor on the free pin IO4, inside the free area) is accepted, complete, exit 0.

## Retrieval, index and determinism

* The nine cases of `eval/retrieval-eval.yaml` find the right page of the right document version at the ranks the set asks for; a
  hit names the version and carries a block id that contains the digest of the document it was built from.
* All 84 quotations of the pack are brought back by a search with the device filter: **67 at the first place, 17 at the second, none
  missing**. (hcp-manager ranks by source level first and by score second, so a fact that only a design guide states is looked for with
  `--source-level official_guide`; the evaluation set does that.)
* The WROOM-2 datasheet is **never searched for the WROOM-1** (`documents_excluded`: "not for ESP32-S3-WROOM-1-N8R8", and
  `sources resolve` lists it as a similar model that was excluded); without the device filter it is searched.
* Deleting the derived text and building it again: a search finds nothing in between and says why; after the rebuild the answers,
  the evidence check, the compiled IR and the validation are **byte for byte** the same.
* The compile / validate / report path runs with an empty `PATH`, a dead proxy and no index and gives the committed files.
* The Skill's script repeats the flow (check, compile, report, evidence, validate) and writes the committed IR and report.

## Supported, not supported, needs a person

**Supported and tested**: the flow from the inventory through download, digest check, index, search, evidence check, schema, compile,
validate, report and edit-draft check; hcp-manager managing the documents and the pack without the Skill (`sources resolve`, `packs
show`, `packs export`).

**Not supported (said, not hidden)**

* **Vector search.** hcp-manager has no vector index in this version (`vector_index: not available`; `--mode hybrid` falls back to
  keywords and says so). Everything in this record about the index is about the **keyword** index. That vector recall goes back to the right
  version of the text, and that a vector recall of a similar model is isolated, is shown for the keyword search and for the whitelist (a recalled block
  changes nothing); a vector service itself was not built or tested. The check, the whitelist and the validation do not depend on it.
* The validator has no electrical model and no impedance model; spacing and skew are not checked because the documents give none.
* The router can execute neither the keep-out nor the impedance; it gets nothing from this pack (the USB pair has neither spacing nor
  skew to hand over): `export-fastroute` writes no file and lists every rule with the reason.
* Chunk ids are not written into the pack's evidence (it cites document, version, digest, page or section and the quotation).

**Needs a person to confirm**

1. **The antenna keep-out.** The guideline gives its size only in a figure ("Keepout Zone for ESP32-S3 Module's Antenna (Antenna feed
   point on the right)": 15 mm minimum to the left, 6 mm, 1 mm and 2 mm maximum margins). The region was read from the dimension labels of
   that figure and is marked `pending`. Which of the two mirrored figures applies to a design depends on the module's antenna side, and the
   region assumes the footprint origin is the centre of the 18 × 25.5 mm body (a convention of this pack that no document gives).
2. **Review of the whole pack** (it is a draft): especially the recommendations that were written as `configurable` with a severity
   (the supply capacitor is an error, the others warnings) and `C-ESP-NEED-BOOT` (derived: a board that enters download mode only
   through an auto-reset circuit and has no button or header on IO0 is reported).
3. **The strapping rules for IO0, IO3, IO45, IO46 allow external drive** (the datasheet says a host MCU may drive them), so they check
   nothing about the net: they protect the pins from the editable pool and document the default levels. A boot conflict is found by the boot rule.
   IO45 does not select the VDD_SPI voltage on modules with PSRAM (module schematics, page 39); the pack does not model that.
4. **The documents that have no version number.** The two guideline pages are "latest": the digest identifies the text that was read; if
   Espressif changes a page, `sources sync` reports `changed` and the evidence check says which quotations moved.
5. **EPAD.** The datasheet says soldering the EPAD to ground "is not a must" (page 41), so there is no rule that requires it.
6. **The example carrier** is made up: its module footprint is simplified (pad pitch and body size, not Espressif's land pattern). It shows
   the flow; it is not a reference design.

## One thing found by looking at the page

The first draft of this pack said that no document gave values for the EN circuit and had a fixed rule that the EPAD must be grounded. Both
were wrong; the text extraction of page 41 had been read around its figure. Looking at the rendered page showed "TBD" for R1 and C2 in
Figure 9-1 **and** a sentence below it with the module's own recommendation (usually 10 kΩ and 1 µF, adjust to the power-up timing), and the
sentence that soldering the EPAD to ground is not a must. The pack was corrected (rules and evidence), which is why a quotation check
alone is not a review.
