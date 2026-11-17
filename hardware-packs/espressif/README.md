# Official Espressif packs: ESP32, ESP32-S3, ESP32-C3

Six packs, a chip pack and a module pack for each series, written from Espressif's own documents and shipped in the built-in pack release
(`hardware-packs/index.yaml`). This file is their acceptance record: what is in them, what is not, and what a person still has to confirm.
Every pack is `review: draft`. Nobody has reviewed them.

Two more packs are in the same release and are described at the end: the module pack `espressif/esp32-s3/esp32-s3-wroom-1/n16r8` and the board pack of the
ESP32-S3-DevKitC-1 v1.1 (see "The development board pack"). `coverage.yaml` and the nine tests of `espressif.rs` are about the six packs of the table below.

| Series | Chip pack | Module pack (inherits the chip pack) |
|---|---|---|
| ESP32 | `espressif/esp32/esp32/d0wd-v3` (ESP32-D0WD-V3, QFN48) | `espressif/esp32/esp32-wroom-32e/n4` (ESP32-WROOM-32E-N4) |
| ESP32-S3 | `espressif/esp32-s3/esp32-s3/qfn56` (ESP32-S3, QFN56) | `espressif/esp32-s3/esp32-s3-wroom-1/n8r8` (ESP32-S3-WROOM-1-N8R8) |
| ESP32-C3 | `espressif/esp32-c3/esp32-c3/qfn32` (ESP32-C3, QFN32) | `espressif/esp32-c3/esp32-c3-wroom-02/n4` (ESP32-C3-WROOM-02-N4) |

Each pack directory `<vendor>/<family>/<device>/<variant>/0.1.0/` holds `pack.yaml` (the manifest), `hcp.yaml` (the constraints),
`sources.yaml` (the documents the pack is locked to: URL, version, SHA-256, retrieved on 2026-11-16) and `reports/pack-report.zh.md`
(the Chinese report: fixed / required / configurable / unknown, with the quoted evidence). The documents themselves are Espressif's and are
not in the repository; `hcp-manager` downloads them and refuses a file whose digest does not match.

## Layers

* A **chip pack** states what the silicon states: the supply range, the strapping pins and their default levels, the boot and reset behaviour, the
  required external parts, the USB pair. It cites the chip datasheet and the hardware design guidelines.
* A **module pack** declares the chip pack as an exact dependency (`id` + `version` + the canonical digest of its `hcp.yaml`) and inherits only
  the device and the rules of the kinds `strapping`, `boot_pin`, `reset_pin` and `electrical`. It adds what only the module has: its own pin table
  (the module's pin numbers, not the chip's), the ground pads, the reserved or not-connected pins of that memory variant, the antenna keep-out. It cites
  the module datasheet. A module rule that changes an inherited one is a `rule.overrides` entry and needs evidence of its own; none is silent.
* `hcp-manager packs resolve` (or `fastroute constraints resolve`) merges the layers into one flat pack. A pack that inherits is unusable until it
  is resolved (`upstream_unresolved`), and a dependency whose digest differs from the one in the manifest is refused.
* The ESP32-S3 example of #16 (`hardware-packs/espressif/esp32-s3-wroom-1-n8r8/`) is a third layer: a `board`-kind overlay (the made-up carrier) over the
  S3 module pack. Its flat `pack.yaml` is generated from the overlay and the module pack and was compared object by object with the pack it replaced. The
  module's facts exist once; there is no second, possibly conflicting copy.

## Coverage

`coverage.yaml` says, per series, per kind of pack and per topic (power supply, ground, decoupling, required external parts, boot / strapping, reset,
download interface, clock, fixed or reserved pins, configurable pins, high-speed, mechanical, module RF), one of exactly four statuses:

| Status | Meaning |
|---|---|
| `supported` | a rule of the pack states it, with evidence from a locked document; the rule ids are listed |
| `unknown` | the documents were not read for it, or do not say; nothing is assumed; the reason is given |
| `not_applicable` | this kind of pack cannot state it (a chip pack has no antenna); the reason is given |
| `conflict` | documents disagree; none is recorded at present, and the test requires the list to stay empty until a `conflict` rule exists |

`apps/hcp-manager/tests/espressif.rs` checks `coverage.yaml` against the packs: every rule named under `supported` exists, is not `unknown`, and has
evidence; every `unknown` and `not_applicable` has a reason; every `unknown` rule of a pack is named in the file, so an open question cannot be hidden
behind a topic that is otherwise covered.

**What "supported" does not mean.** It means a rule with evidence exists for that topic. It does not mean the topic is complete, and it does not mean the
validator checks it: the validator has no electrical model, so supply limits are listed as "reported, not checked", and it has no impedance model, so the 90 ohm
USB figure is reported, not measured. Each design check says which of these it did not do.

### What is deliberately `unknown`

* **Antenna keep-outs of the ESP32-WROOM-32E and ESP32-C3-WROOM-02 modules.** The datasheets draw the zone in a figure. No number was read, so the rule is
  `unknown` and a design that has to pass it does not. The ESP32-S3 module keeps its keep-out figure read from the Hardware Design Guidelines, marked as
  pending a person's confirmation.
* **The final RC values of the EN (reset) circuit**, **the USB length-difference and spacing limits**, **decoupling capacitors** of the ESP32 and ESP32-C3
  supplies, **the download interface a design must provide**, and **the function assignment of the configurable pins**.
* **Other variants.** The R2 / R8 / N16 memory variants, other modules (WROOM-1U, MINI, ...) and other chips of a series are other packs. The S3 module pack
  is the N8R8 variant, because IO35 to IO37 are reserved for the octal PSRAM only on it.

## Tests

```bash
cargo test -p hcp-manager --test espressif
```

Nine tests, run offline:

* the six packs verify and the catalogue is the lock (`index.yaml` is regenerated and compared);
* chips and modules keep their own facts, and a rule of one chip or module is not evidence for another: a module citing another module's datasheet, a chip of another
  series, another version or a changed digest of the chip pack, and a near-miss variant id are each refused;
* a legal minimal design passes for each module, is never reported as a complete pass, and the report lists the `unknown` rules and the limits that were reported, not checked;
* each mistake fails at the rule made for it, for each module: a ground pin on the supply, the reset pin floating or held low, the boot pin tied to ground or to the supply,
  the reset circuit or the boot button deleted, a reserved or not-connected pin in use (where the module has such a rule), and, for the ESP32-S3 module, a part in the antenna
  keep-out. For the ESP32 and ESP32-C3 modules the keep-out size is not in the documents read, so the test asserts that the rule says `unknown` and that no design passes it;
* a high-speed number without evidence is refused, and the numbers the documents do not give stay `unknown`;
* `coverage.yaml` matches the packs (above);
* nothing of the vector layer is in a pack or the release, and the packs compile to the same IR every time;
* the release builds, verifies and installs offline, and building it twice gives the same bytes.

`hcp-manager --workspace hardware-packs/espressif/workspace evidence verify <hcp.yaml>` checks every quotation against the cached document (page,
normalised text): 313 entries over the six packs, none failing, when this record was written. That needs the documents (cache or network); it is
not part of the offline test run.

## The development board pack (ESP32-S3-DevKitC-1 v1.1, N16R8)

`espressif/esp32-s3/esp32-s3-devkitc-1/n16r8-v1.1` is a pack of kind `board` (#20): a ready-made development board as a fixed module of a carrier.
It is built on the module pack `espressif/esp32-s3/esp32-s3-wroom-1/n16r8` (16 MB quad flash, 8 MB octal PSRAM; the same facts as the N8R8 module pack with
the variant's own ordering-table row as evidence), which is built on the S3 chip pack. The board's DevKitC-1 variants are in its user guide's ordering table: there is **no N8R8 board**,
so the N8R8 module pack is not the module of any DevKitC-1.

* Written by `hardware-packs/tools/build_devkitc.py` from three locked documents: the user guide v1.1 (HTML: the header tables, the components, the notes), the board
  schematic V1.1 (PDF, sheet 2: which nets reach the USB-to-UART bridge, the second USB connector, the buttons and the LED) and the DXF dimensions source file (the outline, the
  2 x 22 header pad centres, the antenna outline and the USB connector overhang, read by `hardware-packs/tools/dxf_geometry.py`). The build compares the DXF's silkscreen labels
  with the guide's names for every header pin and stops on a difference.
* 44 header pins: **24 usable**, 7 on the board (RST, GPIO0, TX, RX, GPIO19, GPIO20, GPIO38), 10 reserved (GPIO35 to GPIO37, power, ground), **3 unknown** (the strapping pins GPIO3, GPIO45 and
  GPIO46, which the board documents do not call free). A header pin that the board calls usable but the module pack protects (strapping, reset, boot, octal PSRAM) is an error of the pack check.
* **The RF clearance a carrier must keep around the antenna is unknown** (`C-DK-ANT-CLEARANCE`); the forbidden zone of the pack is only the antenna part of the module (18 x 6.3 mm beyond the
  board's top edge). The example embedding (`esp32-s3-devkitc-1-example/`) is therefore **exit 5, needs attention**, and its report lists the open questions.
* 208 of 208 quotations verified by `hcp-manager evidence verify`. The documents are Espressif's and their disclaimer grants no licence, so they are not in the repository (URL, version and SHA-256
  only); the pack keeps short quotations, page or section locators and measured dimensions. Whether coordinates measured from a vendor's CAD file may be kept in a GPL-licensed repository is a question a
  person should answer before this pack is shipped to others.
* `apps/hcp-manager/tests/devkitc.rs`: nine offline tests: the pack resolves and checks clean and every pin has its evidence or reason; the geometry; the protected-pin conflicts for all eight protected pins;
  the N8R8 module is refused; the embedding at four rotations; the refusals (another revision, another variant, a changed pack); the edit boundary of #17 on the real carrier (legal and illegal edits: pins
  that are not brought out, on-board, reserved and unknown pins, the boot button / reset circuit / bridge / regulator added again, moving or removing the board, the body, the antenna zone and the USB
  connectors, a track across the board, a via, the carrier outline); and the committed example equals what the command writes today.
* Not covered: other hardware revisions (the initial revision drives its RGB LED from GPIO48), the other variants (WROOM-1U, WROOM-2, the end-of-life N8 and N8R2) and the part heights.

## Rebuild

```bash
cargo build --release -p fastroute -p hcp-manager
python3 hardware-packs/tools/build_espressif.py          # writes hcp.yaml, sources.yaml, the manifests and the reports (the six packs, the N16R8 module, the #16 example)
python3 hardware-packs/tools/build_devkitc.py            # the DevKitC-1 board pack (needs the N16R8 module pack built first)
hcp-manager release index --packs hardware-packs         # rewrites and checks index.yaml
```

The builder verifies each quotation against the page it cites while it writes the pack (`hardware-packs/tools/pdfq.py`, `pdftotext -layout` and a
normalised comparison); it needs the cached documents. `workspace/sources.yaml` is the inventory; the cache and derived files are ignored by git.

## What a person has to confirm

* The whole pack set. All six are drafts: the rules were read and quoted by a program-assisted process and checked against the pages, not reviewed by a
  hardware engineer.
* The ESP32-S3 antenna keep-out, which was read from a figure (rule `REG-ESP-ANT`, basis pending).
* The unversioned Hardware Design Guidelines pages. They are "latest" on the day of retrieval (2026-11-16); the pack pins their SHA-256, so a later change
  is detected, but nobody decides whether the changed text still says the same.
* That a strapping default stated for the chip also holds for the module's own internal pull circuitry: the module datasheets were read for it, but only
  the statements quoted in the rules are evidence.

## Not done

* No real board, schematic or KiCad project of any of these chips was validated: the legal and illegal designs in the tests are small made-up designs.
* Nothing was run against a vendor tool or a manufacturer's reference design. The packs have not been compared with Espressif's own checklists beyond the
  quoted statements.
* The matrix is not a statement about the chips' abilities. A `supported` topic is a statement about what these packs cover.
