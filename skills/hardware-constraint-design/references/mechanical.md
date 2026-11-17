# Mechanical and keepout constraints

Read this when a device comes with an antenna, a board outline, mounting holes, a connector that sits
in a fixed place, or an area that must stay free of copper.

## Regions

Mechanical rules refer to a region in the pack's `regions` list. A region has an id, a kind, a polygon
or a circle in millimetres (the coordinate system of the design snapshot: origin at the board's
reference, y down), and the layers it applies to.

| Region kind | Shape | Used by |
| --- | --- | --- |
| `antenna_keepout` | polygon | `keepout` |
| `rf_keepout` | polygon | `keepout` |
| `board_outline` | polygon | `mechanical_fixed` |
| `mount_hole` | circle (`center`, `radius_mm`) | `mechanical_fixed` |
| `connector` | polygon | `mechanical_fixed` |
| `module_body` | polygon | `mechanical_fixed` |

The numbers come from the mechanical drawing or the dimension table of the document. A drawing that
cannot be read as text (a figure with dimensions in an image) is read by a person: write the region as
`unknown` with a `missing` text that says where the drawing is, and do not guess the dimensions.

## Rules

- `keepout`: what must not enter the region. `forbid` lists `track`, `via`, `component`, `zone`. The
  validator tests a track together with its width, and a zone's filled copper if the snapshot has it;
  the layers of the region restrict the test.
- `mechanical_fixed`: what must stay as it is. A mount hole must exist with the centre and radius of the
  region (0.05 mm tolerance). A board outline must have the polygon's vertices. A connector must be in
  its region. A module body must be inside the board outline.

## What is not checked

- A keepout that applies "under the module" on inner layers needs the stackup; give the layers
  explicitly or leave it to KiCad's DRC with a keepout area.
- Tolerances of the manufacturing process.
- 3D clearances (height of parts, enclosure).

## Handing over

The router has no keepout input in this version; the validator checks the result after routing and
KiCad's own DRC checks it again if the keepout is also drawn as a rule area. `fastroute constraints
export-fastroute` lists keepouts under what it did not hand over, with the reason.

## When the document is a reference design

A reference board's outline and holes are `observed`. They belong in rules that are `configurable` or
`optional`, and in `base_design` (see `hcp-format.md`) if the task is to build on that board.
