# High-speed interfaces

Read this when a device has USB, differential clocks, source-synchronous buses (SDIO, SPI at high
clock, RGB / LCD), RF, or any interface whose document gives routing limits.

## What to look for

The numbers that matter, and where they are given:

| Parameter | Typical wording | Param name (unit in the key) |
| --- | --- | --- |
| Differential impedance | "90 Ω differential" | `impedance_ohm` |
| Pair spacing | "keep the gap at ..." | `gap_mm` |
| Intra-pair skew | "length difference below ..." | `skew_mm` |
| Group length matching | "the lines must match within ..." | `length_match_tolerance_mm` |
| Maximum length | "the trace must be shorter than ..." | `max_length_mm` |
| Minimum width | "at least ... wide" | `min_width_mm` |
| Layers | "route on the top layer" | `layers` |

Impedance and spacing depend on the stackup. A document that gives an impedance but not a width has
not given a width; a width computed from a stackup you assumed is not a value from the document. Write
`impedance_ohm` and leave `gap_mm` as `null` with a `missing` text, in a rule that is `unknown`.

Many designs follow a reference board's values. Record them as `basis: observed` in a `configurable`
rule so that nobody mistakes them for a requirement of the chip.

## One rule per interface

```yaml
- id: C-USB
  kind: high_speed
  modifiability: configurable
  params: { nets: [USB_DP, USB_DM], differential: true, impedance_ohm: 90, skew_mm: 0.5 }
```

Put the nets by name or pattern. The same rule is used by the validator, and (for a differential pair
and a length-matching group) handed to the router.

## What each tool does with it

| Parameter | Validator | fastroute |
| --- | --- | --- |
| `skew_mm` (differential) | checked on the routed lengths | `--pairs` (`skew=`) |
| `gap_mm` (differential) | not checked (needs geometry of both nets) | `--pairs` (`gap=`) |
| `length_match_tolerance_mm` | checked: spread of the routed lengths | `--tune` group |
| `max_length_mm` | checked | not enforced by the router; KiCad's DRC checks it |
| `min_width_mm` | checked | not handed over |
| `layers` | checked | not handed over (no way to restrict a net to layers by a rule file) |
| `impedance_ohm` | reported as not checked | not handed over |

A rule that has nothing the validator looks at is reported as `unchecked`, never as a pass. An
interface that is not in the design is `not applicable`. A pair that is not routed yet is `unchecked`.

## Do not

- Do not write a typical value (USB 90 Ω, "length match within 5 mil") because it is well known. Put it
  in the pack only if a document of the device says it, with the quotation.
- Do not take a rule for a different device with a similar name (the `-U` variant, an older revision).
- Do not turn a recommendation into a requirement. "Should" in the document is `configurable`.
