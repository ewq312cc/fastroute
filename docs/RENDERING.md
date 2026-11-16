# Offline rendering (`fastroute render`)

`fastroute render` draws a board — the whole board or one region of it — as a PNG. It runs
without a desktop, browser, network or font files, reads the board only, and gives the same
pixels for the same input, options and version. It is meant for people and for AI tools that
need a picture they can trust: every image comes with the exact region, scale and coordinate
transform (see [AI_DATA_FORMAT.md](AI_DATA_FORMAT.md)).

```sh
fastroute render -i board.dsn -s routed.ses -o board.png --size 4096
fastroute render -i board.dsn --around U1 --padding-mm 3 --px-per-mm 80 -o u1.png
fastroute render -i board.dsn --net USB_DP --net USB_DN --layers F.Cu -o usb.png
fastroute render -i board.dsn --from-report drc.json --select DRC1 -o drc1.png
```

## Design

**What is drawn from.** `fastroute` loads the Specctra design (`.dsn`), optionally with a
session (`.ses`) on top, exactly as `route` loads it, then copies the geometry into a plain
model in millimetres (`fr_render::BoardGeometry`): outline, pads (per layer), traces, vias,
planes, keepouts, unrouted connections, components and numbered issues. The renderer
(crate `crates/fr-render`) knows nothing about the routing engine; it only rasterises that
model with [tiny-skia](https://crates.io/crates/tiny-skia) (pure Rust, anti-aliased paths;
built without SIMD so that the pixels do not depend on the CPU).

**Why not reuse `live.rs`.** `--live` sends board units, Specctra's y-up axis and a
compact array format that exists for the browser page and may change with it. The renderer
needed millimetres, KiCad's y-down axis, pad names, component outlines and a stable meaning,
so the geometry is extracted once into the shared model above instead. `live.rs` is not
touched and `render` does not depend on it, on a browser or on a socket.

**Sessions replace, they do not add.** A session is the complete routing result: the session
writer emits every trace and via, including the pre-routed ones of the design (as
`(type protect)`). Loading design + session therefore *removes* the design's own traces and
vias (except `(type fix)` ones) before importing the session. Adding them on top would put
every via twice.

**Not shown / limits.** Silkscreen, drill holes (the engine does not keep a drill size) and
holes inside planes (a plane is drawn as its outline polygon). Holes in the board are part of
the outline. Labels are upper case (the
built-in 5x7 font has no lower case); characters outside ASCII are drawn as an empty box. The
exact names are always in the JSON next to the image.

## Coordinates

* Millimetres. x to the right, **y downwards**, as in KiCad and in `--report`.
* The origin is the origin of the DSN (for a KiCad export: the board origin). The y values are
  the DSN's y negated, so a design that does not come from KiCad (y up) has negative y.
* `--region-mm=x,y,w,h`: `x,y` is the **top-left** corner (smallest x, smallest y).
* Pixel `(u, v)`: origin at the top-left *corner* of the image, `u` right, `v` down, pixel
  centres at +0.5. World and pixel axes are not flipped against each other:

  ```
  u = (x - region.x) * px_per_mm        v = (y - region.y) * px_per_mm
  ```

The manifest holds both directions (`transform.world_to_pixel_u`, `…_v`, `pixel_to_world_x`,
`…_y`; each `{scale, offset}`), so a consumer never has to derive them.

## Size and scale are independent

| Option | Meaning |
|---|---|
| `--size N` | the longer image side has N pixels (default 2048); the scale follows |
| `--size WxH` | exactly that many pixels; the region grows on one axis to fit the aspect ratio |
| `--px-per-mm S` | fixed scale; the image size follows from the region |

The same region at two scales puts every object at the same world position. The region that
is really shown (`region_mm` in the manifest) always contains the requested one and is centred
on it; it can be a little larger because an image has whole pixels. Limits: 16384 pixels per
side and 100 megapixels, otherwise a clear error.

## Selecting the area

Exactly one of:

| Option | Area |
|---|---|
| *(none)* | the board outline plus a 2 % margin |
| `--region-mm=x,y,w,h` | the rectangle, as given |
| `--around=REF --padding-mm=N` | the whole package of the component (pads, outline, component keepouts) plus N mm (default 2) |
| `--net=NAME` (repeat for a differential pair) | every pad, trace, via and unrouted connection of the net(s) **on the selected layers**, plus N mm. Planes count only with `--net-planes` (a pour would select the whole board) |
| `--from-report=FILE [--select=WHAT]` | the locations of problems in a report (below), plus N mm (default 3) |
| `--select=WHAT` | the same for the board's own problems (`OPEN…` unrouted connections, `CLR…` clearance violations) |

Nothing is cut: geometry that crosses the region border is drawn up to the border. Objects
that merely touch the region are listed in the manifest (`contains`).

Unknown components, nets, layers and issues, empty or invalid rectangles, and nets without
geometry on the selected layers are errors (exit code 2) that name what is missing and, where
possible, similar names.

### Reports

`--from-report` reads, by content:

* fastroute's `--report=FILE` JSON (`unrouted`, `clearance_violations`),
* `inspect.json` (keeps its ids),
* KiCad's `kicad-cli pcb drc --format json` (`violations`, `unconnected_items`; the issues get
  the ids `DRC1…` and are drawn as markers). KiCad's board coordinates are taken as the world
  coordinates, which holds for the DSN that KiCad itself exports.

`--select` is `all`, `unrouted`, `violations`, `drc` or a list of ids such as `OPEN1,CLR3`.
Issue ids are `OPEN<n>` (unrouted connection), `CLR<n>` (fastroute's clearance check) and
`DRC<n>` (external checker). They are deliberately not `U1`/`D1`/`V1`, which are component
references on real boards.

## Layers

`--layers` takes layer names (`F.Cu,B.Cu`, any case), `signal`, `top`, `bottom` or `all`
(default). One layer gives a single-layer image, several are overlaid with the top layer on
top. A via is shown on every layer it spans, a through-hole pad on each of its layers.
Unrouted connections are drawn in every image unless `--no-airlines`.

Colours (fixed): top layer red, bottom blue, inner layers green, yellow, violet, cyan, orange,
grey; pads are a lighter tone of their layer, vias light grey with a dark centre, the outline
yellow, keepouts dashed magenta, unrouted connections dashed white; dark background.

## Annotations

`--annotate=LIST` (default `scale,layer,markers`; `none`, `all`):

| Name | Effect |
|---|---|
| `scale` | scale bar with a 1/2/5 × 10ⁿ mm length |
| `layer` | layer names, top right |
| `grid`, `grid=MM` | millimetre grid (auto step ≈ 60 px, or a fixed step ≥ 4 px) with coordinates along the top and left edge |
| `nets` | net names on traces (at most 3 per net) |
| `pads` | pin names on pads that are large enough |
| `refs` | component references |
| `markers` | numbered markers for the issues in the image |

Labels never overlap: a label that would cover an earlier one is left out (priority: layer,
scale, grid, markers, refs, pads, nets), and `labels.skipped_overlap` / `skipped_small` in the
manifest say how many. Thin lines never vanish: nothing is drawn thinner than one pixel.

## Output files

`-o FILE` is written through a temporary file and a rename. An existing file is refused
unless `--force`; the input design and session are never overwritten, also not with
`--force` and not under another spelling of the same path. `--manifest FILE` writes the
manifest of the image (`-` = standard output; then the summary line is not printed).
`-q` prints nothing on success.

Exit codes: 0 done, 2 usage (bad or conflicting options, unknown names), 3 input (missing or
unreadable design/session/report, a KiCad board instead of a DSN), 4 output (exists without
`--force`, is the input, directory missing), 1 anything else (e.g. a design without a board
outline).

## Tests

* `crates/fr-render`: unit tests for the view maths, region selection, font and annotations;
  `tests/render.rs` draws hand-made boards: whole-board bounds, layer filtering, thin lines,
  byte-identical output, geometry crossing the border, the same region at two scales, metadata
  round trip, label collisions, errors.
* `crates/fastroute/tests/render_cli.rs`: the binary on `testdata/render_demo.dsn` and a
  millimetre design, with an empty `PATH` and no `DISPLAY`: bounds and units, layers, regions
  (rectangle, component, nets, reports), scale independence through the manifest transform,
  sessions replacing wiring, overwrite protection, every error class and exit code, `--quiet`,
  `--help`.
