# Rule drafts: from requirements to checked, confirmed routing rules

People want to say "route the USB pair at 0.15 mm gap" or "make the supply traces wider", and
AI models can help turn a reference board, the current board and such sentences into rules.
This document describes how fastroute lets that happen **without trusting the AI**: the model (or
a person, or a script) writes a *draft*; fastroute checks it with deterministic rules, a person
confirms it, fastroute turns the confirmed part into inputs for the router, and the result is
judged by KiCad's real design rule check — never by what the model says about it.

```
  requirements ─┐                       ┌────────────┐   ┌──────────────┐   ┌─────────────┐
  current board ┼─► draft (rules JSON) ─► rules check ─► person confirms ─► rules apply   │
  reference     │   by a person, a      │ offline,    │   │ names the      │ pairs / tune /│
  board (data)  ┘   script or an AI     │ deterministic│  │ draft by SHA-256│ .kicad_dru   │
                                        └────────────┘   └──────────────┘   └──────┬──────┘
                                                                                    ▼
                          accept only if drc_status is "passed"  ◄── ai-pack with a real kicad-cli DRC ◄── route
```

fastroute itself is offline: the checker, the draft generator and `apply` never touch the
network, and no AI service is built in or required. An AI model is an *external tool* that writes
a draft file; where it runs and what it is shown is your decision (a local model works as well as
a hosted one). Nothing forces uploading a customer's design anywhere, and no single provider is
assumed.

## What is guaranteed, and what stops what

| Could go wrong | What happens |
|---|---|
| The model **loosens a limit the design has** (clearance 0.2 → 0.1 mm, a thinner trace, a longer maximum length) to "make routing easier" | The checker compares every clearance / width / length rule with the design's real limit. A loosening without a `relaxes` declaration is rejected (`silent_relaxation`); a declaration must state the real baseline (`relaxation_mismatch`); and `rules apply` needs the person to accept that rule's loosening by name (`accepted_relaxations`). |
| The model **invents an impedance, a current or a delay** | These values cannot be read off a board or a picture. A rule with such a value needs a quote of a person's words that contain the number (`value_without_source`, `value_not_in_quote`); otherwise it must be `unknown`, carry **no value** and list what is `needed` (`unknown_with_value`). |
| The model **claims a rule is in effect that the engine cannot enforce** (layer preference, current capacity, timing) | A hard or soft rule of a kind the engine cannot enforce is rejected (`cannot_enforce`): it has to be `unsupported`. Unsupported and unknown rules are recorded and **never executed**, and `rules apply` lists each of them as "not executed". |
| The model puts **coordinates, traces or vias** into the answer | Any geometry key anywhere in a draft is rejected (`geometry_in_draft`), and unknown parameters are rejected. A draft contains rules only; there is no path from a draft to wiring. |
| The model **cites words that are not in the requirement** | Every quote must be found verbatim in the input text it names (`quote_not_found`); design references (`nets[NAME]`, `layers[NAME]`) must exist in the design. |
| A net, layer, class or component **does not exist** | `unknown_net`, `unknown_layer`, `unknown_net_class`, `unknown_component`. |
| The draft was made for **another board** or an older state of it | The draft names the design by hash; a different `inspect.json` is refused (`design_mismatch`). |
| The draft **changes after a person looked at it** | The confirmation names the draft by SHA-256; any change voids it. |
| **Nobody decided** | `rules apply` refuses without a confirmation, without a name, and unless every rule is either accepted or acknowledged as not executed. |
| A hostile net name **injects text into a rule file** | Names with whitespace, quotes, brackets or wildcards are not written into pairs / tune / `.kicad_dru` files; the apply fails instead. |
| The model (or this tool) says **"the board is fine"** | Nothing here can say that. Acceptance is the DRC status of the routed board, from a real `kicad-cli` report ([AI_PACK.md](AI_PACK.md)). `apply-report.json` states this itself. |

The tool-level gates protect against accidents and over-confident models. They are not an
identity system: a pipeline that fills in the confirmation by itself has removed the human, and
nothing in fastroute can detect that. Keep the confirmation step a person's step.

## What a picture cannot tell

A routed board, a screenshot or an image from `fastroute render` shows **where copper is**. It
cannot show:

* **impedance** — it depends on the stackup (layer heights, dielectric constant, copper
  thickness, solder mask), none of which is visible, and on the trace geometry to a precision no
  picture has;
* **current capacity** — it depends on copper weight, allowed temperature rise, ambient and
  airflow;
* **timing** — it depends on propagation delay in the actual dielectric, not just on length;
* **intent** — whether two close traces are a pair, a bus or a coincidence.

So a rule that needs one of these values is either stated by a person (and quoted), or it is
`unknown` with a list of what is needed. The structured data of [inspect.json](AI_DATA_FORMAT.md)
(nets, classes, widths, clearances, pad positions) is evidence for *structure* — which nets exist,
what the design already demands — not for electrical intent.

## The rule format

Format `fastroute-rule-draft` 1.x, schema [`docs/schema/rule-draft-1.0.schema.json`](schema/rule-draft-1.0.schema.json)
(also `fastroute rules schema`, so that a tool that generates drafts can constrain its output).
Minor versions only add properties; another major version is refused.

```jsonc
{
  "format": "fastroute-rule-draft", "format_version": "1.0",
  "created_by": { "kind": "ai", "tool": "any text", "model": null },       // human | ai | script
  "design": { "input_sha256": "…", "inspect_sha256": "…" },                  // which board it is for
  "inputs": [                                                                 // everything the rules come from
    { "id": "nl1",  "kind": "natural_language", "text": "USB D+/D- must be a differential pair, gap 0.15 mm." },
    { "id": "pack", "kind": "ai_pack", "path": "pack/", "sha256": "…" }
  ],
  "rules": [
    { "id": "R1", "kind": "diff_pair", "strength": "hard",
      "applies_to": { "nets": ["USB_DP", "USB_DN"] },                        // nets | net_class | all
      "params": { "gap_mm": 0.15 },
      "evidence": [ { "input": "nl1", "kind": "natural_language", "quote": "differential pair, gap 0.15 mm" },
                    { "input": "pack", "kind": "design_data", "ref": "nets[USB_DP].class" } ] }
  ]
}
```

### Strength: hard, soft, unknown, unsupported

| `strength` | Meaning | Value | Executed |
|---|---|---|---|
| `hard` | must hold; a violation fails the board | required, in mm / ohm / A / ps | if the engine can enforce it and a person accepts it |
| `soft` | a preference the router may trade off (`weight` optional) | required | same |
| `unknown` | the value **cannot be determined** from the inputs | **none** (`params` empty), `needed` lists what is missing | **never** |
| `unsupported` | the engine cannot enforce it; the value is kept as a record | optional | **never** |

Hard and soft constraints are separate on purpose: a soft rule can never replace or weaken a hard
one, and a hard rule is only ever *tightened* silently — loosening has its own declaration.

### Evidence

Every rule has at least one `evidence` entry, pointing at an input:

| `kind` | Input kind | Required | Checked |
|---|---|---|---|
| `natural_language` / `human` | `natural_language` | `quote` | found verbatim (whitespace and case ignored) in the input's `text` |
| `design_data` | `ai_pack` / `inspect_json` | `ref` (`nets[NAME]…`, `layers[NAME]…`) | the net / layer exists in the design |
| `reference_board` | `reference_board` | `ref` | counts for structure only: **never** a source for impedance, current or timing values |

### Loosening what the design demands

`clearance`, `trace_width` and `length_max` rules are compared with the design's limit for the same
nets (the maximum clearance / width of their net classes, the minimum maximum length). Tightening
is fine. Loosening must be declared:

```jsonc
"relaxes": { "parameter": "clearance_mm", "baseline": 0.2, "reason": "needed to fit the 0.15 mm pair" }
```

`baseline` must equal the design's real value (checked), and the person must list the rule in
`accepted_relaxations` when confirming.

### Kinds and what they map to

The table is the output of `fastroute rules capabilities`; a test keeps it in sync with the code.
Capabilities: `supported` — `rules apply` writes a fastroute input; `via_kicad` — applied through
KiCad's rules / the plugin's tools; `needs_core` — the engine has the primitives but nothing
reaches them yet (new core work); `unsupported` — the engine cannot do it.

| Kind | Capability | Parameters | Applied as, or why not |
|---|---|---|---|
| `diff_pair` | `supported` | `gap_mm` and/or `skew_mm`; exactly two nets | `--pairs` file: `pair P N gap=<mm> skew=<mm>` (routed first, coupled at the gap, then length-matched with meanders); KiCad: `diff_pair_gap` |
| `length_match` | `supported` | `tolerance_mm`, `target_mm`; ≥ 2 nets | `--tune` file: `group NAME tolerance=<mm> [target=<mm>]` and the nets (meanders after routing) |
| `trace_width` | `via_kicad` | `width_mm`, `layer` | `.kicad_dru` `track_width` constraint: KiCad's DRC checks it; the plugin hands a width to the router only for an `A.NetClass == 'X'` rule with a `(layer ...)` that matches a width of the board's net classes, and prints `NOT applied` for every other form (seen with KiCad 8.0.9); no command-line option sets a width |
| `clearance` | `via_kicad` | `clearance_mm`, `layer` | `.kicad_dru` `clearance` constraint: KiCad's DRC checks it; the plugin does not translate `clearance` rules for the router (it prints `NOT applied`), which keeps the clearances of the board's net classes (seen with KiCad 8.0.9) |
| `length_max` | `via_kicad` | `max_mm` | `.kicad_dru` `length (max)`; the plugin lists nets that are longer after routing — the router does not enforce it |
| `impedance` | `via_kicad` | `target_ohm`, `differential` | `impedance_cli.py` of the KiCad plugin computes the widths from the stackup and writes `.kicad_dru` rules; `rules apply` reports it as a manual step with the command. **A person must state the number** |
| `layer_restriction` | `needs_core` | `allowed_layers` | the engine has per-net-class active layers (DSN `use_layer`) but no rule file or option sets them |
| `keepout` | `needs_core` | `ref`, `margin_mm`, `layers` | keepouts come from the design (DSN); a rule file cannot add one |
| `layer_preference` | `unsupported` | `preferred_layers`, `avoid_layers` | the router has no per-layer cost; a preference cannot be expressed |
| `current_capacity` | `unsupported` | `current_a`, `max_temp_rise_c` | no current / temperature model; give the width as a `trace_width` rule. **A person must state the number** |
| `timing_delay` | `unsupported` | `max_delay_ps`, `skew_ps` | the router matches lengths, not delays. **A person must state the number** |

## Human confirmation

Before anything is executed, a person decides, in a file:

```sh
fastroute rules check draft.json --ai-pack pack/                       # 1. is the draft valid? (exit 3 if not)
fastroute rules confirm-template draft.json --ai-pack pack/ -o confirmation.json
$EDITOR confirmation.json                                              # 2. read it; put your name on it
fastroute rules apply draft.json --confirmation confirmation.json --ai-pack pack/ -o rules-out/
```

The confirmation (`fastroute-rule-confirmation`, [schema](schema/rule-confirmation-1.0.schema.json))
holds the draft's SHA-256, `confirmed_by.name` (empty in the form on purpose), `accepted_rules`,
`accepted_relaxations` and `acknowledged_not_executed`. `apply` demands that **every rule** of
the draft is in exactly one of the two lists — nothing is executed by default and nothing is
silently dropped. The decision points, in order:

1. the draft is read and checked (errors are listed with the path and what to do);
2. each rule that would be executed is accepted by name;
3. each loosening of the design is accepted by name;
4. each rule that will **not** be executed — unknown, unsupported, needs core work — is
   acknowledged, so nobody believes it is in effect;
5. after routing, the DRC result decides whether the board is accepted.

`apply` writes, into a directory and only for accepted rules: `pairs.txt`, `tune.txt`,
`rules.kicad_dru`, `route-args.txt` and `apply-report.json` (what was executed, what was not and
why, the accepted loosenings, manual steps, the next commands). It runs nothing and routes nothing.

## The prototype: a draft from written requirements, offline

`fastroute rules draft` turns text (and the design) into a draft without any AI: a transparent
keyword grammar that quotes the sentence of every rule and never guesses.

```sh
fastroute rules draft --text-file requirements.txt --ai-pack pack/ -o draft.json
```

| Words | Rule |
|---|---|
| *differential*, *diff pair* + `gap`, `skew` | `diff_pair` for the pair of the design (`USB D+/D-` → `USB_DP`/`USB_DN`); no pair found, or no value → `unknown` |
| a number + *ohm* | `impedance` with that number, quoted; *impedance* without a number → `unknown` (needs the value and the stackup) |
| *width*, *wide*, *widen* + mm / mil / µm | `trace_width` for the named nets (*power* is read as nets that look like supplies, and says so); no value → `unknown` |
| *clearance*, *spacing* | `clearance`; a value below the design's limit is declared with `relaxes` |
| *max length*, *no longer than* | `length_max` for the named nets |
| *prefer*, *only on* + layers | `layer_preference` / `layer_restriction`, recorded as `unsupported` |
| *carry … A*, *current* | `current_capacity`: `unsupported` with the stated number, `unknown` without |
| *timing*, *delay*, *high-speed* | `timing_delay`: `unsupported` with a stated ps / ns, `unknown` without |

*must / shall / never / required* → hard; *prefer / try / should / ideally* → soft; a plain
numeric limit is hard. Sentences it does not understand are listed as notes, not turned into
rules. The generator's output passes through the same checker as everything else; it is a
reference implementation of the format, and the examples in
[`docs/examples/rules/`](examples/rules/) show every case: [USB differential pair](examples/rules/usb-diff.draft.json),
[wider supply traces](examples/rules/power-widen.draft.json), [layer preference](examples/rules/layer-preference.draft.json)
(unsupported), [insufficient information](examples/rules/insufficient-info.draft.json) (unknown),
[a request to loosen a hard constraint](examples/rules/relax-hard-constraint.draft.json) (declared),
and the drafts an AI model could write wrongly, each rejected with its reason
(`bad-*.draft.json`).

### Using an AI model

1. Give the model the output of `fastroute rules schema`, the [ai-pack](AI_PACK.md) (pictures
   and `inspect.json`) and the requirements. Where it runs is up to you.
2. Take its answer as `draft.json` and run `rules check`. The errors have codes, paths and
   messages meant to be fed back to the model ("this loosens clearance from 0.2 to 0.1: declare
   it with `relaxes`").
3. A person confirms; `rules apply` writes the inputs; route; run `ai-pack` with `--kicad-pcb`
   and `--require-drc`; accept the board only if `drc_status` is `passed`.

## What the engine will and will not take from an AI

* A draft is rules. It has no way to carry wiring, and the checker rejects geometry anywhere.
* The only way wiring enters fastroute is a session (`.ses`) or a KiCad board. If a model produced
  one, it is untrusted data like any other: `route --initial-session` treats it as a start that
  may be ripped up and re-optimized, and `inspect` / `render` / `ai-pack` measure it with the
  router's own clearance check and KiCad's DRC. Nothing marks such wiring as verified.
* The status of a check is only ever a statement about a check that ran: `not_run`,
  `unavailable`, `timeout` and `error` are never `passed`.

## Limits

* The generator reads English (and a few Chinese words) with keywords; it is a prototype of the
  format and the gates, not a language model. A sentence it cannot read yields a note.
* `rules apply` can write pairs and tune files for the engine and `.kicad_dru` rules for the KiCad
  plugin. The `.kicad_dru` syntax follows KiCad's custom-rules documentation; it was **not**
  loaded into KiCad in the environment this was developed in (no KiCad there), so check the file
  in KiCad before relying on it. The pairs and tune files are read by the engine in the tests.
* Layer restrictions, layer preferences and keepouts from rules need new capabilities in the core
  router; they are recorded as `unsupported` / `needs_core` until then.
* Impedance is only a manual step: `impedance_cli.py` needs KiCad's Python and the board's stackup.

## Tests

`crates/fastroute/tests/rules_cli.rs`: the five scenarios of the prototype (USB differential
pair, wider supply, layer preference, insufficient information, a request to loosen a hard
constraint) with their expected rules; every misuse in `docs/examples/rules/bad-*.draft.json`
rejected with its code; the checks against the design (nets, layers, hashes, damaged packs);
the confirmation gates (no name, wrong draft hash, undecided rules, unaccepted loosening,
accepting a rule that cannot be executed); that unknown and unsupported rules produce no file;
that the written pairs and tune files are accepted by the router; output protection; that the
module stays offline; that the examples are what the tools write; and that the table above
matches `fastroute rules capabilities`.
