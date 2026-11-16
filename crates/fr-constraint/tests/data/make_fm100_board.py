#!/usr/bin/env python3
"""Writes fm100-demo.kicad_pcb: a board with the fictional Acme FM-100 module (see the example pack)
that satisfies every rule of the example. KiCad 8 format. Run: python3 make_fm100_board.py > fm100-demo.kicad_pcb
Tests edit the text of this board to make the mistakes they look for."""
import math

NETS = ["", "GND", "3V3", "EN", "BOOT", "LED_DRIVE", "TXD", "RXD", "USB_DP", "USB_DM"]
nid = {n: i for i, n in enumerate(NETS)}
uuid_n = [0]


def uuid():
    uuid_n[0] += 1
    return f"00000000-0000-0000-0000-{uuid_n[0]:012d}"


def net(n):
    return f'(net {nid[n]} "{n}")' if n else ""


def pad(num, x, y, n="", shape="rect", w=1.0, h=0.6, layers='"F.Cu" "F.Paste" "F.Mask"', kind="smd", rot=0):
    return f'    (pad "{num}" {kind} {shape} (at {x} {y} {rot}) (size {w} {h}) (layers {layers}) {net(n)} (uuid "{uuid()}"))'


def footprint(lib, ref, value, x, y, rot, pads, extra="", side="F.Cu"):
    props = (
        f'    (property "Reference" "{ref}" (at 0 -3 0) (layer "F.SilkS") (uuid "{uuid()}") (effects (font (size 1 1) (thickness 0.15))))\n'
        f'    (property "Value" "{value}" (at 0 3 0) (layer "F.Fab") (uuid "{uuid()}") (effects (font (size 1 1) (thickness 0.15))))\n'
    )
    return f'  (footprint "{lib}" (layer "{side}") (uuid "{uuid()}") (at {x} {y} {rot})\n{props}{extra}' + "\n".join(pads) + "\n  )\n"


# the module: 12 castellated pads, placed in its own frame (6 on each long side), rotated 90 degrees on the board
mod_pads = []
pin_nets = {"1": "GND", "2": "3V3", "3": "EN", "4": "BOOT", "5": "LED_DRIVE", "6": "TXD", "7": "RXD", "8": "USB_DP", "9": "USB_DM", "10": "", "11": "", "12": "GND"}
for i in range(1, 7):
    mod_pads.append(pad(str(i), -6.0, -5.0 + (i - 1) * 2.0, pin_nets[str(i)], w=1.2, h=0.8))
for i in range(7, 13):
    mod_pads.append(pad(str(i), 6.0, 5.0 - (i - 7) * 2.0, pin_nets[str(i)], w=1.2, h=0.8))
out = []
out.append('(kicad_pcb (version 20240108) (generator "pcbnew") (generator_version "8.0")')
out.append('  (general (thickness 1.6) (legacy_teardrops no))')
out.append('  (paper "A4")')
out.append('  (layers (0 "F.Cu" signal) (31 "B.Cu" signal) (36 "B.SilkS" user "B.Silkscreen") (37 "F.SilkS" user "F.Silkscreen") (44 "Edge.Cuts" user) (47 "F.Fab" user))')
out.append('  (setup (pad_to_mask_clearance 0))')
for i, n in enumerate(NETS):
    out.append(f'  (net {i} "{n}")')
for (a, b) in [((0, 0), (60, 0)), ((60, 0), (60, 40)), ((60, 40), (0, 40)), ((0, 40), (0, 0))]:
    out.append(f'  (gr_line (start {a[0]} {a[1]}) (end {b[0]} {b[1]}) (stroke (width 0.1) (type default)) (layer "Edge.Cuts") (uuid "{uuid()}"))')
extra_hcp = f'    (property "HCP_DEVICE" "DEV-FM100" (at 0 0 0) (layer "F.Fab") hide (uuid "{uuid()}") (effects (font (size 1 1) (thickness 0.15))))\n'
out.append(footprint("Acme:FM-100", "U1", "FM-100", 30, 28, 90, mod_pads, extra_hcp))
# supply and reset parts near the module (x 18..26)
out.append(footprint("Capacitor_SMD:C_0805", "C1", "10uF", 20, 18, 0, [pad("1", -1, 0, "3V3"), pad("2", 1, 0, "GND")]))
out.append(footprint("Capacitor_SMD:C_0603", "C2", "100nF", 24, 18, 0, [pad("1", -0.8, 0, "3V3"), pad("2", 0.8, 0, "GND")]))
out.append(footprint("Resistor_SMD:R_0603", "R1", "10k", 20, 24, 0, [pad("1", -0.8, 0, "3V3"), pad("2", 0.8, 0, "EN")]))
out.append(footprint("Capacitor_SMD:C_0603", "C3", "1nF", 24, 24, 0, [pad("1", -0.8, 0, "EN"), pad("2", 0.8, 0, "GND")]))
out.append(footprint("Button_Switch_SMD:SW_SPST", "SW1", "BOOT", 20, 32, 0, [pad("1", -1.5, 0, "BOOT"), pad("2", 1.5, 0, "GND")]))
out.append(footprint("Connector_USB:USB_Micro", "J1", "USB", 52, 36, 0, [pad("1", -1, 0, "USB_DP"), pad("2", 1, 0, "USB_DM")]))
# a mounting hole (a pad without a number and without copper use: np_thru_hole)
out.append(
    f'  (footprint "MountingHole:MountingHole_3.2mm" (layer "F.Cu") (uuid "{uuid()}") (at 3 3)\n'
    f'    (property "Reference" "H1" (at 0 -3 0) (layer "F.SilkS") (uuid "{uuid()}") (effects (font (size 1 1) (thickness 0.15))))\n'
    f'    (property "Value" "MountingHole" (at 0 3 0) (layer "F.Fab") (uuid "{uuid()}") (effects (font (size 1 1) (thickness 0.15))))\n'
    f'    (pad "" np_thru_hole circle (at 0 0) (size 3.2 3.2) (drill 3.2) (layers "*.Cu" "*.Mask") (uuid "{uuid()}"))\n  )\n'
)


def seg(n, layer, w, x1, y1, x2, y2):
    return f'  (segment (start {x1} {y1}) (end {x2} {y2}) (width {w}) (layer "{layer}") (net {nid[n]}) (uuid "{uuid()}"))'


# USB pair to the connector, power and ground tracks; nothing in the antenna keep-out (x 40..60, y 0..10)
out.append(seg("USB_DP", "F.Cu", 0.2, 36.0, 24.0, 36.0, 36.0))
out.append(seg("USB_DP", "F.Cu", 0.2, 36.0, 36.0, 51.0, 36.0))
# the pair is 27.0 mm and 27.2 mm long: within the 0.5 mm skew of the rule
out.append(seg("USB_DM", "F.Cu", 0.2, 34.0, 24.0, 34.0, 38.0))
out.append(seg("USB_DM", "F.Cu", 0.2, 34.0, 38.0, 47.2, 38.0))
out.append(seg("3V3", "F.Cu", 0.4, 19.0, 18.0, 24.8, 18.0))
out.append(seg("GND", "B.Cu", 0.4, 21.0, 18.0, 25.0, 24.0))
out.append(f'  (via (at 25.0 24.0) (size 0.6) (drill 0.3) (layers "F.Cu" "B.Cu") (net {nid["GND"]}) (uuid "{uuid()}"))')
out.append(")")
print("\n".join(out))
