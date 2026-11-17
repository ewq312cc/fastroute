#!/usr/bin/env python3
"""Builds the board pack of the ESP32-S3-DevKitC-1 v1.1 (N16R8) from the locked documents of hardware-packs/espressif/workspace/.

    python3 hardware-packs/tools/build_devkitc.py

* The header tables (which pin is where, with its name and function) are read out of the cached user guide (HTML), not typed.
* The geometry (outline, the 2 x 22 header pad centres, the antenna overhang, the USB connector overhang) is measured in the cached
  DXF ("Dimensions source file") by dxf_geometry.py. The silkscreen labels in the DXF are compared with the names in the user guide's
  tables, so a header that is in the wrong place or wrongly numbered stops the build.
* The module the board carries (ESP32-S3-WROOM-1-N16R8) is the upstream pack espressif/esp32-s3/esp32-s3-wroom-1/n16r8: the board says
  which of its header pins are wired to which module pin, and the pack check refuses a pin that the board calls usable but the module
  pack protects (strapping, reset, boot, reserved).
* What the documents do not say stays `unknown` with the reason. Nothing is typed from memory: every quotation is looked up in its document
  before the pack is written (page of a PDF, text of an HTML page, text of the DXF).
"""
import html
import pathlib
import re
import sys

import yaml

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import build_espressif as be  # noqa: E402
from dxf_geometry import Dxf  # noqa: E402

BOARD = "espressif/esp32-s3/esp32-s3-devkitc-1/n16r8-v1.1"
MODULE = be.S3_MOD16
DEV = "DEV-DEVKITC1-N16R8-V11"
MOD_DEV = "DEV-ESP32S3-WROOM1-N16R8"
PART = "ESP32-S3-DevKitC-1-N16R8"
REVISION = "v1.1"

UG_ID, SCH_ID, DXF_ID = "DOC-DEVKITC1-UG-V11", "DOC-DEVKITC1-SCH-V11", "DOC-DEVKITC1-DXF-V11"
S_UG, S_SCH, S_DXF = "SRC-DK-UG", "SRC-DK-SCH", "SRC-DK-DXF"
DOCS = [UG_ID, SCH_ID, DXF_ID]


def cells(row):
    return [" ".join(html.unescape(re.sub(r"<[^>]+>", " ", c)).split()) for c in re.findall(r"<t[dh].*?</t[dh]>", row, flags=re.S)]


def header_tables(ug):
    """{'J1': [(no, name, type, function), ...], 'J3': [...]} from the two header tables of the user guide."""
    text = pathlib.Path(ug.path).read_text(encoding="utf-8")
    tables = [t for t in re.findall(r"<table.*?</table>", text, flags=re.S) if "I/O/T" in t and "No." in t]
    out = {}
    for name, tb in zip(("J1", "J3"), tables):
        rows = [cells(r) for r in re.findall(r"<tr.*?</tr>", tb, flags=re.S)]
        body = [r for r in rows if len(r) == 4 and r[0].isdigit()]
        if [int(r[0]) for r in body] != list(range(1, 23)):
            sys.exit(f"the table {name} of the user guide does not list pins 1 to 22: {[r[0] for r in body]}")
        out[name] = [tuple(r) for r in body]
    if set(out) != {"J1", "J3"}:
        sys.exit("the user guide has no two header tables with 22 rows")
    return out


def main():
    ug, sch, dx = be.doc(UG_ID), be.doc(SCH_ID), be.doc(DXF_ID)
    geo = Dxf(dx.path)
    tables = header_tables(ug)

    # ---- the module the board carries: its pin names -> numbers -----------------------------------------------------------------
    mod = yaml.safe_load((be.pack_dir(MODULE) / "hcp.yaml").read_text(encoding="utf-8"))
    mod_pins = {p["name"]: p["number"] for d in mod["devices"] for p in d["pins"]}

    # ---- geometry: header pad centres, checked against the silkscreen labels and the guide ------------------------------------
    x0, y0, x1, y1 = geo.outline()
    pads = {}
    for px, py in geo.header_pads():
        side = "J1" if abs(px - 1.27) < 0.02 else "J3" if abs(px - (x1 - 1.27)) < 0.02 else None
        if side:
            pads.setdefault(side, []).append((px, py))
    for side in pads:
        pads[side] = sorted(pads[side], key=lambda p: -p[1])
        if len(pads[side]) != 22:
            sys.exit(f"{side}: the DXF has {len(pads[side])} header pads, the guide has 22")
        ys = [p[1] for p in pads[side]]
        if any(abs((a - b) - 2.54) > 0.01 for a, b in zip(ys, ys[1:])):
            sys.exit(f"{side}: the pads are not at a 2.54 mm pitch")
    labels = [t for t in geo.texts("LAYNR3")]
    for side, col in (("J1", "left"), ("J3", "right")):
        for (no, name, _t, _f), (px, py) in zip(tables[side], pads[side]):
            near = [t for t in labels if abs(t["y"] - (py - 1.6)) < 0.5 and ((t["x"] < 3.0) if col == "left" else (t["x"] > 22.0))]   # silkscreen next to the header, not component references
            if not near:
                sys.exit(f"{side}.{no}: no silkscreen label next to the pad at {px}, {py}")
            label = min(near, key=lambda t: abs(t["y"] - (py - 1.6)))["text"]
            if label != name:
                sys.exit(f"{side}.{no}: the user guide says `{name}` but the silkscreen of the DXF at that pad says `{label}`")
    print("  header tables of the guide and silkscreen labels of the DXF agree for all 44 pins")

    ant = geo.rect("TOP", 15)
    if ant is None or abs((ant[2] - ant[0]) - 18.0) > 0.05:
        sys.exit("the antenna outline (layer TOP, 18 mm wide) is not in the DXF")
    ant_nth = next(i for i, e in enumerate(geo.polylines("TOP")) if geo.rect("TOP", 15) and abs(min(p[0] for p in geo.verts(e)) - ant[0]) < 1e-6 and len(geo.verts(e)) >= 4 and max(p[0] for p in geo.verts(e)) - min(p[0] for p in geo.verts(e)) > 15)

    def usb_overhang(xa, xb):
        """(lowest y, x0, x1, the entity) of the connector drawing (layer 2DLINES_1) below the bottom edge of the board, in the band xa..xb."""
        pts = [(p, e) for e in geo.polylines("2DLINES_1") for p in geo.verts(e) if xa < p[0] < xb and p[1] < -0.5]
        if not pts:
            sys.exit("no USB connector overhang found in the DXF")
        return min(p[1] for p, _ in pts), min(p[0] for p, _ in pts), max(p[0] for p, _ in pts), pts[0][1]

    j2_low, j2_x0, j2_x1, j2_e = usb_overhang(2, 10)
    j4_low, j4_x0, j4_x1, j4_e = usb_overhang(15, 23)

    def entity_quote(e, through_y):
        """The raw excerpt of one polyline entity, from its layer line to the vertex that has `through_y`."""
        data = e["data"]
        start = next(i for i, (c, v) in enumerate(data) if c == "8")
        out, last, n = [], None, 0
        for c, v in data[start:]:
            out.append(f"{c} {v}")
            if c == "20" and abs(float(v) - through_y) < 1e-9:
                return " ".join(out)
        sys.exit("the vertex is not in the entity")

    # ---- the guide's statements ------------------------------------------------------------------------------------------------
    S_ALL = ug.sent("All available GPIO pins")          # "All available GPIO pins (except for the SPI bus for flash) are broken out ..."
    S_MOST = ug.sent("Most of the I/O pins on the module")
    S_NOTE = ug.sent("For boards with Octal SPI flash/PSRAM memory")
    S_RESET = "Press this button to restart the system."
    S_BOOT = "Holding down Boot and then pressing Reset initiates Firmware Download mode for downloading firmware through the serial port."
    S_UART = ug.sent("A Micro-USB port used for power supply to the board")
    S_USB = ug.sent("ESP32-S3 full-speed USB OTG interface")
    S_RGB = "Addressable RGB LED, driven by GPIO38."
    S_LDO = "Power regulator that converts a 5 V supply into a 3.3 V output."
    S_LED = "Turns on when the USB power is connected to the board."
    S_BRIDGE = "Single USB-to-UART bridge chip provides transfer rates up to 3 Mbps."
    S_REV = ug.sent("Both the initial and v1.1 versions")
    S_ORDER = "ESP32-S3-DevKitC-1-N16R8 ESP32-S3-WROOM-1 16 MB 8 MB PCB on-board antenna"   # the row of the ordering table: this variant, its module, its memory

    def uev(q, section=None, **kw):
        # every locator names a section of the user guide (an HTML page has no page numbers)
        if section is None:
            section = "Hardware Reference, Header Block" if kw.get("table") else "Getting Started, Description of Components"
        return ug.ev(S_UG, q, section=section, **kw)

    def row_quote(side, no):
        q = " ".join(tables[side][no - 1])
        # hcp-manager joins a word that is hyphenated at the end of a line when it normalises text, so a quotation must not end in `-`:
        # the last function name (`USB_D-`) is left out of the quotation of that row
        return q[: q.rindex(", ")] if q.endswith("-") else q

    # ---- the board device and its header pins -----------------------------------------------------------------------------------
    CLASS_OF = {"3V3": "power", "5V": "power", "G": "ground", "RST": "reset", "TX": "uart", "RX": "uart", "0": "strapping", "3": "strapping", "45": "strapping", "46": "strapping",
                "19": "usb", "20": "usb", "35": "reserved", "36": "reserved", "37": "reserved"}
    KIND_OF = {"3V3": "power", "5V": "power", "G": "ground", "RST": "reset", "TX": "uart", "RX": "uart", "0": "boot", "19": "usb", "20": "usb",
               "39": "jtag", "40": "jtag", "41": "jtag", "42": "jtag"}
    PINNAME = lambda n: {"G": "GND"}.get(n, n if not n.isdigit() else f"GPIO{n}")  # noqa: E731

    def module_pin(name):
        if name in ("3V3",):
            return mod_pins["3V3"]
        if name == "RST":
            return mod_pins["EN"]
        if name == "TX":
            return mod_pins["TXD0"]
        if name == "RX":
            return mod_pins["RXD0"]
        if name.isdigit():
            return mod_pins[f"IO{name}"]
        return None

    dev_pins, header_pins = [], {"J1": [], "J3": []}
    usable = []
    for side in ("J1", "J3"):
        for (no, name, typ, fn), (px, py) in zip(tables[side], pads[side]):
            n = int(no)
            pname = PINNAME(name)
            dev_pins.append({"number": f"{side}.{no}", "name": pname, "class": CLASS_OF.get(name, "gpio"),
                             "functions": [f.strip() for f in fn.split(",")] if name not in ("G",) else ["GND"],
                             "evidence": [uev(row_quote(side, n), table=f"Header Block, {side}")], "note": f"silkscreen {name}; position from the DXF"})
            pin = {"number": no, "name": pname, "position": [round(px, 3), round(py, 3)], "kind": KIND_OF.get(name, "gpio")}
            mp = module_pin(name)
            if mp:
                pin["module_pin"] = mp
            row_ev = uev(row_quote(side, n), table=f"Header Block, {side}")
            if name in ("3V3", "5V"):
                pin.update(usability="reserved", reason="a power pin of the board (guide: Power Supply Options: 5V and G, 3V3 and G, or USB; mutually exclusive): not a signal pin, in no pool",
                           evidence=[row_ev, uev("There are three mutually exclusive ways to provide power to the board:", section="Hardware Reference, Power Supply Options")])
            elif name == "G":
                pin.update(usability="reserved", reason="ground: not a signal pin, in no pool", evidence=[row_ev])
            elif name == "RST":
                pin.update(usability="onboard", reason="EN (CHIP_PU): the Reset button and its RC circuit are on the board; the pin is the board's reset line, not a signal pin",
                           evidence=[row_ev, uev("Reset Button"), uev(S_RESET)])
            elif name == "0":
                pin.update(usability="onboard", reason="GPIO0 is the Boot button of the board and a strapping pin (module pack rule C-ESP-STRAP-IO0, boot rule C-ESP-BOOT-IO0)",
                           evidence=[row_ev, uev("Boot Button"), uev(S_BOOT)])
            elif name in ("TX", "RX"):
                pin.update(usability="onboard", reason="U0TXD / U0RXD (UART0): the same nets as the on-board USB-to-UART bridge, which the board uses to flash and talk to the chip (user guide; schematic sheet 2, read from the drawing)",
                           evidence=[row_ev, uev(S_UART), sch.ev(S_SCH, "CP2102N-A02-GQFN28", page=2, figure="USB to UART (sheet 2)", basis="observed")])
            elif name in ("19", "20"):
                pin.update(usability="onboard", reason="USB D- (GPIO19) / D+ (GPIO20): wired to the board's second Micro-USB connector, the ESP32-S3 USB port (user guide; schematic sheet 2 `ESP USB`)",
                           evidence=[row_ev, uev(S_USB), sch.ev(S_SCH, "USB:", page=2, figure="ESP USB (sheet 2)", basis="observed")])
            elif name == "38":
                pin.update(usability="onboard", reason="drives the addressable RGB LED of board revision v1.1 (the initial revision used GPIO48)", evidence=[row_ev, uev(S_RGB), uev(S_REV, section="Hardware Revision Details")])
            elif name in ("35", "36", "37"):
                pin.update(usability="reserved", reason="GPIO35, GPIO36 and GPIO37 are used for the internal communication with the Octal SPI PSRAM of the module (N16R8): not available for external use",
                           evidence=[row_ev, uev(S_NOTE), uev(S_ORDER, section="Contents and Packaging, Ordering Information")])
            elif name in ("3", "45", "46"):
                pin.update(usability="unknown", reason=f"GPIO{name} is a strapping pin of the module (module pack rule C-ESP-STRAP-IO{name}); the board documents list it on the header and do not say that it is free for external circuits: nothing is assumed, and the pin is in no pool",
                           evidence=[row_ev])
            else:
                pin.update(usability="usable",
                           evidence=[row_ev, uev(S_ALL, section="Getting Started, Description of Components (Pin Headers)")])
                usable.append(f"{side}.{no}")
            header_pins[side].append(pin)

    # ---- blocks ----------------------------------------------------------------------------------------------------------------
    blocks = [
        {"id": "BLK-DK-RESET", "title": "Reset button and RC circuit on EN (on the board)", "kind": "reset_circuit", "modifiability": "fixed", "device": DEV, "evidence": [uev("Reset Button"), uev(S_RESET)]},
        {"id": "BLK-DK-BOOT", "title": "Boot button on GPIO0 (on the board)", "kind": "boot_button", "modifiability": "fixed", "device": DEV, "evidence": [uev("Boot Button"), uev(S_BOOT)]},
        {"id": "BLK-DK-USB-UART", "title": "Micro-USB port with USB-to-UART bridge on UART0 (on the board)", "kind": "programming_interface", "modifiability": "fixed", "device": DEV,
         "evidence": [uev(S_UART), uev(S_BRIDGE)]},
        {"id": "BLK-DK-USB", "title": "ESP32-S3 USB port on GPIO19 and GPIO20 (on the board)", "kind": "usb", "modifiability": "fixed", "device": DEV, "evidence": [uev(S_USB)]},
        {"id": "BLK-DK-POWER", "title": "5 V to 3.3 V regulator and power-on LED (on the board)", "kind": "power_decoupling", "modifiability": "fixed", "device": DEV, "evidence": [uev(S_LDO), uev(S_LED)]},
        {"id": "BLK-DK-RGB", "title": "Addressable RGB LED on GPIO38 (on the board)", "kind": "generic", "modifiability": "fixed", "device": DEV, "evidence": [uev(S_RGB)]},
        {"id": "BLK-DK-PERIPHERAL", "title": "A peripheral on the carrier (sensor, LED, button, connector) wired to usable header pins", "kind": "generic", "modifiability": "ai_editable",
         "components": [{"role": "peripheral", "kind": "ic", "min_count": 1}], "evidence": [uev(S_MOST, section="ESP32-S3-DevKitC-1 v1.1 (introduction)")]},
    ]
    onboard = ["BLK-DK-RESET", "BLK-DK-BOOT", "BLK-DK-USB-UART", "BLK-DK-USB", "BLK-DK-POWER", "BLK-DK-RGB"]

    # ---- rules and regions ------------------------------------------------------------------------------------------------------
    rules = [
        be.rule("C-DK-RSVD-PSRAM", "reserved_pin", "fixed", "GPIO35, GPIO36 and GPIO37 (J3.13, J3.12, J3.11) are used for the Octal SPI flash/PSRAM of the module and are not available for external use",
                {"device": DEV}, {"pins": ["J3.11", "J3.12", "J3.13"], "reason": "used for the internal communication between the ESP32-S3 and the Octal SPI memory of the module"},
                [uev(S_NOTE), uev(S_ORDER, section="Contents and Packaging, Ordering Information")]),
        be.rule("C-DK-ANT-CLEARANCE", "keepout", "unknown", "The RF clearance a carrier must keep around the board's PCB antenna",
                {"region": "REG-DK-ANT"}, {"forbid": ["track", "via", "component", "zone"]},
                missing="The DXF gives the antenna part of the module (18 x 6.3 mm, beyond the board's top edge) and that is the only zone this pack forbids. The user guide of the board says nothing about the "
                        "clearance a carrier must keep beside or under it; the module's guideline (cut the base board away on both sides of the antenna and below it) is written for a module soldered to a base "
                        "board, not for a board plugged into a carrier, and this pack does not apply it by guess. A person has to decide."),
    ]
    hdr_ev = lambda q, nth=0, **kw: dx.ev(S_DXF, q, basis="observed", **kw)  # noqa: E731
    ant_poly = [[round(ant[0], 3), round(ant[1], 3)], [round(ant[2], 3), round(ant[1], 3)], [round(ant[2], 3), round(ant[3], 3)], [round(ant[0], 3), round(ant[3], 3)]]
    length_text = "62.74mm"
    regions = [
        {"id": "REG-DK-OUTLINE", "kind": "board_outline", "unit": "mm", "frame": "device", "device": DEV, "modifiability": "fixed",
         "title": f"Outline of the board, the bounding rectangle of the BOARD_OUTLINE layer of the DXF: {x1 - x0:g} x {y1 - y0:g} mm (corners are rounded, which this pack does not model). "
                  f"The drawing's own dimension texts say 25.40mm and {length_text}; the outline layer measures {y1 - y0:g} mm in length, 0.125 mm more, and the larger value is used",
         "polygon": [[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
         "evidence": [hdr_ev(geo.quote("BOARD_OUTLINE", 0), section="DXF layer BOARD_OUTLINE"), hdr_ev(geo.text_quote("25.40mm"), section="DXF dimension text"), hdr_ev(geo.text_quote(length_text), section="DXF dimension text")]},
        {"id": "REG-DK-ANT", "kind": "antenna_keepout", "unit": "mm", "frame": "device", "device": DEV, "modifiability": "fixed",
         "title": "The antenna part of the module (PCB antenna, `MINI-ANT-TYPEB` in the DXF): 18 x 6.3 mm that stick out beyond the top edge of the board; nothing of a carrier may be here. The wider RF clearance is unknown (C-DK-ANT-CLEARANCE)",
         "polygon": ant_poly,
         "evidence": [hdr_ev(geo.text_quote("MINI-ANT-TYPEB"), section="DXF text MINI-ANT-TYPEB"), hdr_ev(entity_quote(geo.polylines("TOP")[ant_nth], ant[3]), section="DXF layer TOP, antenna outline")]},
        {"id": "REG-DK-USB-J2", "kind": "connector", "unit": "mm", "frame": "device", "device": DEV, "modifiability": "fixed",
         "title": f"Micro-USB connector J2 (USB-to-UART port): the {abs(j2_low):.3f} mm of its shell that stick out beyond the bottom edge of the board",
         "polygon": [[round(j2_x0, 3), round(j2_low, 3)], [round(j2_x1, 3), round(j2_low, 3)], [round(j2_x1, 3), 0.0], [round(j2_x0, 3), 0.0]],
         "evidence": [hdr_ev(entity_quote(j2_e, j2_low), section="DXF layer 2DLINES_1, connector drawing")]},
        {"id": "REG-DK-USB-J4", "kind": "connector", "unit": "mm", "frame": "device", "device": DEV, "modifiability": "fixed",
         "title": f"Micro-USB connector J4 (ESP32-S3 USB port): the {abs(j4_low):.3f} mm of its shell that stick out beyond the bottom edge of the board",
         "polygon": [[round(j4_x0, 3), round(j4_low, 3)], [round(j4_x1, 3), round(j4_low, 3)], [round(j4_x1, 3), 0.0], [round(j4_x0, 3), 0.0]],
         "evidence": [hdr_ev(entity_quote(j4_e, j4_low), section="DXF layer 2DLINES_1, connector drawing")]},
    ]

    ids = {"hdr": usable}
    hcp = {
        "hcp_version": "1.1",
        "pack": {
            "id": BOARD, "kind": "board", "title": "ESP32-S3-DevKitC-1 v1.1 (ESP32-S3-WROOM-1-N16R8): board pack", "version": be.VERSION, "language": "en", "created": be.TODAY,
            "extractor": {"kind": "mixed", "tool": "hardware-packs/tools/build_devkitc.py: header tables read from the user guide, geometry measured in the DXF, usability decided by an AI assistant from the locked documents; every quotation is checked against its document"},
            "review": {"status": "draft", "note": "Draft: nobody has reviewed this pack. It is for the hardware revision v1.1 only (the initial revision drives its RGB LED from GPIO48) and for the N16R8 variant (16 MB quad flash, 8 MB octal PSRAM); other variants and the "
                                                   "WROOM-1U / WROOM-2 boards are other packs. The wider RF clearance around the antenna, the strapping pins GPIO3, GPIO45 and GPIO46, and the real height of the parts are `unknown` or not modelled."},
            "upstream": [{"id": MODULE, "version": be.VERSION, "kind": "module", "canonical_sha256": be.digest(MODULE),
                          "inherit": {"devices": [MOD_DEV], "rules": ["C-ESP-RESERVED-PSRAM", "C-ESP-EN", "C-ESP-STRAP-IO0", "C-ESP-STRAP-IO46", "C-ESP-STRAP-IO45", "C-ESP-STRAP-IO3", "C-ESP-BOOT-IO0"]}}],
        },
        "sources": [be.source(UG_ID, S_UG, [DEV]), be.source(SCH_ID, S_SCH, [DEV]), be.source(DXF_ID, S_DXF, [DEV])],
        "devices": [{"id": DEV, "vendor": "Espressif", "part_number": PART, "revision": REVISION, "package": "development board, 2 x 22 pin headers, ESP32-S3-WROOM-1-N16R8", "pins": dev_pins}],
        "peripheral_blocks": blocks,
        "rules": rules,
        "regions": regions,
        "board_integration": {
            "hardware_revision": REVISION, "board_device": DEV, "upstream_device": MOD_DEV,
            "frame": {"unit": "mm", "origin": "the lower-left corner of the board outline, top view (the side with the module and the silkscreen)",
                      "orientation": "x to the right, y up; the antenna end is at +y. J1 is the left header, J3 the right one; pin 1 of both is at the antenna end"},
            "exposed_headers": [{"id": "J1", "pitch_mm": 2.54, "pins": header_pins["J1"]}, {"id": "J3", "pitch_mm": 2.54, "pins": header_pins["J3"]}],
            "onboard_blocks": onboard,
            "carrier": {"allowed_blocks": ["BLK-DK-PERIPHERAL"], "allowed_ops": ["add_component", "connect", "add_track"], "margin_mm": 0},
        },
    }
    be.write_pack(BOARD, "board", PART, hcp, [(MODULE, "module")], DOCS, package="development board", revision=REVISION)
    print(f"  usable header pins ({len(usable)}): {', '.join(usable)}")
    print("  pack written to", be.pack_dir(BOARD))


if __name__ == "__main__":
    main()
