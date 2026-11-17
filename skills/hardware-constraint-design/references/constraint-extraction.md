# Extracting constraints into a pack

The aim is a pack in which every rule says **what** is required, **under which conditions**, **how
firmly** (`fixed`, `required`, `configurable`, …) and **where it is written** — and in which what
is not known stays visibly unknown. You read the original text; tools only help you find it.

Contents: [Evidence levels](#evidence-levels) · [The procedure](#the-procedure) ·
[Checklist by topic](#checklist-by-topic) · [Rules of thumb](#rules-of-thumb) ·
[Worked cases](#worked-cases) · [Retrieval aids](#retrieval-aids-keyword-and-vector) ·
[The report](#the-report-and-what-it-must-answer)

## Evidence levels

Every piece of evidence has a `basis`; choose it honestly, the checker enforces what it can.

| `basis` | Use it when | Never use it for |
|---|---|---|
| `explicit` | the document says it, and you quote the sentence | something you concluded |
| `derived` | it follows from two or more statements; set `derived_from` and say how in the rule's `title` | a quotation |
| `observed` | a reference board, example or dev kit does it | a requirement of the chip (`fixed` / `required`) |
| `pending` | you suspect something and could not confirm it | anything that is executed — use `unknown` instead |

A `fixed` or `required` statement about the chip needs an **explicit** quotation from an
**official** source. The reference board is for configurable choices, mechanics and editable areas.

## The procedure

1. **Lock the device and the documents** (see `document-research.md`): exact part number,
   revision, document versions. No exact manual → stop; list the gap.
2. **Collect candidates** with `hcp-manager search … --device <exact part>` (keyword, filtered by
   device and version). Treat every hit as *a place to read*, not as a fact.
3. **Read the original page** (and the paragraph before and after: conditions and exceptions hide
   there). Note document, version, page, section, table/figure.
4. **Write the rule**: the value **with its unit and its condition**. Copy a short quotation (at
   most 400 characters) exactly; hcp-manager will check it against the page.
5. **Classify** it: `fixed` (must not change), `required` (must be present), `configurable` (a
   choice with limits), `optional`, `derived`, `ai_editable` (only with a base design and a
   whitelist), `unknown` (the documents do not say), `conflict` (they disagree).
6. **Check the pack**: `fastroute constraints check pack.yaml`, then
   `hcp-manager evidence verify pack.yaml`, then `fastroute constraints report pack.yaml`.
   Read the report as the user would: it must say what cannot be touched, what must stay, what may
   change and where it is written.
7. **Do not touch** `fixed` or `required` rules, or widen the AI-editable whitelist, while
   extracting. That is a person's decision (set `pack.review`).

## Checklist by topic

For each topic, ask the question and write down either a rule or a gap. A gap is a result.

**Pins and the module/chip**
* Which pins are reserved, internal (flash, PSRAM, antenna switch), no-connect, input-only?
  → `reserved_pin`, `nc_pin` with the table/page.
* Which pins sample a level at reset or power-up (strapping)? Their default level, when they are
  sampled, what an external pull or driver does. → `strapping` (+ `boot_pin` for the boot-mode
  pin, `reset_pin` for reset). Strapping pins are **not** free GPIOs: say so in the rule.
* Which functions can a pin have, and which cannot be used together? → pin `functions`,
  `exclusive_functions`.
* Which pins must be on which net (power, ground, enable, supply sense)? → `fixed_net`.

**Peripherals the design must have** (`peripheral_blocks` + `required_block`)
* Supply and decoupling: capacitor kinds and values, how close, which pin.
* Reset / enable circuit: pull-up value, filter capacitor, reset button.
* Boot / download: boot-mode button or jumper, the download interface (UART, USB) and the
  pins it needs, an auto-download circuit if the document requires one.
* Antenna, crystal, matching network: required parts and where they connect.
* Connectors the user must keep (programming header, USB).

**Electrical** (`electrical`)
* Supply range, current, input levels, absolute maximum ratings — each with its **condition**
  (temperature, mode, with/without radio) and unit in the key (`min_v`, `max_ma`). Limits that no
  tool checks are still written down: they are reported, not hidden.

**High-speed and routing** (`high_speed`, `clearance`, `trace_width`)
* Differential pair? Impedance (`impedance_ohm`), spacing (`gap_mm`), skew (`skew_mm`), maximum
  length, layer, reference plane, stubs, vias. **Each missing parameter stays `null`** with the rule
  `unknown`; "typical" values from experience are not evidence.
* Distinguish three origins in `title`/`basis`: what the chip requires (`explicit` from the manual),
  what the reference board does (`observed`), what you derived for this project (`derived`).

**Mechanical** (`regions`, `keepout`, `mechanical_fixed`)
* Board outline, mounting holes, fixed connectors, module body, **antenna keep-out** (which layers,
  what dimension, relative to which edge — a dimension that exists only in a drawing is `unknown`
  until a person reads it off the drawing), editable areas.
* Coordinates in millimetres, y down, in the board frame (or the device frame with `frame: device`).

**Editable scope** (`base_design`)
* What is the base board bound to (commit, file digests)? What is immutable? Which regions, blocks
  and pins may an AI use? Everything not listed is read-only.

## Rules of thumb

* **Numbers travel with their unit, condition and version.** A number alone is a bug waiting.
* **Never copy a value from a similar model** or from "the usual". If the document for this exact
  version does not give it, it is `unknown`.
* **Conflict, not choice.** Manual vs errata vs board: record each with its evidence and stop. The
  only automatic exception is an errata that says in words that it replaces a sentence.
* **A recommendation is not a requirement.** "Should", "recommended", "typically" →
  `configurable` or `optional`; "must", "shall", "do not" → `fixed`/`required` if the source is
  official and the sentence is quoted.
* **Observed behaviour stays observed.** The board connects IO0 to a button: that proves a button
  works, not that the chip requires one.
* **One rule, one fact.** Split "pull-up 10 kΩ and capacitor 1 nF" into two components of one block
  (each with its value), not one string.
* **Leave `fixed` / `required` / the whitelist alone** unless a person asked you to change them.

## Worked cases

These are also the extraction samples of the test suite (`crates/fr-constraint/tests/extraction.rs`).

**Explicit rule.** Datasheet p.5, pin table: "RSVD1 and RSVD2 are connected to the flash inside
the module; do not use."

```yaml
- id: C-RESERVED
  kind: reserved_pin
  modifiability: fixed
  subject: { device: DEV-FM100 }
  params: { pins: ["10", "11"], reason: used by the internal flash }
  evidence:
    - source_id: SRC-FM100-DS
      locator: { page: 5, table: "Pin definitions" }
      quote: "RSVD1 and RSVD2 are connected to the flash inside the module; do not use."
      basis: explicit
```

**Conditional rule.** "If USB is used, route D+ and D- as a 90 Ω pair": keep the condition.

```yaml
- id: C-USB
  kind: high_speed
  modifiability: configurable
  params: { nets: ["USB_DP", "USB_DM"], differential: true, impedance_ohm: 90 }
  applies_when: ["the USB interface is used"]
```

**The board differs from the manual.** Manual: IO0 high at reset; the board pulls it low. Write a
`conflict` with both values and both pieces of evidence; the report lists it under "conflicts and
gaps" and no executable rule is made. Writing the board's behaviour as `fixed` is refused
(`weak_evidence`).

**Data is missing.** The guide gives no pair spacing: `modifiability: unknown`, `missing: "not given;
depends on the stackup"`, `gap_mm: null`. A number there is refused (`unknown_value`).

**A retrieval tool recalled the wrong thing.** A hit from the manual of the "-1U" model, or a score
or vector pasted into the pack: the pack is refused (`source_not_applicable`, `vector_field`).
Evidence comes from documents you read, for this device.

**A similar model.** Two models share most of a name. Search and cite with the exact part number;
if the tool lists documents of other models as *excluded*, leave them out.

## Retrieval aids: keyword and vector

* Keyword search (`hcp-manager search`) is always available, offline, and filters by device,
  revision, version and level before it ranks.
* The vector layer of hcp-manager (`--mode hybrid`) may help to *find* candidate passages (a spelling
  variant, with a plugged-in model a paraphrase). The default vectors are lexical, so do not expect
  synonyms. It never replaces reading the page: its results are not evidence, its scores are not
  strength, and no pack, IR or validator reads them.
* Without usable vectors nothing changes except how candidates are found; the evidence
  requirements are the same.

## The report and what it must answer

`fastroute constraints report pack.yaml -o report.md` (Chinese by default, `--lang en`) always has
the same sections: 1 not changeable, 2 must stay, 3 configurable / optional / derived, 4 where an AI
may edit (the whitelist), 5 high-speed / mechanical / routing, 6 conflicts and gaps, 7 sources. A
reader should be able to answer from it, for any item: **can I change it? must I keep it? what is
the evidence, in which document, version and page?** Citations can be sent back to the original with
`hcp-manager evidence verify`. If the report cannot answer one of these, fix the pack, not the report.
