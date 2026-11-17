# ESP32-S3-DevKitC-1 in a carrier: example

`request.json` asks `hcp-manager packs embed` to place the ESP32-S3-DevKitC-1 v1.1 (N16R8) on an 80 x 100 mm carrier, with its antenna end sticking out over
the carrier's top edge, and to check that the carrier may get a temperature sensor on the header pins J1.4 and J1.5 (GPIO4 and GPIO5). `out/` is what the command
writes (the plan `embedding.json` and the readable reports `report.en.md` / `report.zh.md`); a test keeps these files equal to what the command writes today.
The pack is `espressif/esp32-s3/esp32-s3-devkitc-1/n16r8-v1.1` (see `hardware-packs/espressif/README.md`); it needs the module pack
`espressif/esp32-s3/esp32-s3-wroom-1/n16r8` and the chip pack `espressif/esp32-s3/esp32-s3/qfn56`, which `packs builtin install` brings along.

```bash
cargo build --release -p hcp-manager
export PATH="$PWD/target/release:$PATH"
hcp-manager release build --packs hardware-packs --out /tmp/hcp-release
WS=$(mktemp -d)
hcp-manager --workspace "$WS" packs builtin --release /tmp/hcp-release install espressif/esp32-s3/esp32-s3-devkitc-1/n16r8-v1.1
hcp-manager --workspace "$WS" packs embed --board espressif/esp32-s3/esp32-s3-devkitc-1/n16r8-v1.1 \
    --request hardware-packs/espressif/esp32-s3-devkitc-1-example/request.json --out /tmp/devkitc-out --lang en
```

Nothing needs a network, a model or an index. The exit code is **5** (embedded, needs attention), and that is the right answer: the report names what is still open, and the
sensor on GPIO4 and GPIO5 is feasible.

## What it says

* **24 of the 44 header pins may be used by the carrier**: GPIO1, GPIO2, GPIO4 to GPIO18, GPIO21, GPIO39 to GPIO42, GPIO47 and GPIO48. Each has the row of the user guide's header table and the
  guide's statement that the available GPIO pins are broken out, as evidence.
* **7 are on the board already** and never in the pool: RST (the Reset button), GPIO0 (the Boot button, also a strapping pin), TX / RX (the USB-to-UART bridge on UART0), GPIO19 / GPIO20 (the
  ESP32-S3 USB port) and GPIO38 (the RGB LED of revision v1.1).
* **10 are reserved**: GPIO35, GPIO36 and GPIO37 (the module's octal PSRAM), the two 3V3 pins, 5V and the four ground pins.
* **3 are unknown**: GPIO3, GPIO45 and GPIO46 are strapping pins of the module and the board documents do not say they are free; nothing is assumed.
* **The RF clearance around the antenna is unknown** (`C-DK-ANT-CLEARANCE`). The zone that the pack does forbid is the antenna part of the module, 18 x 6.3 mm beyond the board's top edge, as
  the DXF draws it. What a carrier must keep clear beside it is written in no board document; the module guideline is for a module soldered to a base board.
* The board body, that antenna zone and the two USB connector overhangs are fixed: the carrier's editable area is the rest, cut into non-overlapping rectangles (7 here; two slivers narrower than
  0.5 mm are left out).

## Where the numbers come from

* The header tables (number, name, function) are read out of Espressif's user guide v1.1 page; the pin positions, the outline (25.4 x 62.865 mm), the antenna outline and the USB overhang are
  measured in Espressif's DXF ("Dimensions source file", V1.1, 2022-04-29). The build compares the silkscreen labels in the DXF with the user guide's names for all 44 pins and stops if one differs.
* The drawing's own dimension text says 62.74 mm for the length; its outline layer measures 62.865 mm. The pack uses the larger value and says so in the region's title.
* `hcp-manager evidence verify` finds all 208 quotations of the pack in the cached documents (user guide, schematic, DXF); the DXF is read as text, so some of its quotations are excerpts of
  the file. The three documents are Espressif's; their disclaimer grants no licence, so they are **not** in this repository, only their URLs, versions and SHA-256.

## What it does not cover

* Hardware revision **v1.1 only** (the initial revision drives its RGB LED from GPIO48 and has another pool) and the **N16R8** variant only. The other variants (WROOM-1U, WROOM-2, the end-of-life
  N8 and N8R2) are other packs; a request for another revision or variant is refused.
* The carrier is a rectangle, the board is placed at 0, 90, 180 or 270 degrees, and the board's rounded corners are not modelled. The height of the parts (and so what may sit under the board) is not modelled.
* The power supply options (5V, 3V3 or USB, mutually exclusive) are described in the user guide but not checked: the power pins are simply not available to the carrier.
* It is a **draft that nobody has reviewed**. Someone with the board in hand should check the pin positions against it and decide the three strapping pins and the antenna clearance.
