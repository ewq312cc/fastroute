#!/usr/bin/env python3
"""Writes the example carrier board for the ESP32-S3-WROOM-1-N8R8 pack: carrier.kicad_pcb and carrier-notes.md.

    python3 make_carrier.py            (run in this directory; writes the two files next to this script)

The carrier is a made-up board for the example, not a product and not a reference design. Its footprint of the
module is SIMPLIFIED (pad pitch 1.27 mm, the pad numbers and the 18 x 25.5 mm body of the datasheet, the origin at
the centre of the body): use the manufacturer's land pattern for a real board. What the pack takes from it
(outline, mounting holes, the free area) is written in carrier-notes.md, which is the pack's `open_board` source."""
import pathlib

HERE = pathlib.Path(__file__).resolve().parent
NETS = ["", "GND", "3V3", "EN", "BOOT", "UART_TX", "UART_RX", "USB_DM", "USB_DP"]
nid = {n: i for i, n in enumerate(NETS)}
_n = [0]

OUTLINE = [(0, 5.25), (12, 5.25), (12, 12.25), (47, 12.25), (47, 40), (0, 40)]
HOLES = [("H1", 4, 9.25), ("H2", 4, 36), ("H3", 43, 36)]
HOLE_D = 3.2
MODULE_AT = (36.0, 19.0)
USER_AREA = [(7, 16), (21, 16), (21, 34), (7, 34)]


def uuid():
    _n[0] += 1
    return f"00000000-0000-0000-0000-{_n[0]:012d}"


def net(n):
    return f'(net {nid[n]} "{n}")' if n else ""


def pad(num, x, y, n="", w=1.0, h=0.6, kind="smd"):
    layers = '"F.Cu" "F.Paste" "F.Mask"'
    return f'    (pad "{num}" {kind} rect (at {x:.3f} {y:.3f} 0) (size {w} {h}) (layers {layers}) {net(n)} (uuid "{uuid()}"))'


def footprint(lib, ref, value, x, y, pads, extra=""):
    props = (
        f'    (property "Reference" "{ref}" (at 0 -3 0) (layer "F.SilkS") (uuid "{uuid()}") (effects (font (size 1 1) (thickness 0.15))))\n'
        f'    (property "Value" "{value}" (at 0 3 0) (layer "F.Fab") (uuid "{uuid()}") (effects (font (size 1 1) (thickness 0.15))))\n'
    )
    return f'  (footprint "{lib}" (layer "F.Cu") (uuid "{uuid()}") (at {x} {y} 0)\n{props}{extra}' + "\n".join(pads) + "\n  )\n"


# ---- the module: 41 pads in the datasheet's order, in the module's own frame (origin: centre of the body) ----
PIN_NET = {1: "GND", 2: "3V3", 3: "EN", 13: "USB_DM", 14: "USB_DP", 27: "BOOT", 36: "UART_RX", 37: "UART_TX", 40: "GND", 41: "GND"}
mod_pads = []
for i in range(1, 15):  # left side, top to bottom
    mod_pads.append(pad(str(i), -8.75, -5.5 + (i - 1) * 1.27, PIN_NET.get(i, ""), 1.5, 0.9))
for i in range(15, 27):  # bottom, left to right
    mod_pads.append(pad(str(i), -6.985 + (i - 15) * 1.27, 12.0, PIN_NET.get(i, ""), 0.9, 1.5))
for i in range(27, 41):  # right side, bottom to top
    mod_pads.append(pad(str(i), 8.75, 11.01 - (i - 27) * 1.27, PIN_NET.get(i, ""), 1.5, 0.9))
mod_pads.append(pad("41", 0, 3.0, "GND", 3.7, 3.7))


def abs_pin(i):
    for p in mod_pads:
        if p.startswith(f'    (pad "{i}"'):
            at = p.split("(at ")[1].split(")")[0].split()
            return (MODULE_AT[0] + float(at[0]), MODULE_AT[1] + float(at[1]))


out = []
out.append('(kicad_pcb (version 20240108) (generator "pcbnew") (generator_version "8.0")')
out.append("  (general (thickness 1.6) (legacy_teardrops no))")
out.append('  (paper "A4")')
out.append('  (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (36 "B.SilkS" user "B.Silkscreen") (37 "F.SilkS" user "F.Silkscreen") (44 "Edge.Cuts" user) (47 "F.Fab" user))')
out.append("  (setup (pad_to_mask_clearance 0))")
for i, n in enumerate(NETS):
    out.append(f'  (net {i} "{n}")')
for a, b in zip(OUTLINE, OUTLINE[1:] + OUTLINE[:1]):
    out.append(f'  (gr_line (start {a[0]} {a[1]}) (end {b[0]} {b[1]}) (stroke (width 0.1) (type default)) (layer "Edge.Cuts") (uuid "{uuid()}"))')
hcp = f'    (property "HCP_DEVICE" "DEV-ESP32S3-WROOM1-N8R8" (at 0 0 0) (layer "F.Fab") hide (uuid "{uuid()}") (effects (font (size 1 1) (thickness 0.15))))\n'
out.append(footprint("Espressif:ESP32-S3-WROOM-1 (simplified)", "U1", "ESP32-S3-WROOM-1-N8R8", MODULE_AT[0], MODULE_AT[1], mod_pads, hcp))
out.append(footprint("Capacitor_SMD:C_0805", "C1", "22uF", 23.5, 14.0, [pad("1", 0.8, 0, "3V3"), pad("2", -0.8, 0, "GND")]))
out.append(footprint("Capacitor_SMD:C_0603", "C2", "100nF", 23.5, 16.0, [pad("1", 0.8, 0, "3V3"), pad("2", -0.8, 0, "GND")]))
out.append(footprint("Resistor_SMD:R_0603", "R1", "10k", 23.5, 18.0, [pad("1", -0.8, 0, "3V3"), pad("2", 0.8, 0, "EN")]))
out.append(footprint("Capacitor_SMD:C_0603", "C3", "1uF", 23.5, 20.0, [pad("1", -0.8, 0, "EN"), pad("2", 0.8, 0, "GND")]))
out.append(footprint("Resistor_SMD:R_0603", "R3", "10k", 23.5, 22.0, [pad("1", -0.8, 0, "3V3"), pad("2", 0.8, 0, "BOOT")]))
out.append(footprint("Button_Switch_SMD:SW_SPST", "SW1", "BOOT", 24.0, 25.0, [pad("1", -1.5, 0, "BOOT"), pad("2", 1.5, 0, "GND")]))
out.append(footprint("Connector_PinHeader_2.54mm:PinHeader_1x03", "J2", "UART", 24.0, 38.5, [pad("1", -2.0, 0, "UART_TX", 1.2, 1.2), pad("2", 0, 0, "UART_RX", 1.2, 1.2), pad("3", 2.0, 0, "GND", 1.2, 1.2)]))
# the USB pair: both halves leave the module on the left side and are tuned to the same length
dm0, dp0 = abs_pin(13), abs_pin(14)
dm_path = [dm0, (25.0, dm0[1]), (25.0, 36.5), (30.0, 36.5)]
L = sum(abs(b[0] - a[0]) + abs(b[1] - a[1]) for a, b in zip(dm_path, dm_path[1:]))
dp_end_x = 26.0 + (L - abs(dp0[0] - 26.0) - abs(35.0 - dp0[1]))
dp_path = [dp0, (26.0, dp0[1]), (26.0, 35.0), (dp_end_x, 35.0)]
out.append(footprint("Connector_USB:USB_simplified", "J1", "USB", 32.0, 36.0, [pad("1", 30.0 - 32.0, 36.5 - 36.0, "USB_DM", 1.0, 1.0), pad("2", dp_end_x - 32.0, 35.0 - 36.0, "USB_DP", 1.0, 1.0)]))
for ref, x, y in HOLES:
    out.append(
        f'  (footprint "MountingHole:MountingHole_3.2mm" (layer "F.Cu") (uuid "{uuid()}") (at {x} {y})\n'
        f'    (property "Reference" "{ref}" (at 0 -3 0) (layer "F.SilkS") (uuid "{uuid()}") (effects (font (size 1 1) (thickness 0.15))))\n'
        f'    (property "Value" "MountingHole" (at 0 3 0) (layer "F.Fab") (uuid "{uuid()}") (effects (font (size 1 1) (thickness 0.15))))\n'
        f'    (pad "" np_thru_hole circle (at 0 0) (size {HOLE_D} {HOLE_D}) (drill {HOLE_D}) (layers "*.Cu" "*.Mask") (uuid "{uuid()}"))\n  )\n'
    )


def seg(n, layer, w, a, b):
    return f'  (segment (start {a[0]:.3f} {a[1]:.3f}) (end {b[0]:.3f} {b[1]:.3f}) (width {w}) (layer "{layer}") (net {nid[n]}) (uuid "{uuid()}"))'


for a, b in zip(dm_path, dm_path[1:]):
    out.append(seg("USB_DM", "F.Cu", 0.2, a, b))
for a, b in zip(dp_path, dp_path[1:]):
    out.append(seg("USB_DP", "F.Cu", 0.2, a, b))
p2 = abs_pin(2)
out.append(seg("3V3", "F.Cu", 0.5, (24.3, 14.0), (25.0, p2[1])))
out.append(seg("3V3", "F.Cu", 0.5, (25.0, p2[1]), p2))
out.append(seg("GND", "F.Cu", 0.5, (22.7, 14.0), (21.9, 14.0)))
out.append(f'  (via (at 21.9 14.0) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net {nid["GND"]}) (uuid "{uuid()}"))')
out.append(")")
(HERE / "carrier.kicad_pcb").write_text("\n".join(out) + "\n", encoding="utf-8")

pts = ", ".join(f"({x}, {y})" for x, y in OUTLINE)
notes = f"""# Example carrier board for the ESP32-S3-WROOM-1-N8R8 pack (example-1)

This is a made-up board that goes with the example pack. It is not a product and not a reference design, and its
footprint of the module is simplified. The pack takes the facts below from it as an `open_board` source: they describe
this one board and never override the manufacturer's documents.

Coordinates are millimetres in KiCad's frame (x to the right, y down).

## Board
Board outline: {pts}.
The outline has a notch at the top: there is no base board in x 12 to 47 above y = 12.25, so that the antenna end of the
module hangs over the notch.
Module U1 centre at ({MODULE_AT[0]}, {MODULE_AT[1]}), not rotated, antenna end towards the top edge.
""" + "\n".join(f"Mounting hole {r} at ({x}, {y}) mm, diameter {HOLE_D} mm." for r, x, y in HOLES) + f"""

## Free area
Free prototyping area: ({USER_AREA[0][0]}, {USER_AREA[0][1]}) to ({USER_AREA[2][0]}, {USER_AREA[2][1]}) mm, on both layers, nothing placed in it. Module pins IO4 and IO5 (pins 4 and 5) are not connected on this board.
Example block: a status LED D1 with a series resistor R2 of 330 ohm in series with D1.
"""
(HERE / "carrier-notes.md").write_text(notes, encoding="utf-8")
