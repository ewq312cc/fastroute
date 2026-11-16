"""Tests of route_cli.py's output policy: the input board is never overwritten by default, an
existing output needs --force, and --json / --fail-on-partial / the exit codes work.

KiCad is not needed: a stand-in `pcbnew` module and a stand-in router replace the real ones, so
this runs with a plain Python 3:

    python3 -m unittest discover -s integrations/kicad/tests -p 'test_*.py'
"""

import io
import json
import sys
import tempfile
import types
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

PLUGINS = Path(__file__).resolve().parents[1] / "plugins"
ORIGINAL = "(kicad_pcb (version 20240108) (generator original))\n"
ROUTED = "(kicad_pcb (version 20240108) (generator routed))\n"


class State:
    """What the stand-ins do in the current test."""

    def __init__(self):
        self.has_outline = True
        self.save_fails = False
        self.router_result = {"ok": True, "unrouted": 0, "violations": 0, "score": 12.5}
        self.router_calls = 0
        self.saved = []


STATE = State()
_saved_modules = {}


def setUpModule():
    for name in ("pcbnew", "core", "route_cli"):
        _saved_modules[name] = sys.modules.pop(name, None)
    pcbnew = types.ModuleType("pcbnew")

    def load_board(path):
        return types.SimpleNamespace(path=path)

    def save_board(path, board):
        if STATE.save_fails:
            raise RuntimeError("disk full")
        Path(path).write_text(ROUTED)
        STATE.saved.append(path)

    pcbnew.LoadBoard = load_board
    pcbnew.SaveBoard = save_board
    sys.modules["pcbnew"] = pcbnew
    sys.path.insert(0, str(PLUGINS))
    import core
    import route_cli

    core.has_board_outline = lambda board: STATE.has_outline
    core.min_track_width_nm = lambda board: 0
    core.copper_edge_clearance_nm = lambda board: 0

    class FakeRouter:
        def __init__(self, board, **kw):
            self.kw = kw

        def prepare(self):
            return None

        def command(self):
            return ["fastroute", "-de", "x.dsn"]

        def run(self, show):
            STATE.router_calls += 1
            r = core.RouteResult()
            for k, v in STATE.router_result.items():
                setattr(r, k, v)
            r.message = r.message or ("routing failed" if not r.ok else "")
            return r

    core.Router = FakeRouter
    route_cli.core = core


def tearDownModule():
    sys.path.remove(str(PLUGINS))
    for name, mod in _saved_modules.items():
        sys.modules.pop(name, None)
        if mod is not None:
            sys.modules[name] = mod


class RouteCliTest(unittest.TestCase):
    def setUp(self):
        global STATE
        STATE.__init__()
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self.tmp.name)
        self.board = self.dir / "board.kicad_pcb"
        self.board.write_text(ORIGINAL)

    def tearDown(self):
        self.tmp.cleanup()

    def cli(self, *args):
        import route_cli

        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            try:
                code = route_cli.main([str(self.board), *map(str, args)])
            except SystemExit as e:  # argparse
                code = e.code
        return code, out.getvalue(), err.getvalue()

    # ---- the default output is never the input ------------------------------------------------

    def test_without_o_the_result_goes_next_to_the_input_and_the_input_is_untouched(self):
        code, out, err = self.cli("--clear", "-q")
        self.assertEqual(code, 0, err)
        routed = self.dir / "board.routed.kicad_pcb"
        self.assertEqual(routed.read_text(), ROUTED)
        self.assertEqual(self.board.read_text(), ORIGINAL, "the input board changed")
        self.assertIn("board.routed.kicad_pcb", out)
        self.assertEqual(sorted(p.name for p in self.dir.iterdir()), ["board.kicad_pcb", "board.routed.kicad_pcb"], "temporary files left behind")

    def test_default_name_keeps_the_stem(self):
        import route_cli

        self.assertEqual(route_cli.default_output(Path("/a/b/my board.kicad_pcb")), Path("/a/b/my board.routed.kicad_pcb"))
        self.assertEqual(route_cli.default_output(Path("x.routed.kicad_pcb")), Path("x.routed.routed.kicad_pcb"))

    # ---- an output that exists is refused before any work is done -----------------------------

    def test_an_existing_output_is_refused_without_force_before_routing(self):
        existing = self.dir / "board.routed.kicad_pcb"
        existing.write_text("precious\n")
        code, out, err = self.cli()
        self.assertEqual(code, 4)
        self.assertIn("already exists", err)
        self.assertIn("--force", err)
        self.assertEqual(existing.read_text(), "precious\n")
        self.assertEqual(STATE.router_calls, 0, "the long run must not start when the output is refused")
        # explicit -o too
        mine = self.dir / "mine.kicad_pcb"
        mine.write_text("mine\n")
        code, out, err = self.cli("-o", mine)
        self.assertEqual(code, 4)
        self.assertEqual(mine.read_text(), "mine\n")

    def test_force_replaces_the_output(self):
        existing = self.dir / "board.routed.kicad_pcb"
        existing.write_text("old\n")
        code, out, err = self.cli("--force", "-q")
        self.assertEqual(code, 0, err)
        self.assertEqual(existing.read_text(), ROUTED)
        self.assertEqual(self.board.read_text(), ORIGINAL)

    def test_writing_over_the_input_needs_both_o_and_force(self):
        code, out, err = self.cli("-o", self.board)
        self.assertEqual(code, 4, err)
        self.assertEqual(self.board.read_text(), ORIGINAL)
        code, out, err = self.cli("-o", self.board, "--force", "-q")
        self.assertEqual(code, 0, err)
        self.assertEqual(self.board.read_text(), ROUTED)

    def test_missing_directory_and_directory_as_output(self):
        code, out, err = self.cli("-o", self.dir / "nope" / "x.kicad_pcb")
        self.assertEqual(code, 4)
        self.assertIn("does not exist", err)
        code, out, err = self.cli("-o", self.dir, "--force")
        self.assertEqual(code, 4)
        self.assertIn("is a directory", err)
        self.assertEqual(STATE.router_calls, 0)

    def test_a_failing_save_leaves_neither_a_partial_file_nor_a_changed_old_one(self):
        STATE.save_fails = True
        existing = self.dir / "board.routed.kicad_pcb"
        existing.write_text("old\n")
        code, out, err = self.cli("--force", "-q")
        self.assertEqual(code, 4)
        self.assertIn("cannot write the board", err)
        self.assertEqual(existing.read_text(), "old\n")
        self.assertEqual(sorted(p.name for p in self.dir.iterdir()), ["board.kicad_pcb", "board.routed.kicad_pcb"])

    # ---- outcomes and exit codes --------------------------------------------------------------

    def test_a_board_that_cannot_be_used_is_exit_2_and_nothing_is_written(self):
        STATE.has_outline = False
        code, out, err = self.cli()
        self.assertEqual(code, 2)
        self.assertIn("Edge.Cuts", err)
        self.assertEqual(sorted(p.name for p in self.dir.iterdir()), ["board.kicad_pcb"])
        self.board.unlink()
        code, out, err = self.cli()
        self.assertEqual(code, 2)
        self.assertIn("no such file", err)

    def test_a_failed_route_is_exit_1_and_writes_nothing(self):
        STATE.router_result = {"ok": False, "message": "fastroute crashed"}
        code, out, err = self.cli()
        self.assertEqual(code, 1)
        self.assertIn("fastroute crashed", err)
        self.assertEqual(sorted(p.name for p in self.dir.iterdir()), ["board.kicad_pcb"])

    def test_partial_results_are_written_and_exit_0_unless_asked(self):
        STATE.router_result = {"ok": True, "unrouted": 3, "violations": 1}
        code, out, err = self.cli("-q")
        self.assertEqual(code, 0, err)
        self.assertTrue((self.dir / "board.routed.kicad_pcb").exists())
        (self.dir / "board.routed.kicad_pcb").unlink()
        code, out, err = self.cli("-q", "--fail-on-partial")
        self.assertEqual(code, 5, err)
        self.assertEqual((self.dir / "board.routed.kicad_pcb").read_text(), ROUTED, "the partial result is written anyway")

    def test_export_only_writes_no_board_and_is_not_blocked_by_an_existing_output(self):
        (self.dir / "board.routed.kicad_pcb").write_text("keep\n")
        code, out, err = self.cli("--export-only", "--work-dir", self.dir / "work")
        self.assertEqual(code, 0, err)
        self.assertEqual((self.dir / "board.routed.kicad_pcb").read_text(), "keep\n")

    # ---- structured output --------------------------------------------------------------------

    def test_json_is_one_line_on_stdout_and_the_text_goes_to_stderr(self):
        STATE.router_result = {"ok": True, "unrouted": 0, "violations": 2, "violations_unfixable": 1, "kicad_unconnected": 0, "score": 9.0}
        code, out, err = self.cli("--json")
        self.assertEqual(code, 0, err)
        lines = out.strip().splitlines()
        self.assertEqual(len(lines), 1, out)
        s = json.loads(lines[0])
        self.assertEqual(s["status"], "complete")
        self.assertEqual((s["unrouted"], s["violations"], s["violations_unfixable"], s["kicad_unconnected"]), (0, 2, 1, 0))
        self.assertEqual(s["output"], str(self.dir / "board.routed.kicad_pcb"))
        self.assertEqual(s["input"], str(self.board))
        self.assertEqual(s["exit_code"], 0)
        self.assertIsNone(s["error"])
        self.assertIn("routed in", err, "the human summary is on stderr")

    def test_json_for_partial_failed_and_refused(self):
        STATE.router_result = {"ok": True, "unrouted": 4, "violations": 0}
        code, out, err = self.cli("--json", "--fail-on-partial")
        s = json.loads(out.strip().splitlines()[-1])
        self.assertEqual((code, s["status"], s["exit_code"], s["unrouted"]), (5, "partial", 5, 4))
        # an unknown count is not reported as complete
        (self.dir / "board.routed.kicad_pcb").unlink()
        STATE.router_result = {"ok": True, "unrouted": None, "violations": None}
        code, out, err = self.cli("--json")
        s = json.loads(out.strip().splitlines()[-1])
        self.assertEqual((s["status"], s["unrouted"]), ("partial", None))
        # failure
        STATE.router_result = {"ok": False, "message": "boom"}
        code, out, err = self.cli("--json", "--force")
        s = json.loads(out.strip().splitlines()[-1])
        self.assertEqual((code, s["status"], s["error"], s["output"]), (1, "failed", "boom", None))
        # refused output
        code, out, err = self.cli("--json")
        s = json.loads(out.strip().splitlines()[-1])
        self.assertEqual((code, s["status"], s["exit_code"]), (4, "failed", 4))
        self.assertIn("already exists", s["error"])

    def test_without_json_standard_output_is_the_human_summary(self):
        code, out, err = self.cli()
        self.assertEqual(code, 0, err)
        self.assertIn("routed in", out)
        self.assertNotIn('"status"', out)

    def test_help_mentions_the_safe_defaults(self):
        code, out, err = self.cli("--help")
        self.assertEqual(code, 0)
        for text in ("--force", "--json", "--fail-on-partial", "routed.kicad_pcb"):
            self.assertIn(text, out)


if __name__ == "__main__":
    unittest.main()
