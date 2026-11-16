"""`.kicad_dru` clearance rules reaching the router: core.add_dru_class_clearances.

Runs without KiCad (a stand-in `pcbnew`) on a DSN shaped like KiCad's own export; the same rules
were also run through a real KiCad 8.0.9 and its DRC (see docs/CLI.md, "Checked against a real
KiCad"). Run with `python3 -m unittest discover -s integrations/kicad/tests -p 'test_*.py'`.
"""

import re
import sys
import tempfile
import types
import unittest
from pathlib import Path

PLUGINS = Path(__file__).resolve().parents[1] / "plugins"
_saved = {}

DSN = """(pcb board.dsn
  (parser (host_cad "KiCad's Pcbnew"))
  (resolution um 10)
  (unit um)
  (network
    (net "GND" (pins U1-1 U2-1))
    (net "/VPP{slash}MCLR" (pins U1-2 U2-2))
    (net "SIG" (pins U1-3 U2-3))
    (net "Net-(D1-K)" (pins U1-4 U2-4))
    (net "HV+" (pins J1-1 J2-1))
    (class kicad_default "GND" "/VPP{slash}MCLR" "SIG" "Net-(D1-K)"
      (circuit
        (use_via "Via[0-1]_600:300_um")
      )
      (rule
        (width 200)
        (clearance 200)
      )
    )
    (class HV "HV+"
      (circuit
        (use_via "Via[0-1]_800:400_um")
      )
      (rule
        (width 300)
        (clearance 400)
      )
    )
  )
)
"""

NETS = {"GND": "Default", "/VPP{slash}MCLR": "Default", "SIG": "Default", "Net-(D1-K)": "Default", "HV+": "HV"}


def setUpModule():
    for name in ("pcbnew", "core"):
        _saved[name] = sys.modules.pop(name, None)
    sys.modules["pcbnew"] = types.ModuleType("pcbnew")
    sys.path.insert(0, str(PLUGINS))


def tearDownModule():
    sys.path.remove(str(PLUGINS))
    for name, mod in _saved.items():
        sys.modules.pop(name, None)
        if mod is not None:
            sys.modules[name] = mod


class FakeNet:
    def __init__(self, cls):
        self.cls = cls

    def GetNetClassName(self):
        return self.cls


class FakeBoard:
    def __init__(self, path, nets=NETS):
        self.path, self.nets = path, nets

    def GetFileName(self):
        return str(self.path)

    def GetNetsByName(self):
        return {name: FakeNet(cls) for name, cls in self.nets.items()}

    # the board's own layer names differ from the canonical ones (as in KiCad's demo boards)
    LAYERS = {"F.Cu": (0, "top_layer"), "B.Cu": (31, "bottom_layer")}

    def GetLayerID(self, name):
        return self.LAYERS.get(name, (-1, ""))[0]

    def GetLayerName(self, lid):
        return next(n for i, n in self.LAYERS.values() if i == lid)


class ClearanceRulesTest(unittest.TestCase):
    def setUp(self):
        import core

        self.core = core
        core._HANDLED_RULES.clear()
        core._TYPE_DROPPED_RULES.clear()
        core._RULE_NOTES.clear()
        core._DERIVED.clear()
        core._NO_NECKDOWN.clear()
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self.tmp.name)
        self.board = FakeBoard(self.dir / "b.kicad_pcb")
        self.dsn = self.dir / "b.dsn"
        self.dsn.write_text(DSN)

    def tearDown(self):
        self.tmp.cleanup()

    def translate(self, rules, ignored=()):
        (self.dir / "b.kicad_dru").write_text("(version 1)\n" + rules)
        carried = self.core.add_dru_class_clearances(self.board, self.dsn, ignored)
        text = self.dsn.read_text()
        pairs = {tuple(sorted(m.group(1).split())): float(m.group(2)) for m in re.finditer(r"\(class_class \(classes ([^)]*)\) \(rule \(clearance ([\d.]+)\)\)\)", text)}
        return carried, pairs, text

    def warnings(self):
        return self.core.dru_warnings(self.board, ())

    # ---- no condition: every pair of classes ------------------------------------------------

    def test_a_rule_without_a_condition_applies_to_every_pair_of_classes(self):
        carried, pairs, _ = self.translate('(rule "all" (constraint clearance (min 0.3mm)))\n')
        self.assertEqual(carried, [("all", ["every pair of net classes"])])
        # the two classes of different clearance (200 / 400) keep the larger one in the mixed pair
        # (the router never goes below it); each class with itself gets the rule's value
        self.assertEqual(pairs[("HV", "kicad_default")], 400.0)
        self.assertEqual(pairs[("kicad_default", "kicad_default")], 300.0)
        self.assertFalse([w for w in self.warnings() if "NOT applied" in w], self.warnings())

    def test_a_value_above_every_class_clearance_wins_everywhere(self):
        _, pairs, _ = self.translate('(rule "big" (constraint clearance (min 0.5mm)))\n')
        self.assertEqual(set(pairs.values()), {500.0})
        self.assertEqual(len(pairs), 3)  # (default, default) (default, HV) (HV, HV)

    def test_lowering_a_class_clearance_is_said(self):
        _, pairs, _ = self.translate('(rule "low" (constraint clearance (min 0.1mm)))\n')
        self.assertEqual(pairs[("kicad_default", "kicad_default")], 100.0)
        notes = [w for w in self.warnings() if w.startswith("rule 'low'")]
        self.assertTrue(any("lowers the clearance" in w and "the router follows" in w for w in notes), notes)

    def test_a_layer_restriction_is_applied_to_all_layers_and_said(self):
        _, pairs, _ = self.translate('(rule "top" (layer "F.Cu") (constraint clearance (min 0.3mm)))\n')
        self.assertEqual(pairs[("kicad_default", "kicad_default")], 300.0)
        self.assertTrue(any("all layers" in w and "stricter" in w for w in self.warnings()), self.warnings())

    # ---- net conditions: a class of their own ---------------------------------------------------

    def test_nets_of_a_rule_get_a_class_of_their_own_with_the_old_circuit_and_rule(self):
        carried, pairs, text = self.translate(
            """(rule "R1" (condition "A.NetName == '/VPP{slash}MCLR' || A.NetName == 'Net-(D1-K)'") (constraint clearance (min 0.45mm)))\n"""
        )
        self.assertEqual(carried, [("R1", ["fr_dru_R1"])])
        classes = {b["name"]: b for b in self.core._dsn_class_blocks(text)}
        self.assertEqual(classes["fr_dru_R1"]["nets"], ["/VPP{slash}MCLR", "Net-(D1-K)"])
        self.assertEqual(classes["kicad_default"]["nets"], ["GND", "SIG"])
        for name in ("fr_dru_R1", "kicad_default"):  # same circuit, width and own clearance
            self.assertIn('(use_via "Via[0-1]_600:300_um")', classes[name]["body"])
            self.assertIn("(width 200)", classes[name]["body"])
            self.assertIn("(clearance 200)", classes[name]["body"])
        # with every class (itself included) at the rule's value, and the rest as before
        self.assertEqual(pairs[("fr_dru_R1", "kicad_default")], 450.0)
        self.assertEqual(pairs[("fr_dru_R1", "fr_dru_R1")], 450.0)
        self.assertEqual(pairs[("HV", "fr_dru_R1")], 450.0)
        self.assertNotIn(("kicad_default", "kicad_default"), pairs)
        self.assertFalse([w for w in self.warnings() if "NOT applied" in w], self.warnings())

    def test_a_net_that_is_in_the_condition_twice_or_in_two_rules_stays_routable(self):
        carried, pairs, text = self.translate(
            """(rule "A" (condition "A.NetName == 'GND'") (constraint clearance (min 0.3mm)))
(rule "B" (condition "A.NetName == 'GND' || A.NetName == 'SIG'") (constraint clearance (min 0.5mm)))\n"""
        )
        names = {b["name"]: b["nets"] for b in self.core._dsn_class_blocks(text)}
        # GND is in the class of rule A or of rule B, never in two; SIG is left in the rest
        holders = [n for n, nets in names.items() if "GND" in nets]
        self.assertEqual(len(holders), 1, names)
        self.assertEqual({r for r, _ in carried}, {"A", "B"})
        self.assertTrue(all(c > 0 for c in pairs.values()))

    def test_a_net_class_term_selects_the_nets_of_that_class_by_name(self):
        carried, pairs, text = self.translate(
            """(rule "mix" (condition "A.NetClass == 'Default' && A.NetName == 'SIG'") (constraint clearance (min 0.35mm)))\n"""
        )
        names = {b["name"]: b["nets"] for b in self.core._dsn_class_blocks(text)}
        self.assertEqual(names["fr_dru_mix"], ["SIG"])
        self.assertEqual(pairs[("fr_dru_mix", "kicad_default")], 350.0)

    def test_a_class_rule_still_applies_to_nets_that_were_moved_out_of_the_class(self):
        # R1 moves SIG out of kicad_default; the rule of the class (0.6 mm against HV) must
        # still cover SIG, which is still of the class Default in KiCad
        carried, pairs, _ = self.translate(
            """(rule "R1" (condition "A.NetName == 'SIG'") (constraint clearance (min 0.3mm)))
(rule "cls" (condition "A.NetClass == 'Default' && B.NetClass == 'HV'") (constraint clearance (min 0.6mm)))\n"""
        )
        self.assertEqual(pairs[("HV", "kicad_default")], 600.0)
        self.assertEqual(pairs[("HV", "fr_dru_R1")], 600.0)

    def test_nets_of_a_class_the_autorouter_ignores_stay_and_are_said(self):
        carried, pairs, text = self.translate(
            """(rule "R1" (condition "A.NetName == 'SIG'") (constraint clearance (min 0.3mm)))\n""", ignored={"Default"}
        )
        names = {b["name"]: b["nets"] for b in self.core._dsn_class_blocks(text)}
        self.assertIn("SIG", names["kicad_default"])
        self.assertNotIn("fr_dru_R1", names)
        self.assertTrue(any("ignored by the autorouter" in w for w in self.warnings()), self.warnings())

    def test_a_condition_that_matches_no_net_is_said_not_silently_dropped(self):
        carried, pairs, text = self.translate("""(rule "typo" (condition "A.NetName == 'SGI'") (constraint clearance (min 0.3mm)))\n""")
        # (only the pair of classes of different clearance that is always written, as before)
        self.assertEqual((carried, pairs), ([], {("HV", "kicad_default"): 400.0}))
        self.assertTrue(any("matches no net" in w for w in self.warnings()), self.warnings())
        self.assertNotIn("fr_dru_", text, "no class was made")

    def test_wildcards_of_kicad_work_in_net_names(self):
        _, _, text = self.translate("""(rule "w" (condition "A.NetName == 'Net-(D1-*'") (constraint clearance (min 0.3mm)))\n""")
        names = {b["name"]: b["nets"] for b in self.core._dsn_class_blocks(text)}
        self.assertEqual(names["fr_dru_w"], ["Net-(D1-K)"])

    # ---- what stays unsupported is still reported ----------------------------------------------

    def test_conditions_the_router_cannot_use_are_not_applied_and_reported(self):
        carried, pairs, text = self.translate(
            """(rule "T" (condition "A.Type == 'Track' && A.NetName == 'GND'") (constraint clearance (min 0.3mm)))
(rule "B" (condition "A.NetName == 'GND' && B.NetName == 'SIG'") (constraint clearance (min 0.3mm)))
(rule "Z" (condition "A.insideArea('zone')") (constraint clearance (min 0.3mm)))\n"""
        )
        self.assertEqual((carried, pairs), ([], {("HV", "kicad_default"): 400.0}))
        self.assertNotIn("fr_dru_", text, "no class was made")
        warned = [w for w in self.warnings() if "NOT applied" in w]
        self.assertEqual(len(warned), 3, self.warnings())

    # ---- the class-pair form that existed before keeps working ---------------------------------

    def test_class_pair_rules_are_as_before(self):
        carried, pairs, _ = self.translate(
            """(rule "hv" (condition "A.NetClass == 'HV' && B.NetClass != 'HV'") (constraint clearance (min 0.6mm)))\n"""
        )
        self.assertEqual(carried, [("hv", ["kicad_default"])])
        self.assertEqual(pairs[("HV", "kicad_default")], 600.0)

    def test_the_dsn_stays_parseable_and_the_nets_stay_in_exactly_one_class(self):
        _, _, text = self.translate(
            """(rule "R1" (condition "A.NetName == 'GND' || A.NetName == 'HV+'") (constraint clearance (min 0.3mm)))\n"""
        )
        held = [n for b in self.core._dsn_class_blocks(text) for n in b["nets"]]
        self.assertEqual(sorted(held), sorted(NETS), "every net is in exactly one class")
        depth = 0
        for ch in re.sub(r'"[^"]*"', "", text):
            depth += (ch == "(") - (ch == ")")
            self.assertGreaterEqual(depth, 0)
        self.assertEqual(depth, 0, "balanced parentheses")


class NetWidthRulesTest(unittest.TestCase):
    """`track_width` rules for single nets (and net classes without a layer): core.add_dru_net_widths."""

    def setUp(self):
        import core

        self.core = core
        core._HANDLED_RULES.clear()
        core._TYPE_DROPPED_RULES.clear()
        core._RULE_NOTES.clear()
        core._DERIVED.clear()
        core._NO_NECKDOWN.clear()
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self.tmp.name)
        self.board = FakeBoard(self.dir / "b.kicad_pcb")
        self.dsn = self.dir / "b.dsn"
        self.dsn.write_text(DSN)

    def tearDown(self):
        self.tmp.cleanup()

    def widths(self, rules, ignored=(), layer_widths=()):
        (self.dir / "b.kicad_dru").write_text("(version 1)\n" + rules)
        done = self.core.add_dru_net_widths(self.board, self.dsn, ignored, layer_widths)
        return done, {b["name"]: b for b in self.core._dsn_class_blocks(self.dsn.read_text())}

    @staticmethod
    def width_of(block):
        return float(re.search(r"\(rule\s*\(width\s+([\d.]+)\)", block["body"]).group(1))

    def warnings(self):
        return self.core.dru_warnings(self.board, ())

    def test_the_nets_of_a_rule_get_a_class_with_the_width_and_the_rest_as_before(self):
        done, classes = self.widths(
            """(rule "W1" (condition "A.NetName == 'SIG' || A.NetName == 'GND'") (constraint track_width (min 0.6mm) (opt 0.6mm)))\n"""
        )
        self.assertEqual(done, [("W1", ["fr_dru_w_W1"], 0.6, None)])
        self.assertEqual(classes["fr_dru_w_W1"]["nets"], ["GND", "SIG"])
        self.assertEqual(classes["kicad_default"]["nets"], ["/VPP{slash}MCLR", "Net-(D1-K)"])
        self.assertEqual(self.width_of(classes["fr_dru_w_W1"]), 600.0)
        self.assertEqual(self.width_of(classes["kicad_default"]), 200.0)
        self.assertEqual(self.width_of(classes["HV"]), 300.0)
        for name in ("fr_dru_w_W1", "kicad_default"):  # circuit and clearance are the old ones
            self.assertIn('(use_via "Via[0-1]_600:300_um")', classes[name]["body"])
            self.assertIn("(clearance 200)", classes[name]["body"])
        self.assertEqual(self.core._NO_NECKDOWN, {"fr_dru_w_W1"})
        self.assertFalse([w for w in self.warnings() if "NOT applied" in w], self.warnings())

    def test_the_width_is_opt_else_the_middle_of_min_and_max_else_min(self):
        done, classes = self.widths(
            """(rule "a" (condition "A.NetName == 'GND'") (constraint track_width (min 0.3mm) (opt 0.5mm) (max 0.9mm)))
(rule "b" (condition "A.NetName == 'SIG'") (constraint track_width (min 0.4mm) (max 0.6mm)))
(rule "c" (condition "A.NetName == 'Net-(D1-K)'") (constraint track_width (min 10mil)))\n"""
        )
        self.assertEqual([round(w, 4) for _, _, w, _ in done], [0.5, 0.5, 0.254])
        self.assertEqual(sorted(self.width_of(b) for n, b in classes.items() if n.startswith("fr_dru_w_")), [254.0, 500.0, 500.0])
        self.assertTrue(any("middle of min and max" in w for w in self.warnings() if w.startswith("rule 'b'")), self.warnings())

    def test_a_rule_that_gives_no_width_is_left_to_the_warnings(self):
        done, classes = self.widths("""(rule "m" (condition "A.NetName == 'GND'") (constraint track_width (max 0.5mm)))\n""")
        self.assertEqual(done, [])
        self.assertEqual(set(classes), {"kicad_default", "HV"})

    def test_a_layer_restriction_sets_only_that_layers_width_with_the_boards_layer_name(self):
        done, classes = self.widths(
            """(rule "L" (layer "F.Cu") (condition "A.NetName == 'SIG'") (constraint track_width (min 0.5mm) (opt 0.5mm)))\n"""
        )
        new = classes["fr_dru_w_L"]
        self.assertEqual(self.width_of(new), 200.0, "the width on the other layers is unchanged")
        self.assertIn("(layer_rule top_layer (rule (width 500)))", new["body"])
        self.assertEqual(done[0][3], "F.Cu")

    def test_without_a_condition_every_class_gets_the_width_in_place(self):
        done, classes = self.widths("""(rule "all" (constraint track_width (min 0.3mm)))\n""")
        self.assertEqual(set(classes), {"kicad_default", "HV"}, "no new class")
        self.assertEqual({self.width_of(b) for b in classes.values()}, {300.0})
        self.assertEqual(classes["kicad_default"]["nets"], ["GND", "/VPP{slash}MCLR", "SIG", "Net-(D1-K)"])

    def test_a_rule_that_names_a_class_sets_the_class_in_place(self):
        done, classes = self.widths("""(rule "hv" (condition "A.NetClass == 'HV'") (constraint track_width (min 0.8mm) (opt 0.8mm)))\n""")
        self.assertEqual(set(classes), {"kicad_default", "HV"})
        self.assertEqual((self.width_of(classes["HV"]), self.width_of(classes["kicad_default"])), (800.0, 200.0))
        self.assertIn("HV", self.core._NO_NECKDOWN)

    def test_a_later_rule_wins_for_a_net_and_it_is_in_one_class_only(self):
        done, classes = self.widths(
            """(rule "first" (condition "A.NetName == 'SIG'") (constraint track_width (min 0.4mm) (opt 0.4mm)))
(rule "second" (condition "A.NetName == 'SIG'") (constraint track_width (min 0.7mm) (opt 0.7mm)))\n"""
        )
        holders = [b for b in classes.values() if "SIG" in b["nets"]]
        self.assertEqual(len(holders), 1)
        self.assertEqual(self.width_of(holders[0]), 700.0)

    def test_a_net_rule_overrides_the_layer_widths_of_its_class(self):
        # (what add_dru_layer_widths wrote, e.g. controlled impedance, is a layer_rule of the class)
        self.dsn.write_text(DSN.replace("      (rule\n        (width 200)\n        (clearance 200)\n      )\n", "      (rule\n        (width 200)\n        (clearance 200)\n      )\n      (layer_rule top_layer (rule (width 250)))\n", 1))
        done, classes = self.widths("""(rule "w" (condition "A.NetName == 'SIG'") (constraint track_width (min 0.6mm) (opt 0.6mm)))\n""")
        self.assertNotIn("layer_rule", classes["fr_dru_w_w"]["body"])
        self.assertIn("(layer_rule top_layer (rule (width 250)))", classes["kicad_default"]["body"], "the other nets keep it")

    def test_rules_that_add_dru_layer_widths_carries_are_not_done_twice(self):
        done, classes = self.widths(
            """(rule "i" (layer "F.Cu") (condition "A.NetClass == 'Default'") (constraint track_width (min 0.3mm) (opt 0.3mm)))\n""",
            layer_widths=[("Default", "F.Cu", 0.3)],
        )
        self.assertEqual(done, [])
        self.assertEqual(self.dsn.read_text(), DSN)

    def test_nets_of_an_ignored_class_stay_and_it_is_said(self):
        done, classes = self.widths(
            """(rule "w" (condition "A.NetName == 'SIG'") (constraint track_width (min 0.6mm) (opt 0.6mm)))\n""", ignored={"Default"}
        )
        self.assertEqual(done, [])
        self.assertIn("SIG", classes["kicad_default"]["nets"])
        self.assertTrue(any("ignored by the autorouter" in w for w in self.warnings()), self.warnings())

    def test_unusable_conditions_stay_not_applied(self):
        done, classes = self.widths(
            """(rule "t" (condition "A.Type == 'Track' && A.NetName == 'GND'") (constraint track_width (min 0.5mm)))
(rule "b" (condition "A.NetName == 'GND' && B.NetName == 'SIG'") (constraint track_width (min 0.5mm)))\n"""
        )
        self.assertEqual(done, [])
        self.assertEqual(self.dsn.read_text(), DSN)
        self.assertEqual(len([w for w in self.warnings() if "NOT applied" in w]), 2, self.warnings())

    def test_a_condition_that_matches_no_net_is_said(self):
        done, classes = self.widths("""(rule "x" (condition "A.NetName == 'nope'") (constraint track_width (min 0.5mm)))\n""")
        self.assertEqual(done, [])
        self.assertTrue(any("matches no net" in w for w in self.warnings()), self.warnings())

    # ---- together with clearance rules ------------------------------------------------------------

    def test_a_net_with_a_width_rule_and_a_clearance_rule_keeps_both_and_the_rules_of_its_class(self):
        (self.dir / "b.kicad_dru").write_text(
            """(version 1)
(rule "W" (condition "A.NetName == 'SIG'") (constraint track_width (min 0.6mm) (opt 0.6mm)))
(rule "C" (condition "A.NetName == 'SIG'") (constraint clearance (min 0.45mm)))
(rule "cls" (condition "A.NetClass == 'Default' && B.NetClass == 'HV'") (constraint clearance (min 0.7mm)))
"""
        )
        self.core.add_dru_net_widths(self.board, self.dsn, (), ())
        carried = self.core.add_dru_class_clearances(self.board, self.dsn, ())
        text = self.dsn.read_text()
        classes = {b["name"]: b for b in self.core._dsn_class_blocks(text)}
        holder = next(n for n, b in classes.items() if "SIG" in b["nets"])
        self.assertEqual(holder, "fr_dru_C", "the clearance class was made out of the width class")
        self.assertEqual(float(re.search(r"\(rule\s*\(width\s+([\d.]+)\)", classes[holder]["body"]).group(1)), 600.0)
        pairs = {tuple(sorted(m.group(1).split())): float(m.group(2)) for m in re.finditer(r"\(class_class \(classes ([^)]*)\) \(rule \(clearance ([\d.]+)\)\)\)", text)}
        self.assertEqual(pairs[("HV", "fr_dru_C")], 700.0, "the rule of the class Default still covers SIG")
        self.assertEqual(pairs[("fr_dru_C", "fr_dru_C")], 450.0)
        self.assertEqual({r for r, _ in carried}, {"C", "cls"})
        # the net moved a second time (by the clearance rule): its class has the width rule, so no
        # neck-down there either; the class it left is empty and gone
        self.assertEqual(self.core._NO_NECKDOWN, {"fr_dru_C"})
        self.assertNotIn("fr_dru_w_W", text)
        self.assertTrue(all(b["nets"] for b in classes.values() if b["name"].startswith("fr_dru_")), "no empty class left")

    # ---- ignored net classes: an exact mirror of the router ------------------------------------

    def test_the_router_compares_class_names_ignoring_case_and_calls_the_default_class_default(self):
        names = self.core._ignored_names([" Power ", "", "DEFAULT"])
        self.assertEqual(names, {"power", "default"})
        for cls, ignored in (("POWER", True), ("power", True), ("Default", True), ("kicad_default", True), ("HV", False), ("fr_dru_x", False)):
            self.assertEqual(self.core.class_is_ignored(cls, names), ignored, cls)
        # (the router has no class that is called kicad_default, so naming it ignores nothing)
        self.assertFalse(self.core.class_is_ignored("kicad_default", self.core._ignored_names(["kicad_default"])))
        self.assertFalse(self.core.class_is_ignored("HV", self.core._ignored_names([])))

    def test_width_and_clearance_rules_leave_the_nets_of_an_ignored_class_alone_whatever_its_case(self):
        for spelled in ("HV", "hv", "Hv"):
            self.dsn.write_text(DSN)
            self.core._HANDLED_RULES.clear()
            self.core._RULE_NOTES.clear()
            self.core._DERIVED.clear()
            self.core._NO_NECKDOWN.clear()
            (self.dir / "b.kicad_dru").write_text(
                """(version 1)
(rule "W" (condition "A.NetName == 'HV+' || A.NetName == 'SIG'") (constraint track_width (min 0.6mm)))
(rule "C" (condition "A.NetName == 'HV+' || A.NetName == 'SIG'") (constraint clearance (min 0.5mm)))
"""
            )
            self.core.add_dru_net_widths(self.board, self.dsn, {spelled}, ())
            self.core.add_dru_class_clearances(self.board, self.dsn, {spelled})
            classes = {b["name"]: b for b in self.core._dsn_class_blocks(self.dsn.read_text())}
            self.assertEqual(classes["HV"]["nets"], ["HV+"], f"HV+ stays in its ignored class ({spelled})")
            self.assertEqual(self.width_of(classes["HV"]), 300.0)
            holder = next(n for n, b in classes.items() if "SIG" in b["nets"])
            self.assertTrue(holder.startswith("fr_dru_"), "SIG is not in an ignored class: the rules apply")
            self.assertEqual(sum(1 for b in classes.values() for n in b["nets"] if n == "HV+"), 1)
            warned = [w for w in self.warnings() if "ignored by the autorouter" in w]
            self.assertEqual(len(warned), 2, (spelled, self.warnings()))

    def test_the_default_class_is_ignored_as_default_in_any_case(self):
        for spelled in ("Default", "default", "DEFAULT"):
            self.dsn.write_text(DSN)
            self.core._HANDLED_RULES.clear()
            self.core._RULE_NOTES.clear()
            self.core._DERIVED.clear()
            self.core._NO_NECKDOWN.clear()
            (self.dir / "b.kicad_dru").write_text(
                """(version 1)
(rule "all" (constraint track_width (min 0.6mm)))
(rule "one" (condition "A.NetName == 'SIG' || A.NetName == 'HV+'") (constraint clearance (min 0.5mm)))
"""
            )
            self.core.add_dru_net_widths(self.board, self.dsn, {spelled}, ())
            self.core.add_dru_class_clearances(self.board, self.dsn, {spelled})
            classes = {b["name"]: b for b in self.core._dsn_class_blocks(self.dsn.read_text())}
            self.assertEqual(self.width_of(classes["kicad_default"]), 200.0, f"the ignored default class keeps its width ({spelled})")
            self.assertEqual(self.width_of(classes["HV"]), 600.0, "the other class gets the rule for every class")
            self.assertIn("SIG", classes["kicad_default"]["nets"])
            self.assertEqual(sum(1 for b in classes.values() for n in b["nets"] if n == "SIG"), 1)

    def test_a_class_rule_does_not_touch_an_ignored_class_either(self):
        (self.dir / "b.kicad_dru").write_text("""(version 1)\n(rule "hv" (condition "A.NetClass == 'HV'") (constraint track_width (min 0.8mm)))\n""")
        done = self.core.add_dru_net_widths(self.board, self.dsn, {"hv"}, ())
        self.assertEqual(done, [])
        self.assertEqual(self.dsn.read_text(), DSN)
        self.assertTrue(any("ignored by the autorouter" in w for w in self.warnings()), self.warnings())

    def test_the_dsn_stays_balanced_and_every_net_in_one_class(self):
        self.widths(
            """(rule "a" (condition "A.NetName == 'SIG' || A.NetName == 'HV+'") (constraint track_width (min 0.6mm)))
(rule "b" (layer "B.Cu") (condition "A.NetName == 'GND'") (constraint track_width (min 0.5mm)))\n"""
        )
        text = self.dsn.read_text()
        held = [n for b in self.core._dsn_class_blocks(text) for n in b["nets"]]
        self.assertEqual(sorted(held), sorted(NETS))
        depth = 0
        for ch in re.sub(r'"[^"]*"', "", text):
            depth += (ch == "(") - (ch == ")")
            self.assertGreaterEqual(depth, 0)
        self.assertEqual(depth, 0)


if __name__ == "__main__":
    unittest.main()
