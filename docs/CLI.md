# Command line

fastroute has one program with five commands, and the original flag interface that every
existing script uses. Nothing was removed: old commands keep running as before.

| Command | Does | Reads | Writes |
|---|---|---|---|
| `fastroute route` | autoroutes a design | `.dsn` (engine) or `.kicad_pcb` (KiCad adapter) | `.ses` or `.kicad_pcb` |
| `fastroute render` | draws the board or a region as a PNG ([RENDERING.md](RENDERING.md)) | `.dsn` [+ `.ses`] | `.png` + `.manifest.json` |
| `fastroute inspect` | describes the board as JSON ([AI_DATA_FORMAT.md](AI_DATA_FORMAT.md)) | `.dsn` [+ `.ses`] | `inspect.json` |
| `fastroute ai-pack` | writes a local analysis pack ([AI_PACK.md](AI_PACK.md)) | `.dsn` [+ `.ses`, `.kicad_pcb`] | a directory |
| `fastroute rules` | checks, confirms and applies rule drafts from people, scripts or AI models ([AI_RULES.md](AI_RULES.md)) | drafts (JSON), `inspect.json` / ai-pack | check reports, pairs / tune / `.kicad_dru` files |
| `fastroute -de … -do …` | the original interface | `.dsn` | `.ses` |

`fastroute --help`, `fastroute <command> --help` and `fastroute help <command>` print the same
text. Options are written `--name value` or `--name=value`; `-i`, `-o`, `-s`, `-q` are short forms.
All commands share the exit codes, the output rules and the error format below.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | done |
| 1 | failed (anything not covered below) |
| 2 | usage: unknown or conflicting options, a value that is not a number, an unknown name |
| 3 | input: a file is missing, unreadable or not what it should be (a KiCad board where a DSN is needed, a design without an outline, a session that is not a session; for `rules`: a draft that is not valid, a confirmation that does not match its draft) |
| 4 | output: the output exists (without `--force`), is the input, or cannot be written |
| 5 | `route` / `ai-pack --require-drc`: the result is written, but connections remain unrouted / there is no DRC result |

Errors are one line on standard error, `fastroute <command>: <what is wrong and what to do>`,
usage errors add `(see fastroute <command> --help)`. Checks run in the order usage → input →
output, so the first thing wrong is the one reported, and an unusable input is never reported
as an output problem. Everything that can be refused is refused **before** a long run starts.

## Outputs are never overwritten by accident

* An existing output file is refused (exit 4) unless `--force`.
* An input file is never a valid output, not with `--force` and not under another spelling of
  the same path.
* Without `--output`, `route` writes `<name>.routed.ses` (or `<name>.routed.kicad_pcb`) next to
  the input; the default never is the input.
* Files are written through a temporary file and a rename, so a failure or a kill never leaves
  half a file.
* `render` and `ai-pack` also write nothing when any of their outputs would be refused.

## `fastroute route`

```sh
fastroute route board.dsn                          # -> board.routed.ses
fastroute route board.dsn -o result.ses --max-time 600 --json
fastroute route board.kicad_pcb -o routed.kicad_pcb   # through the KiCad adapter
```

| Option | Meaning |
|---|---|
| `-i`, `--input FILE` | the design (also the first positional argument) |
| `-o`, `--output FILE` | the result; default `<input>.routed.ses` / `.kicad_pcb` |
| `--force` | replace an existing output |
| `--mode fast\|exact\|quick` | parallel optimizer (default) · Freerouting-identical · no optimizer |
| `--max-passes N`, `--max-time SECONDS`, `--threads N`, `--multi-start N` | limits and parallelism |
| `--report FILE`, `--diagnose` | JSON report of the result (`.dsn` input) |
| `--initial-session FILE`, `--pairs FILE`, `--tune FILE`, `--pair-skew MM`, `--layer-heights MM,..`, `--no-neckdown-classes A,B` | as in the original interface (`.dsn` input) |
| `--parity`, `--no-enhancements` | exact Freerouting behaviour (`.dsn` input) |
| `--router.<path>=<value>` | any Freerouting-style router setting, passed on unchanged |
| `--live`, `--live-port PORT` | watch the routing in a browser; **opt-in**, nothing depends on it |
| `--clear`, `--keep-existing`, `--python`, `--route-cli`, `--work-dir` | KiCad boards only, see below |
| `--allow-partial` | exit 0 also when connections remain unrouted |
| `--json` | one JSON summary on standard output |
| `-q`, `--quiet` / `-v`, `--verbose` | only warnings and errors / debug output |

The routing log goes to standard error; with `--quiet` only warnings and errors remain.

### Results and exit codes

A route that leaves connections unrouted still writes its best result and is **exit code 5**
(`--allow-partial`: 0), so a script can tell "done" from "done, but not finished" without
reading a log:

```json
{"command":"route","fastroute_version":"0.1.14","status":"partial","exit_code":5,
 "input":"board.dsn","output":"board.routed.ses","unrouted":2,"violations":1,
 "seconds":1.535,"error":null}
```

`status` is `complete`, `partial` or `failed`; for `failed`, `output`, `unrouted` and
`violations` are `null` and `error` says why. The same fields are used for `.kicad_pcb` input.

### KiCad boards

A `.kicad_pcb` is routed through the plugin's adapter `integrations/kicad/plugins/route_cli.py`,
which needs **KiCad's own Python** (the `pcbnew` module) — see *External programs* below. The
command checks the dependencies first and names what is missing:

```
fastroute route: no Python with KiCad's pcbnew module: python3 (python3 on PATH): ModuleNotFoundError:
No module named 'pcbnew'. Routing a .kicad_pcb needs KiCad's Python (KiCad 8 or newer, which can import a session headless): give it with
--python or $FASTROUTE_KICAD_PYTHON ...
```

The Python is `--python`, `$FASTROUTE_KICAD_PYTHON`, KiCad's bundled one (macOS, Windows), then
`python3`; the first that can `import pcbnew` is used. `route_cli.py` is `--route-cli`,
`$FASTROUTE_ROUTE_CLI`, or the one in the sources / the installed plugin next to the program.
A `.dsn` needs none of this; exporting one from KiCad (File → Export → Specctra DSN) and
routing that works everywhere.

`route_cli.py` can also be run directly and has its own options (`--obstacle-zones`,
`--zones-as-planes`, `--no-refill`, `--export-only`, …; see the
[plugin README](../integrations/kicad/README.md)). What changed there:

* **`-o` is no longer "overwrite the input".** Without `-o` the result is
  `<board>.routed.kicad_pcb` next to the input; an output that exists is refused (exit 4,
  before any work) unless `--force`. Writing over the input now takes `-o board.kicad_pcb --force`.
* New: `--force`, `--json` (one JSON summary line on standard output, the text goes to standard
  error), `--fail-on-partial` (exit 5 if connections remain unrouted).
* The board is saved through a temporary file. Exit codes: 0 done, 1 routing failed, 2 the board
  cannot be used, 4 output refused or not writable, 5 `--fail-on-partial`.

## Compatibility: the original interface

The original flags are unchanged: same names, same defaults, same behaviour, same exit codes.
Their parser is separate from the new one on purpose.

| Original | Same thing in `fastroute route` |
|---|---|
| `-de FILE` | `--input FILE` |
| `-do FILE` | `--output FILE` |
| `-mp N` | `--max-passes N` |
| `--max-time=S`, `--multi-start=N`, `--report=F`, `--diagnose`, `--initial-session=F`, `--pairs=F`, `--tune=F`, `--pair-skew=MM`, `--layer-heights=…`, `--no-neckdown-classes=…`, `--parity`, `--no-enhancements` | the same names as `--option value` or `--option=value` |
| `--live[=PORT]` | `--live` `[--live-port PORT]` |
| `-v` | `-v`, `--verbose` |
| `--router.<path>=<value>` | unchanged |
| `--optimizer-mode`, `--time-limit-mode`, `--time-limit-factor`, `--no-time-limits` | not offered by `route`; keep using the original flags (they are for benchmarking and parity runs) |
| *(none)* | `--quiet` — added to the original interface as well |

What differs between the two interfaces, so that scripts can choose:

| | original (`-de … -do …`) | `fastroute route` |
|---|---|---|
| no output given | no session is written | `<name>.routed.ses` |
| output exists | replaced silently | refused (exit 4) unless `--force` |
| input = output | possible | never |
| unrouted connections remain | exit 0 | exit 5 (`--allow-partial`: 0) |
| errors | exit 1, `fastroute: <message>` | exit 1/2/3/4, `fastroute route: <message>` |
| bad design file | exit 1 after the parser's log | exit 3 before routing |
| summary for programs | none (`--report` has the numbers) | `--json` |
| the session file during the run | rewritten with the best board so far | the same |

Migration: replace `fastroute -de a.dsn -do a.ses ARGS` with
`fastroute route a.dsn -o a.ses --force ARGS` (`--force` keeps the old "replace" behaviour for a
file that exists), and check for exit code 5 where a script used to parse the log for unrouted
connections. Nothing has to be migrated; the original form keeps working.

## External programs

| Needed by | Program | Why | If missing |
|---|---|---|---|
| nothing | — | `route` / `render` / `inspect` / `ai-pack` on `.dsn` / `.ses`, no browser, window system or network | — |
| `route` on `.kicad_pcb` | **KiCad's Python with `pcbnew`** (**KiCad 8 or newer**) and `route_cli.py` | read the board, export the DSN, import the SES, refill zones | exit 3 naming `--python` / `--route-cli`, or the KiCad version that cannot import a session headless |
| `ai-pack --kicad-pcb` | **`kicad-cli`** (**KiCad 8 or newer**: `pcb drc` does not exist in 7) | KiCad's own DRC | `drc_status: "unavailable"`, a warning that names the version, the pack is still written |
| `--live` | a web browser (`xdg-open` / `open` / `start`; `FASTROUTE_LIVE_NO_OPEN=1` skips opening) | the live view | the URL is printed; routing is unaffected |

`pcbnew` and `kicad-cli` come with KiCad; they are not Rust crates and are not bundled. On
Linux `python3` can import `pcbnew` if the distribution's KiCad Python module is installed
(check with `python3 -c "import pcbnew"`); on macOS and Windows use the Python inside the KiCad
installation (see the plugin README for the paths).

## Checked against a real KiCad

The stand-ins in the tests prove the logic; these were also run against a real **KiCad 7.0.11**
(Ubuntu `kicad` package, 12 of KiCad's demo boards):

| What | Result |
|---|---|
| `inspect` / `render` / `ai-pack` on the DSN that KiCad's own exporter writes | all boards exit 0; pad, outline and via coordinates agree with `pcbnew` exactly (same frame: mm, y down, board origin) |
| `ai-pack --kicad-pcb` | KiCad 7's `kicad-cli pcb` has no `drc` command, so `drc_status` is `unavailable` with the version in the message, the pack is `partial`, `--verify` passes, and `--require-drc` exits 5. No false "passed" |
| `route board.kicad_pcb` | refused in 0.2 s with exit 3 and the reason: in KiCad 7 `ImportSpecctraSES` takes only a file name and needs the PCB editor, so the routed session could not be imported headless. Before this check the run would route first and fail afterwards |
| `route_cli.py --export-only` | works |

Not run for lack of a KiCad 8+ in the environment (the stand-in tests cover the code paths):
the `kicad-cli pcb drc` success path and the parsing of a real DRC report, the coordinate frame of
DRC items, a complete headless `route` ending in an imported board, and `.kicad_dru` files read by
KiCad. Two things seen in the real data are documented in
[AI_DATA_FORMAT.md](AI_DATA_FORMAT.md): KiCad's DSN export renames duplicate pad names and omits
mounting holes without copper, and `pads[].x/y` is the pad centre as the router uses it, which for
custom-shaped pads is not KiCad's pad position.

## Tests

* `crates/fastroute/tests/route_cli.rs` — the `route` command: complete / partial / failed,
  JSON summary, overwrite protection (also before routing starts and for the report file),
  default output next to the input, `--quiet`, every usage and input error with its exit code,
  help of every command, legacy flags and a byte-identical session from both interfaces, a
  headless run (empty `PATH`, no display), and the KiCad adapter with a stand-in Python:
  argument forwarding, exit-code mapping, missing `pcbnew` / interpreter / `route_cli.py`, and a
  KiCad whose Python API cannot import a session headless (refused before the adapter starts).
* `integrations/kicad/tests/test_route_cli.py` — `route_cli.py` with a stand-in `pcbnew`:
  default output, refusal and `--force`, writing over the input, temporary file on a failed
  save, `--json`, `--fail-on-partial`. Run by `cargo test` (via `crates/fastroute/tests/kicad_adapter_py.rs`) and by
  `python3 -m unittest discover -s integrations/kicad/tests -p 'test_*.py'`.
  They were also run against the previous `route_cli.py`, which fails 11 of the 15.
* the tests of `render`, `inspect`, `ai-pack` and `rules` check the same output rules, exit codes
  and messages for their commands.
