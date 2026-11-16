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


class ClearanceRulesTest(unittest.TestCase):
    def setUp(self):
        import core

        self.core = core
        core._HANDLED_RULES.clear()
        core._TYPE_DROPPED_RULES.clear()
        core._RULE_NOTES.clear()
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


if __name__ == "__main__":
    unittest.main()
