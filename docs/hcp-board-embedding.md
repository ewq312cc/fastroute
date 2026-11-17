# Embedding a development board in a carrier

A ready-made development board can be used as a **fixed module** of a new carrier design: the board, its headers, outline and keep-outs stay as
they are, and an AI may only add peripherals and connections in the areas of the carrier that the board pack leaves free, using only the
header pins that the board brings out and the documents show to be usable. The board is **not** turned into something editable, and no
complete KiCad project is generated: the output is a plan (JSON), a carrier pack with its Constraint IR, a difference report and a readable
report. It builds on the fixed-base boundary of [HCP.md](HCP.md) (`base_design`, `edit-request`, `check-edit`).

## What a board pack says

A pack of `kind: board` (`hcp_version` 1.1, see [hcp-pack-release.md](hcp-pack-release.md) for kinds and inheritance) with a `board_integration` section.
The board's geometry is in **its own frame** as ordinary `regions` with `frame: device` and `device` = the board device.

| Where | What |
|---|---|
| `pack.upstream` | the module and/or chip pack the board is built on (named things only; see inheritance) |
| `devices[board_device]` | the board as a device. **Its pins are the header pins and nothing else**, numbered `<header>.<number>` (`J1.4`). A pin of the chip that is not on a header is not a pin of the board and is in no pool (`device_pin_not_exposed`) |
| `board_integration.hardware_revision` | the exact revision the pack is for. A request for another revision is refused |
| `board_integration.frame` | unit (mm) and, in words, where the origin is and which way x and y point (top view) |
| `regions` (frame `device`) | exactly one `board_outline`; `mount_hole`s; `module_body`; `antenna_keepout`; `keepout`: everything that may not be built over |
| `board_integration.exposed_headers[].pins[]` | `number`, `name`, `position` (board frame), `kind`, `usability`, `reason`, `module_pin`, `evidence` |
| `usability` | `usable` (needs evidence), `reserved` (the documents say it must not be used; reason), `onboard` (already used by something the board implements, for example the boot button or the USB-UART bridge; reason), `unknown` (the documents do not say: nothing is assumed; reason) |
| `module_pin`, `upstream_device` | which pin of the upstream device a header pin is wired to, when the documents say. A pin marked `usable` that an upstream rule protects (reserved, strapping, reset, boot, no-connect, fixed net) is an error (`board_pin_conflicts_with_upstream`): the board and the upstream pack disagree, and it has to be settled or recorded as a `conflict` |
| `board_integration.onboard_blocks` | the `peripheral_blocks` the board implements (power, reset, boot, clock, USB, LEDs). A carrier never adds them again |
| `board_integration.carrier` | `allowed_blocks` (`ai_editable` blocks), `allowed_ops`, `margin_mm` around the body |

Electrical facts (voltage, current, levels, pin multiplexing, strapping state) that no board-level document gives stay `unknown`: nothing is
inferred from the chip's datasheet about what the *board* supports.

## The request

`schemas/board-embedding-request-v1.schema.json`: the exact board (`id`, `version`, `hardware_revision`, `canonical_sha256` of the board pack's HCP
document, which `hcp-manager packs show` prints), the carrier (`name`, `outline`: a rectangle), the placement (`ref`, `origin_mm`, `rotation_deg`: 0, 90, 180 or
270 counter-clockwise about the board's origin; the board is **placed where you say**, there is no automatic placement) and optionally `targets` (the peripherals
the carrier is to get, as a block and header pins). Targets are **checked, never granted**.

## The command

```sh
hcp-manager packs embed --board <id>[@version] --request request.json --out DIR [--lang zh|en] [--force]
```

The board pack is read from the workspace (installed with `packs builtin install`, or imported), resolved with its exact upstream packs, and bound
to the request by digest. Written to `DIR` (nothing is overwritten without `--force`, and the inputs are never overwritten):

| File | What |
|---|---|
| `embedding.json` | `format: hcp-board-embedding`: the board in the carrier's frame (body, every header pin), the carrier's regions, `pin_pool`, `excluded_pins` with reasons, on-board blocks, the checked targets, `unknown`, notes, provenance |
| `carrier-pack.yaml` | a flat HCP document in the carrier's frame: the board device with its header pins, the on-board blocks (fixed) and the carrier blocks (`ai_editable`), the board's rules, the carrier outline, the board body and keep-outs (fixed regions), the free area cut into editable regions, and a `base_design` (immutable component = the board's reference; `editable_regions` with the pool, blocks and operations) |
| `ir.json` | the Constraint IR of that pack: what `fastroute constraints edit-request` and `check-edit` read |
| `diff-report.json` | what the carrier pack took from the board pack, what it generated, and what it left out, with the reason |
| `report.zh.md` / `report.en.md` | readable: what the board fixes, which header pins may be used, which may not and why, what is on the board already, what the carrier is to get, the editable areas, unknown items, where the evidence is |
| `board-embedding-request.json` | the request (its digest is the `binding` of the carrier pack) |

Exit codes: 0 embedded and nothing needs attention, 5 embedded but a target is blocked, a pin is unknown or there is no free area (the files are written and
say what), 3 the request or the pack is refused (nothing is written), 4 an output would be overwritten, 2 usage.

### What is refused (nothing is written)

`board_mismatch` (another pack id or version), `board_revision_mismatch` (another hardware revision), `board_pack_changed` (the pack differs from the one the
request names), `header_outside_carrier` (the board cannot be plugged in there), `request_schema`, `no_board_integration`, and every error of the board pack (including
`board_pin_conflicts_with_upstream` and an upstream that changed).

### The boundary of the carrier (the existing `check-edit`)

On the carrier pack, `check-edit` rejects, with evidence: a pin that is not in the pool (`pin_not_in_pool`: a pin the board does not bring out, a reserved,
on-board or unknown pin), a block that is not allowed (`unauthorized_block`: for example the boot button or the reset circuit again), moving or removing the board,
work in the body or the keep-outs (`outside_region`), a connection or a track from one free area into another, an operation that the area does not allow, and the
carrier outline as an area. These are tested on a fictional board (`crates/fr-constraint/tests/board.rs`).

## The sample

`docs/examples/hcp-devboard-sample/` holds a board pack (`board-pack.yaml`) and a request (`request.json`) for a **made-up** board, released under CC0 1.0, so that the command can be run offline and the files redistributed without a licence question. A test keeps the files equal to what the tests build and embeds them.

A **real** board pack is `espressif/esp32-s3/esp32-s3-devkitc-1/n16r8-v1.1` (ESP32-S3-DevKitC-1 v1.1, N16R8), with an example request and its reports in `hardware-packs/espressif/esp32-s3-devkitc-1-example/`. It is written from Espressif's user guide, schematic and DXF (which are not in the repository: their disclaimer grants no licence), is a draft that nobody has reviewed, and its embedding says it needs attention: three strapping pins and the RF clearance around the antenna are unknown. An unknown rule of a board pack is listed in the embedding's `unknown` and in the report, like an unknown pin, and makes the status `needs_attention`.

## Limits of this version (stated, not hidden)

* The carrier outline is a **rectangle**; the board is placed at **0, 90, 180 or 270°**. Other shapes and angles are not supported.
* The free area is computed exactly for rectangles. A board outline or a keep-out that is not a rectangle is replaced by its **bounding rectangle** (conservative: a little free area is
  lost, none is invented). The free area is cut into rectangles that do not overlap; a track that has to cross from one to another is refused (two areas are two regions).
* Nothing is written to a KiCad project and nothing is routed. The plan, the pack, the IR and the report are the output; a KiCad adapter is separate work.
* No height or clearance-under-the-board limit is modelled: the documents of a board rarely give one, so the whole body area is not editable.
* Whether a header pin is really free is only as good as the board pack's evidence; the pack of a real board is a draft until a person reviews it (see its `acceptance.md`).
