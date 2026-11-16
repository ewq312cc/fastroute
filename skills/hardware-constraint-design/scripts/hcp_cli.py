#!/usr/bin/env python3
"""The front door of the hardware-constraint-design Skill: one command line over two programs.

    fastroute constraints ...   schema, check, compile, report, validate, export, check-edit
                                (deterministic, offline: no AI model, network or vector index)
    hcp-manager ...             documents, download and cache, keyword search, evidence check, packs
                                (the independent application; this script implements none of it)

The script finds the programs, negotiates their versions, forwards the command, and turns the ways
things go wrong (a program missing or too old, no inventory, no index, offline) into messages that say
what to do next. It reads and writes nothing of its own: no cache, no index, no state. Python 3
standard library only.

Exit codes (the programs' own, passed through): 0 ok, 1 a violation / failure, 2 usage, 3 invalid
input, 4 output refused, 5 ok but something needs a person or was not checked. This script adds 6:
a required program is missing or does not speak a version this script knows.
"""
import argparse
import json
import os
import pathlib
import shutil
import subprocess
import sys

SKILL = pathlib.Path(__file__).resolve().parent.parent
MANAGER_INTERFACE = "1"
FASTROUTE_NEEDS = ["check", "compile", "report", "validate", "extract-kicad", "export-fastroute", "edit-request", "check-edit"]
EXIT_TOOL = 6


# ---- discovery and version negotiation -------------------------------------------------------------

def _candidates(name, flag, env):
    seen = []
    for c in (flag, os.environ.get(env)):
        if c:
            seen.append(c)
    found = shutil.which(name)
    if found:
        seen.append(found)
    root = SKILL.parent.parent  # when the Skill lives in the fastroute repository
    for profile in ("release", "debug"):
        p = root / "target" / profile / name
        if p.is_file():
            seen.append(str(p))
    return seen


def _run(cmd, timeout=60):
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        return p.returncode, p.stdout, p.stderr
    except (OSError, subprocess.TimeoutExpired) as e:
        return 127, "", str(e)


def find_fastroute(flag=None):
    """The first fastroute that has every `constraints` command this script uses."""
    problems = []
    for path in _candidates("fastroute", flag, "FASTROUTE_BIN"):
        code, out, err = _run([path, "constraints", "--help"])
        if code != 0:
            problems.append(f"{path}: cannot run (`constraints --help` exit {code})")
            continue
        missing = [c for c in FASTROUTE_NEEDS if c not in out]
        if missing:
            problems.append(f"{path}: too old, no `constraints {'`, `constraints '.join(missing)}`")
            continue
        _, ver, _ = _run([path, "--version"])
        return {"path": path, "version": ver.strip() or "unknown", "ok": True}, problems
    return None, problems


def find_manager(flag=None):
    """The first hcp-manager whose `info` speaks interface MANAGER_INTERFACE."""
    problems = []
    for path in _candidates("hcp-manager", flag, "HCP_MANAGER_BIN"):
        code, out, err = _run([path, "info", "--json"])
        if code != 0:
            problems.append(f"{path}: cannot run (`info --json` exit {code})")
            continue
        try:
            info = json.loads(out)
        except ValueError:
            problems.append(f"{path}: `info --json` is not JSON")
            continue
        if info.get("format") != "hcp-manager-info":
            problems.append(f"{path}: not hcp-manager")
            continue
        if info.get("interface") != MANAGER_INTERFACE:
            problems.append(f"{path}: speaks interface {info.get('interface')!r}, this Skill speaks {MANAGER_INTERFACE!r}: update the Skill or the application")
            continue
        return {"path": path, "version": info.get("version"), "interface": info["interface"], "capabilities": info.get("capabilities", {}), "ok": True}, problems
    return None, problems


FASTROUTE_HELP = (
    "fastroute with `constraints` is required (it does the deterministic work). Build it from the repository: "
    "`cargo build --release -p fastroute`, then put target/release on PATH or set FASTROUTE_BIN."
)
MANAGER_HELP = (
    "hcp-manager is not available: documents cannot be fetched, cached or searched, and evidence cannot be compared with them. "
    "The local workflow still works (check, compile, report, validate need only fastroute); say so to the user and mark the "
    "evidence as not verified. To install: `cargo build --release -p hcp-manager`, then PATH or HCP_MANAGER_BIN."
)


def need_fastroute(args):
    tool, problems = find_fastroute(args.fastroute)
    if tool is None:
        print(json.dumps({"status": "tool_missing", "tool": "fastroute", "problems": problems, "next": FASTROUTE_HELP}, indent=2), file=sys.stderr)
        sys.exit(EXIT_TOOL)
    return tool["path"]


def need_manager(args):
    tool, problems = find_manager(args.manager)
    if tool is None:
        print(json.dumps({"status": "tool_missing", "tool": "hcp-manager", "problems": problems, "next": MANAGER_HELP}, indent=2), file=sys.stderr)
        sys.exit(EXIT_TOOL)
    return tool["path"]


# ---- commands --------------------------------------------------------------------------------------

def cmd_doctor(args):
    fr, fr_problems = find_fastroute(args.fastroute)
    mg, mg_problems = find_manager(args.manager)
    caps = (mg or {}).get("capabilities", {})
    report = {
        "fastroute": fr or {"ok": False, "problems": fr_problems},
        "hcp_manager": mg or {"ok": False, "problems": mg_problems},
        "workflows": {
            "local_pack_workflow": bool(fr),
            "document_research": bool(mg),
            "keyword_search": bool(caps.get("keyword_search")),
            "vector_search": bool(caps.get("vector_index")),
            "network_fetch": bool(caps.get("network_fetch")),
            "pdf_text": bool(caps.get("pdf_text")),
        },
        "notes": [],
    }
    if not fr:
        report["notes"].append(FASTROUTE_HELP)
    if not mg:
        report["notes"].append(MANAGER_HELP)
    if mg and not caps.get("vector_index"):
        report["notes"].append("no vector index: search is by keyword, filtered by device, version and source level; nothing depends on a vector result.")
    print(json.dumps(report, indent=2, ensure_ascii=False))
    return EXIT_TOOL if not fr else (5 if not mg else 0)


def forward_fastroute(args, rest, sub):
    fr = need_fastroute(args)
    return subprocess.call([fr, "constraints", sub, *rest])


MANAGER_MAP = {
    "init": ["init"],
    "sources": ["sources"],
    "resolve": ["sources", "resolve"],
    "sync": ["sources", "sync"],
    "index": ["index"],
    "search": ["search"],
    "verify": ["evidence", "verify"],
    "packs": ["packs"],
}


def cmd_research(args, rest):
    if not rest or rest[0] not in MANAGER_MAP:
        print("usage: hcp_cli.py research {%s} [options]" % "|".join(MANAGER_MAP), file=sys.stderr)
        return 2
    mg = need_manager(args)
    cmd = [mg]
    if args.workspace:
        cmd += ["--workspace", args.workspace]
    cmd += MANAGER_MAP[rest[0]] + rest[1:]
    p = subprocess.run(cmd, capture_output=True, text=True)
    sys.stdout.write(p.stdout)
    sys.stderr.write(p.stderr)
    if p.returncode == 3 and "no inventory" in p.stderr:
        print("next: `research init`, then add the documents (`research sources import FILE` or `research sources add ...`), then `research sync`.", file=sys.stderr)
    elif p.returncode == 3 and rest[0] == "search" and ("index" in p.stderr or "not built" in p.stderr):
        print("next: the derived text is missing or stale: `research index build` (it is rebuilt from the cache; nothing is lost).", file=sys.stderr)
    elif p.returncode == 5 and rest[0] == "sync":
        print("note: some documents could not be fetched or their digest changed; work from what is cached, say which documents are missing, and ask a person before adopting a changed document (`--accept`).", file=sys.stderr)
    return p.returncode


def cmd_templates(rest):
    text = (SKILL / "assets" / "rule-templates.yaml").read_text(encoding="utf-8")
    parts = text.split("\n  - name: ")
    templates = {chunk.split("\n", 1)[0].strip(): "  - name: " + chunk for chunk in parts[1:]}
    if not rest:
        for name, body in templates.items():
            use = next((l.split(":", 1)[1].strip().strip('"') for l in body.splitlines() if l.strip().startswith("use_when:")), "")
            print(f"{name}\t{use}")
        return 0
    if rest[0] not in templates:
        print(f"no template '{rest[0]}'; the names: {', '.join(templates)}", file=sys.stderr)
        return 2
    print(templates[rest[0]].rstrip())
    return 0


def cmd_extract(args, rest):
    if not rest or rest[0] not in ("kicad", "templates"):
        print("usage: hcp_cli.py extract kicad BOARD.kicad_pcb -o design.json | extract templates [NAME]", file=sys.stderr)
        return 2
    if rest[0] == "templates":
        return cmd_templates(rest[1:])
    return forward_fastroute(args, rest[1:], "extract-kicad")


def _step(name, status, **kw):
    return {"step": name, "status": status, **kw}


def cmd_pipeline(args, rest):
    ap = argparse.ArgumentParser(prog="hcp_cli.py pipeline", description="check, compile, report, verify the evidence, validate: one run, one status file")
    ap.add_argument("pack")
    ap.add_argument("--out", required=True, help="directory for the outputs (created)")
    ap.add_argument("--design", help="a design snapshot (JSON)")
    ap.add_argument("--kicad-pcb", help="a KiCad board (read-only)")
    ap.add_argument("--lang", default="zh", choices=["zh", "en"])
    ap.add_argument("--export", action="store_true", help="also write the inputs fastroute can execute")
    o = ap.parse_args(rest)
    if o.design and o.kicad_pcb:
        print("give --design or --kicad-pcb, not both", file=sys.stderr)
        return 2
    fr = need_fastroute(args)
    out = pathlib.Path(o.out)
    out.mkdir(parents=True, exist_ok=True)
    steps, humans = [], []
    base = [fr, "constraints"]

    def run(name, cmd, outputs):
        code, so, se = _run(cmd, timeout=600)
        (out / f"{name}.log").write_text((so or "") + (se or ""), encoding="utf-8")
        return code, so, se

    code, _, se = run("1-check", base + ["check", o.pack, "--json", "-o", str(out / "check.json"), "--force", "-q"], ["check.json"])
    if code != 0:
        steps.append(_step("check", "failed", exit=code, note="the pack is not valid; see check.json and 1-check.log", outputs=["check.json"]))
        return _finish(out, steps, humans, 3 if code == 3 else code)
    steps.append(_step("check", "ok", exit=0, outputs=["check.json"]))

    code, _, se = run("2-compile", base + ["compile", o.pack, "-o", str(out / "ir.json"), "--force", "-q"], ["ir.json"])
    if code != 0:
        steps.append(_step("compile", "failed", exit=code, note=se.strip()[-300:]))
        return _finish(out, steps, humans, code)
    steps.append(_step("compile", "ok", exit=0, outputs=["ir.json"]))
    ir = json.loads((out / "ir.json").read_text(encoding="utf-8"))
    rules = ir.get("rules", [])
    unknown = [r["rule_id"] for r in rules if r.get("status") == "unknown"]
    conflict = [r["rule_id"] for r in rules if r.get("status") == "conflict"]
    review = ir.get("pack", {}).get("review")
    if unknown:
        humans.append(f"{len(unknown)} rule(s) are unknown (the documents do not say): {', '.join(unknown)}. Find the document or accept that they stay unknown.")
    if conflict:
        humans.append(f"{len(conflict)} rule(s) are in conflict: {', '.join(conflict)}. A person decides which document applies.")
    if review in (None, "draft"):
        humans.append("the pack is a draft: a person has to review it before it is used as `reviewed` (who and when are theirs to give).")

    code, _, se = run("3-report", base + ["report", o.pack, "-o", str(out / "pack-report.md"), "--lang", o.lang, "--force", "-q"], ["pack-report.md"])
    steps.append(_step("report", "ok" if code == 0 else "failed", exit=code, outputs=["pack-report.md"]))

    mg, _ = find_manager(args.manager)
    if mg and args.workspace and (pathlib.Path(args.workspace) / "sources.yaml").is_file():
        code, so, se = run("4-evidence", [mg["path"], "--workspace", args.workspace, "--json", "evidence", "verify", o.pack], [])
        (out / "evidence.json").write_text(so, encoding="utf-8")
        if code == 0:
            steps.append(_step("evidence", "ok", exit=0, outputs=["evidence.json"]))
        elif code == 5:
            steps.append(_step("evidence", "needs_human", exit=5, outputs=["evidence.json"], note="some evidence could not be compared (document not cached, or chunk stale)"))
            humans.append("some evidence could not be compared with the documents: see evidence.json (sync the documents or rebuild the index, then run again).")
        else:
            steps.append(_step("evidence", "failed", exit=code, outputs=["evidence.json"], note="a source, digest or quotation of the pack is not in the cited document"))
    else:
        why = "hcp-manager is not available" if not mg else "no workspace with an inventory was given (--workspace)"
        steps.append(_step("evidence", "skipped", note=f"{why}: the quotations were not compared with the documents"))
        humans.append(f"the evidence was not compared with the documents ({why}): a person has to check the quotations against the pages cited in pack-report.md.")

    final = 0
    if o.design or o.kicad_pcb:
        design = ["--design", o.design] if o.design else ["--kicad-pcb", o.kicad_pcb]
        code, _, se = run("5-validate", base + ["validate", "--ir", str(out / "ir.json")] + design + ["-o", str(out / "validation.json"), "--report", str(out / "validation.md"), "--lang", o.lang, "--force", "-q"], ["validation.json", "validation.md"])
        status = {0: "ok", 5: "needs_human", 1: "failed"}.get(code, "failed")
        steps.append(_step("validate", status, exit=code, outputs=["validation.json", "validation.md"]))
        if code == 5:
            humans.append("the design violates no rule, but some rules were not checked or are unknown: see validation.md.")
        final = code if code in (1, 3) else final
    else:
        steps.append(_step("validate", "skipped", note="no design given (--design or --kicad-pcb)"))

    if o.export:
        design = ["--design", o.design] if o.design else (["--kicad-pcb", o.kicad_pcb] if o.kicad_pcb else [])
        code, _, se = run("6-export", base + ["export-fastroute", "--ir", str(out / "ir.json")] + design + ["-o", str(out / "fastroute"), "--force", "-q"], ["fastroute/"])
        steps.append(_step("export", "ok" if code == 0 else "failed", exit=code, outputs=["fastroute/export-report.json"]))
    return _finish(out, steps, humans, final)


def _finish(out, steps, humans, final):
    failed = [s for s in steps if s["status"] == "failed"]
    if final == 0 and failed:
        final = 1
    if final == 0 and (humans or any(s["status"] in ("needs_human", "skipped") for s in steps)):
        final = 5
    status = {"format": "hcp-skill-pipeline", "format_version": "1.0", "exit": final, "steps": steps, "needs_human": humans}
    (out / "pipeline-status.json").write_text(json.dumps(status, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    width = max(len(s["step"]) for s in steps)
    for s in steps:
        print(f"{s['step']:<{width}}  {s['status']:<11}{(' - ' + s['note']) if s.get('note') else ''}")
    for h in humans:
        print(f"needs a person: {h}")
    print(f"status: {out / 'pipeline-status.json'} (exit {final})")
    return final


def main(argv):
    ap = argparse.ArgumentParser(
        prog="hcp_cli.py",
        description=__doc__.split("\n\n")[0],
        epilog="commands: doctor | research {init,sources,resolve,sync,index,search,verify,packs} | extract {kicad,templates} | "
        "check | compile | report | validate | export | edit-request | check-edit | pipeline. "
        "`hcp_cli.py COMMAND --help` shows the options of the program behind it.",
    )
    ap.add_argument("--fastroute", help="path of fastroute (else FASTROUTE_BIN, PATH, the repository's target/)")
    ap.add_argument("--manager", help="path of hcp-manager (else HCP_MANAGER_BIN, PATH, the repository's target/)")
    ap.add_argument("--workspace", help="the hcp-manager workspace (else its own default)")
    ap.add_argument("command")
    ap.add_argument("rest", nargs=argparse.REMAINDER)
    args = ap.parse_args(argv)
    c, rest = args.command, args.rest
    if c == "doctor":
        return cmd_doctor(args)
    if c == "research":
        return cmd_research(args, rest)
    if c == "extract":
        return cmd_extract(args, rest)
    if c == "pipeline":
        return cmd_pipeline(args, rest)
    fastroute_cmds = {"check": "check", "compile": "compile", "report": "report", "validate": "validate", "export": "export-fastroute", "edit-request": "edit-request", "check-edit": "check-edit"}
    if c in fastroute_cmds:
        return forward_fastroute(args, rest, fastroute_cmds[c])
    print(f"unknown command '{c}' (doctor, research, extract, check, compile, report, validate, export, edit-request, check-edit, pipeline)", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
