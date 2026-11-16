# Command line

fastroute has one program with six commands, and the original flag interface that every
existing script uses. Nothing was removed: old commands keep running as before.

| Command | Does | Reads | Writes |
|---|---|---|---|
| `fastroute route` | autoroutes a design | `.dsn` (engine) or `.kicad_pcb` (KiCad adapter) | `.ses` or `.kicad_pcb` |
| `fastroute render` | draws the board or a region as a PNG ([RENDERING.md](RENDERING.md)) | `.dsn` [+ `.ses`] | `.png` + `.manifest.json` |
| `fastroute inspect` | describes the board as JSON ([AI_DATA_FORMAT.md](AI_DATA_FORMAT.md)) | `.dsn` [+ `.ses`] | `inspect.json` |
| `fastroute ai-pack` | writes a local analysis pack ([AI_PACK.md](AI_PACK.md)) | `.dsn` [+ `.ses`, `.kicad_pcb`] | a directory |
| `fastroute rules` | checks, confirms and applies rule drafts from people, scripts or AI models ([AI_RULES.md](AI_RULES.md)) | drafts (JSON), `inspect.json` / ai-pack | check reports, pairs / tune / `.kicad_dru` files |
| `fastroute constraints` | checks, compiles and validates hardware constraint packs (HCP) against a design ([HCP.md](HCP.md)) | packs (YAML/JSON), designs | check reports, Constraint IR, validation reports |
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

## `fastroute constraints`

Hardware Constraint Packs (HCP): what a device requires, with evidence, checked deterministically
and offline (no AI model, network or vector index). Overview and trust boundaries: [HCP.md](HCP.md).

```sh
fastroute constraints check pack.yaml                     # schema + references, evidence, units, whitelist
fastroute constraints compile pack.yaml -o ir.json        # the Constraint IR (same bytes every time)
fastroute constraints validate --ir ir.json --design design.json --report report.md
```

| Command | Does |
|---|---|
| `schema [--ir \| --snapshot \| --edit-draft \| --edit-request]` | prints the JSON Schema of a pack, of the IR, of a design snapshot, of an AI edit draft or of an edit request |
| `check PACK` | exit 0 valid (warnings listed), 3 not valid; `--json` for the report as data |
| `compile PACK -o IR` | exit 3 for a pack with errors (nothing is written); keeps the evidence of every rule and says per rule what the validator and the fastroute adapter can execute |
| `validate (--ir IR \| --pack PACK) --design SNAPSHOT` | checks a design snapshot: **0** pass and every rule checked, **5** no rule violated but some are unchecked / unsupported / unknown / in conflict, **1** a rule is violated (rule, object, actual and required value, severity, evidence per finding), **3** invalid input (also an IR edited after compilation) |

`--report FILE` writes a readable Markdown report (`--lang zh|en`, default `zh`); `--output FILE`
the JSON. Outputs follow the usual rules (exit 4 if one exists without `--force`, never an input).

### A fixed base design that an AI may edit around

```sh
fastroute constraints edit-request --pack pack.yaml --requirement "add a status LED" -o request.json
fastroute constraints check-edit draft.json --pack pack.yaml --kicad-pcb carrier.kicad_pcb \
    --base-dir ./carrier-repo --report diff.md
```

* `edit-request` computes the whitelist from the pack (editable regions, allowed blocks, pin pools
  without protected pins, allowed operations, immutable objects, fixed regions) and binds the request to
  the pack and the base design by SHA-256. Same pack, same bytes.
* `check-edit` checks an AI's **draft** (a list of changes, never a rewritten board) against the
  whitelist and the pack's rules, and writes what would change. **0** accepted and fully checked, **5**
  accepted but not everything was checked / the pack or the base design changed since the draft was made
  (review) / the generator stopped, **1** rejected (outside the regions, a fixed object, a protected pin,
  an unauthorised block, cross-region copper, a keep-out, a rule of the pack newly broken), **3** not a
  draft, **4** an output that would overwrite an input, a bound file or anything inside `--base-dir`.
* The base design is only read. A search result in the draft (keyword or vector) is information: it
  never adds a region, block, pin or operation. Details and finding codes:
  `skills/hardware-constraint-design/references/editable-base.md`.

### KiCad boards and fastroute

```sh
fastroute constraints extract-kicad board.kicad_pcb -o design.json      # read-only, no KiCad needed
fastroute constraints validate --pack pack.yaml --kicad-pcb board.kicad_pcb --report report.md
fastroute constraints export-fastroute --pack pack.yaml --kicad-pcb board.kicad_pcb -o rules-out/
```

* `extract-kicad` (and `validate --kicad-pcb`) **only read** the board (format of KiCad 6 to 10,
  also the older `module`): components, pads with absolute positions and nets, tracks, vias, zones,
  outline, mounting holes. The board is never modified and is never an output. The reader was
  compared with KiCad's own Python API (`pcbnew`) on all 15 of KiCad 8's demo boards and a fixture:
  every pad position (to 0.000001 mm), net, track, via and component agrees; an arc is two chords.
* The **KiCad DRC and the HCP validation are separate results** (`fastroute ai-pack --kicad-pcb`
  runs the DRC; `constraints validate` the pack). Neither says anything about the other.
* `export-fastroute` writes only what fastroute reads, and `export-report.json` says what it did not
  hand over and why:

| Rule of the IR | Handed to | As | Not handed over (reported) |
|---|---|---|---|
| `high_speed`, differential, two nets | the router | `--pairs` `pair P N gap=… skew=…` | `impedance_ohm`, `layers`, `min_width_mm` (no model / no rule-file option); `max_length_mm` (the router does not enforce maximum lengths; KiCad's DRC does) |
| `high_speed`, `length_match_tolerance_mm`, two or more nets | the router | `--tune` group | the same |
| `clearance`, `trace_width` | KiCad and the plugin | `.kicad_dru` rules (see [IMPROVEMENTS.md](IMPROVEMENTS.md): the plugin hands them to the router) | a pattern that matches no net, an unsafe net name |
| `keepout` | – | – | a rule file cannot add a keep-out; it comes from the design (a new core capability) |
| `electrical` | – | – | no electrical model |
| `reserved_pin`, `nc_pin`, `fixed_net`, `exclusive_functions`, `required_block`, strapping / reset / boot, mechanical | the validator | – | they are design checks, listed as `checked_by_the_validator_not_the_router` |
| `unknown`, `conflict` rules | nobody | – | never exported |
| `applies_when` conditions | – | – | words cannot be evaluated: the rule is exported for the nets it names (`conditions_not_evaluated`) |

fastroute runs without the pack, the IR, `hcp-manager` or any index: only the written files.

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

The stand-ins in the tests prove the logic; these were also run against real KiCads: **7.0.11**
(Ubuntu `kicad`, 12 of KiCad's demo boards) and **8.0.9** (KiCad's PPA, `pic_programmer`,
`flat_hierarchy`, `interf_u`).

| What | KiCad 7.0.11 | KiCad 8.0.9 |
|---|---|---|
| `inspect` / `render` / `ai-pack` on the DSN that KiCad's exporter writes | all boards exit 0 | `inspect` on 3 boards, `render` and `ai-pack` on `pic_programmer`: exit 0 |
| coordinates: pads of `inspect.json` against `pcbnew` and against the items of KiCad's DRC report | exact (mm, y down, same origin; also for boards with a user origin that is not the page origin) | exact: 10 pads named by DRC items, 0.000 mm apart |
| `kicad-cli pcb drc --format json --units mm --severity-all --output` | **does not exist** (`pcb` has only `export`): `drc_status: unavailable` naming the version, pack `partial`, `--require-drc` exits 5, never "passed" | works; the report is read: `drc_status` `failed`/`passed`, issues with coordinates, `--verify` consistent |
| `render --from-report drc.json --select DRC72,DRC73,DRC74` | – | the region follows the items; the numbered markers sit on the pads and holes they name |
| `fastroute route board.kicad_pcb` | refused in 0.2 s with the reason (`ImportSpecctraSES` takes only a file name and needs the PCB editor; the same failure happened with the original `route_cli.py`) | routes, imports the session, saves; KiCad's own count after the import is 0 open connections |
| a `.kicad_dru` written by `fastroute rules apply` | – | read and evaluated by KiCad's DRC (a 3 mm `track_width` rule gave 7 violations on the net it names; a 0.3 mm `clearance` rule 12 more); the clearance and track_width rules reach the router (below) |

What the real runs found and fixed:

* KiCad 7 cannot do the two command-line steps (above); both are detected up front with the reason.
* The plugin did not understand the router's score line when the board has violations between
  fixed items (`final score: 957.19 (4 unrouted and 2 violations, 2 of them pre-existing ...)`), so
  the unrouted and violation counts were `null` for such boards. Fixed.
* `fastroute route` called a run with unknown counts "complete". The adapter's verdict now decides,
  and it is KiCad's own open-connection count after the import: the router counted 4 unrouted
  connections on `pic_programmer` (connections through a zone), KiCad 0, so the result is `complete`.
* KiCad 8 writes a project (`.kicad_pro`) and local settings (`.kicad_prl`) next to every board
  it saves, also next to the temporary file; they were left behind. Now removed.
* The plugin did not hand `clearance` and single-net `track_width` rules to the router
  (`NOT applied`) unless they named net classes (and for widths also a layer), while the text that
  `rules apply` wrote said it carries them into the DSN. They now reach the router:
  * a clearance rule without a condition applies to every pair of net classes; the nets of an
    `A.NetName` / `A.NetClass` condition get a class of their own (same circuit, width and via
    rules) whose pairs get the clearance. With KiCad 8.0.9's DRC on `pic_programmer`: a 0.3 mm rule
    for all nets left 0 clearance violations (79 when the router did not see it); a 0.45 mm rule
    for two nets and a 0.35 mm rule for a third left none on those nets (4-6 each before).
  * a `track_width` rule sets the width of every class (no condition), of a net class, or of the
    nets of a condition (a class of their own), on all layers or on the layer it names; `opt`, else
    the middle of `min` and `max`, else `min`. Those classes get no neck-down. Real DRC: three nets
    with 0.6 mm / 0.5-0.7 mm rules (one only on F.Cu) had 6, 5 and 33 `track_width` violations
    before and 0 after.
  * nets of a net class that the autorouter ignores (`--router.autorouter.ignore_net_classes`) stay
    in their class: moving them into a class of their own would make the router route them. A width
    rule is then not applied to them (nothing is routed there); a clearance rule is applied to the
    whole class, and only ever raises it (printed), because the router routes the other nets past
    the pads and copper of that class. On `pic_programmer` with `POWER` ignored, a 0.45 mm rule for
    GND left 15 clearance violations against GND / VCC before and none after (one more connection
    stays unrouted under the stricter clearance). The router compares the class names ignoring case
    and calls the default class `default`; the plugin compared exactly, so with
    `ignore_net_classes=power` for the class `POWER` a rule moved GND out of it and GND was routed
    (11 ignored connections instead of 50 on `pic_programmer`). Fixed and tested on that board.
  What still is not applied is printed as `NOT applied`: conditions on item types (`A.Type`), areas,
  and `B.` terms on nets. A `(layer ...)` clearance restriction is applied to all layers and a
  clearance below a class's own lowers it; both are said. When rules overlap, the later one wins for
  a width, and a rule on nets overrides the per-class-and-layer widths of impedance rules.
* `pads[].x/y` is the pad centre as the router uses it; for custom-shaped pads (JP1 of `pic_programmer`)
  this is 0.25 mm from KiCad's pad position. KiCad's DSN export also renames duplicate pad names
  (`HOLE`, `HOLE@1`) and drops mounting holes without copper (both in [AI_DATA_FORMAT.md](AI_DATA_FORMAT.md)).
* The demo boards come without a library table, so KiCad's DRC reports `lib_footprint_issues`
  warnings and `drc_status` is `failed` with 0 errors. Read `errors` / `warnings`.

Not run: KiCad 9 and 10.

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
* `crates/fastroute/tests/real_kicad.rs` — the flows above against an installed KiCad (headless
  route, real `kicad-cli pcb drc`, DRC coordinates against `inspect.json`); each test says why and
  passes when there is no KiCad. `FASTROUTE_KICAD_PYTHON`, `FASTROUTE_KICAD_CLI` and
  `FASTROUTE_KICAD_DEMO` point them at one; with KiCad 7 they check the up-front refusals instead.
* the tests of `render`, `inspect`, `ai-pack` and `rules` check the same output rules, exit codes
  and messages for their commands.
