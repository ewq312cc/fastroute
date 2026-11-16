"""Headless autorouting of a .kicad_pcb file with fastroute.

Run with the Python interpreter that ships with KiCad (it provides `pcbnew`):

    macOS:   /Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/Current/bin/python3
    Linux:   python3 (with the distribution's KiCad Python module installed)
    Windows: "C:\\Program Files\\KiCad\\<ver>\\bin\\python.exe"

    python3 route_cli.py board.kicad_pcb [-o routed.kicad_pcb] [--force] [--clear | --keep-existing]
                         [--mode fast|exact|quick] [--max-passes N] [--threads N] [--max-time SECONDS]
                         [--fastroute PATH] [--json] [--fail-on-partial]
                         [-- extra fastroute args, e.g. --router.autorouter.ignore_net_classes=GUC]

The input board is never overwritten by default: without -o the result is written to
<board>.routed.kicad_pcb next to it, and an output file that already exists is refused (before any
work is done) unless --force is given. Writing over the input needs -o <the input> and --force.

Exit codes: 0 done, 1 routing failed, 2 the board cannot be used, 4 the output is refused or cannot
be written, 5 (--fail-on-partial only) the result is written but connections remain unrouted.
"""

import argparse
import json
import os
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import pcbnew  # noqa: E402

import core  # noqa: E402


def default_output(board):
    """Where the result goes when -o is not given: next to the input, never the input itself."""
    return board.with_name(board.stem + ".routed.kicad_pcb")


def check_output(out, force):
    """Why `out` may not be written, or None. Checked before any work is done."""
    if out.is_dir():
        return f"{out}: is a directory"
    if out.exists() and not force:
        return f"{out}: already exists (use --force to replace it; the input board is never overwritten by default)"
    if not out.parent.is_dir():
        return f"{out}: the directory {out.parent} does not exist"
    return None


def save_board(board, out):
    """Saves through a temporary file, so that a failure never leaves half a board at `out`."""
    tmp = out.with_name("." + out.name + ".tmp.kicad_pcb")
    try:
        pcbnew.SaveBoard(str(tmp), board)
        os.replace(tmp, out)
    finally:
        if tmp.exists():
            tmp.unlink()


def main(argv=None):
    ap = argparse.ArgumentParser(description="Autoroute a KiCad board with fastroute")
    ap.add_argument("board", type=Path)
    ap.add_argument(
        "-o", "--output", type=Path,
        help="output board (default: <board>.routed.kicad_pcb next to the input; the input is never "
        "the default)",
    )
    ap.add_argument("--force", action="store_true", help="replace an existing output file")
    ap.add_argument(
        "--json", action="store_true",
        help="print one JSON summary line on standard output (the human messages go to standard error)",
    )
    ap.add_argument(
        "--fail-on-partial", action="store_true",
        help="exit with 5 if connections remain unrouted (the result is written anyway)",
    )
    existing = ap.add_mutually_exclusive_group()
    existing.add_argument("--clear", action="store_true", help="remove unlocked tracks and vias first")
    existing.add_argument(
        "--keep-existing",
        action="store_true",
        help="keep all existing tracks and vias unchanged, route only the open connections "
        "(by default unlocked tracks may be moved or rerouted)",
    )
    ap.add_argument("--mode", choices=("fast", "exact", "quick"), default="fast")
    ap.add_argument("--max-passes", type=int, default=0)
    ap.add_argument("--threads", type=int, default=0)
    ap.add_argument(
        "--neckdown", choices=("on", "off"), default="on", help="narrow traces at pins"
    )
    ap.add_argument(
        "--ignore-min-width",
        action="store_true",
        help="allow neck-down below the board's minimum track width",
    )
    ap.add_argument("--no-refill", action="store_true", help="do not refill zones afterwards")
    ap.add_argument(
        "--zones-as-planes",
        action="store_true",
        help="treat zone nets as connected by the zone (Freerouting's behaviour) instead of routing them",
    )
    ap.add_argument(
        "--no-text-keepouts", action="store_true", help="do not keep traces away from copper texts"
    )
    ap.add_argument(
        "--max-time", type=float, help="stop after this many seconds and keep the best result"
    )
    ap.add_argument(
        "--clearance-margin-um", type=float, default=5.0,
        help="added to every clearance for the router (KiCad measures exact circles; default 5)",
    )
    ap.add_argument(
        "--obstacle-zones", default="",
        help="comma separated net names or net classes (* and ? allowed) whose zones stay fixed "
        "copper that other nets must not cross (e.g. a power pour on an outer layer)",
    )
    ap.add_argument("--fastroute", type=Path, help="path to the fastroute executable")
    ap.add_argument("--work-dir", type=Path, help="keep the DSN/SES files in this directory")
    ap.add_argument(
        "--export-only",
        action="store_true",
        help="only write the prepared DSN (and tune/pairs files) to --work-dir and "
        "the fastroute arguments to args.txt there; do not route",
    )
    ap.add_argument(
        "--no-keep-stitching", action="store_true",
        help="do not keep zone stitching vias (vias without a track); by default they are "
        "locked for the run so the router neither removes nor moves them",
    )
    ap.add_argument(
        "--live", action="store_true",
        help="watch the routing live in the web browser (fastroute --live)",
    )
    ap.add_argument("-q", "--quiet", action="store_true")
    # Everything after "--" goes to fastroute unchanged (argparse cannot mix a positional
    # argument with a trailing "--" list).
    argv = list(sys.argv[1:] if argv is None else argv)
    extra = []
    if "--" in argv:
        i = argv.index("--")
        argv, extra = argv[:i], argv[i + 1:]
    args = ap.parse_args(argv)
    args.extra = extra
    out = args.output or default_output(args.board)
    started = time.monotonic()
    # (with --json the human text goes to standard error, so that standard output is one JSON line)
    say = (lambda *a, **k: print(*a, file=sys.stderr, **k)) if args.json else print

    def finish(code, status, error=None, **fields):
        if args.json:
            summary = {"status": status, "exit_code": code, "input": str(args.board), "output": str(out) if status != "failed" else None,
                       "error": error, "seconds": round(time.monotonic() - started, 3)}
            summary.update(fields)
            print(json.dumps(summary))
        if error:
            print(f"error: {error}", file=sys.stderr)
        return code

    if not args.board.is_file():
        return finish(2, "failed", f"{args.board}: no such file")
    problem = None if args.export_only else check_output(out, args.force)
    if problem:
        return finish(4, "failed", problem)
    board = pcbnew.LoadBoard(str(args.board))
    if board is None or not core.has_board_outline(board):
        return finish(2, "failed", f"{args.board}: not a board with an outline on Edge.Cuts")
    if args.work_dir:
        args.work_dir.mkdir(parents=True, exist_ok=True)

    min_width = 0 if args.ignore_min_width else core.min_track_width_nm(board)
    neckdown = args.neckdown == "on"
    router = core.Router(
        board,
        binary=args.fastroute,
        extra_args=core.mode_args(
            args.mode, args.max_passes, args.threads, neckdown, min_width,
            core.copper_edge_clearance_nm(board),
        )
        + args.extra,
        work_dir=args.work_dir,
        refill=not args.no_refill,
        route_zone_nets=not args.zones_as_planes,
        text_keepouts=not args.no_text_keepouts,
        clear_tracks=args.clear,
        keep_existing=args.keep_existing,
        max_time=args.max_time,
        clearance_margin_um=args.clearance_margin_um,
        obstacle_zones=[p.strip() for p in args.obstacle_zones.split(",")],
        live=args.live,
        keep_stitching=not args.no_keep_stitching,
    )

    def show(level, text):
        if not args.quiet or level in ("WARN", "ERROR"):
            print(f"{level:5} {text}", file=sys.stderr)

    if args.export_only:
        if not args.work_dir:
            return finish(2, "failed", "--export-only needs --work-dir")
        failed = router.prepare()
        if failed is not None:
            return finish(1, "failed", failed.message)
        for w in getattr(router, "warnings", []):
            print(f"WARN  .kicad_dru {w}", file=sys.stderr)
        # the arguments after the binary, one per line (paths as written in work_dir)
        (args.work_dir / "args.txt").write_text("\n".join(router.command()[1:]) + "\n")
        say(f"exported to {args.work_dir}")
        return finish(0, "exported", work_dir=str(args.work_dir))

    t = time.monotonic()
    result = router.run(show)
    if not result.ok:
        return finish(1, "failed", result.message)
    try:
        save_board(board, out)
    except Exception as e:  # pcbnew raises RuntimeError / IOError, depending on the version
        return finish(4, "failed", f"{out}: cannot write the board: {e}")
    if result.partial:
        print(f"warning: {result.message.splitlines()[0]}", file=sys.stderr)
    for w in result.warnings:
        if not w.startswith(".kicad_dru"):  # (those were shown before routing)
            print(f"warning: {w}", file=sys.stderr)
    if result.unrouted_ignored:
        print(f"{result.unrouted_ignored} of the unrouted connections are in ignored net classes", file=sys.stderr)
    say(
        f"routed in {time.monotonic() - t:.1f} s: score {result.score}, "
        f"{result.unrouted} unrouted, {result.violations} violations"
        + (f" ({result.violations_unfixable} pre-existing, unfixable)" if result.violations_unfixable else "")
        + (f", KiCad: {result.kicad_unconnected} open connections" if result.kicad_unconnected is not None else "")
        + f" -> {out}"
    )
    # "complete" only if the router says nothing is left; an unknown count is not a claim of success
    complete = result.unrouted == 0
    return finish(
        5 if (not complete and args.fail_on_partial) else 0,
        "complete" if complete else "partial",
        unrouted=result.unrouted,
        violations=result.violations,
        violations_unfixable=result.violations_unfixable,
        kicad_unconnected=result.kicad_unconnected,
        score=result.score,
    )


if __name__ == "__main__":
    sys.exit(main())
