# Boot, reset and strapping pins

Read this when a device has pins that are sampled at power-up, a reset or enable input, or a download
mode. These are the rules a general GPIO table misses, and a board that breaks them looks fine on paper
and does not start.

## What to look for in the documents

| Topic | Where it usually is | What to write down |
| --- | --- | --- |
| Strapping pins | datasheet, section on strapping / boot configuration | pin, when it is sampled, the default level without external parts, what a level selects |
| Reset / enable | datasheet pin table, hardware design guidelines | pin, active level, the required RC or supervisor if the document says so |
| Boot / download mode | datasheet, boot configuration table; the chip's technical reference | pin, the level that selects the download mode, what else must be true |
| Required circuit | hardware design guidelines, "minimum system" | the parts and values, as a `required_block` |
| Pins used inside a module | module datasheet | `reserved_pin` for the variant that is on the board |

Read the variant on the board. A module with PSRAM reserves pins that the same chip in another module
leaves free; the similar part number in the search results is not the part.

## Which rule kind

- `strapping`: a pin that is read at reset. Fixed, with an explicit quotation from the official
  source. `external_drive_allowed: false` when the document says the pin must not be driven by something
  that can be active at power-up.
- `reset_pin`: the reset or enable input. `active_level` is what holds the device in reset.
- `boot_pin`: the pin that chooses the download mode. `boot_level` is the level for download.
- `required_block`: parts the document requires around a pin. Describe the block once in
  `peripheral_blocks` (components with value limits, the pins and nets they connect to); the rule only
  refers to it.
- `reserved_pin` / `nc_pin`: pins that must have no connection and no function.

A reference board is evidence of what one board does, not of what the chip needs. Put what the reference
board does into a `configurable` or `optional` rule with `basis: observed`.

## What the validator checks

- Strapping: the pin is not tied hard to a supply or ground net, not driven by a chip or connector
  (unless `external_drive_allowed`), and not assigned a GPIO function when external drive is not allowed.
- Reset: the pin is connected, not on a floating or unconnected net, and not tied to the level that holds
  the device in reset.
- Boot: the pin has a net, the net is not a rail, and a switch or connector is on it, so that a person
  or a programmer can select the download mode.
- Required blocks: each component of the block is on the nets the block hangs on, once, and inside the
  value limits.

Net names are matched by pattern (`*`, `?`; a sheet path before the last `/` is ignored). Supply and
ground nets are recognised by name (`GND`, `AGND`, `VDD`, `3V3`, `5V`, ...); a design that names them
differently gives `ground_nets` / `power_nets` in the rule's `params`.

## What it does not check

Timing (how long reset is held, power-up order), the values of pull-up and pull-down resistors as a
circuit, and anything that needs a simulation. Say so in the pack with an `electrical` rule or in the
rule's `notes`; do not leave the impression that the validator covers it.

## When the documents say nothing

Write a rule of kind `unknown` and say what is missing and which documents were read. Do not use the
value a similar chip uses and do not use the value a reference board happens to use.
