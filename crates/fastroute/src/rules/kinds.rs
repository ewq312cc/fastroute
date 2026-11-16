//! What a rule can be about, what its parameters are, and whether fastroute (the engine, the
//! KiCad plugin around it) can really enforce it. This table is the one source of truth for the
//! checker, the draft generator, `rules apply` and the mapping table in `docs/AI_RULES.md`.

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    /// `rules apply` writes a fastroute input for it (a pairs or tune file).
    Supported,
    /// Applied through KiCad's rules (`.kicad_dru`) by the KiCad plugin, or by a plugin tool.
    ViaKicad,
    /// The engine has the primitives, but no rule file / option reaches them yet: new core work.
    NeedsCore,
    /// The engine cannot do it.
    Unsupported,
}

impl Capability {
    /// Hard / soft rules of this capability can be executed.
    pub fn executable(self) -> bool {
        matches!(self, Capability::Supported | Capability::ViaKicad)
    }
}

#[derive(Debug)]
pub struct Param {
    pub key: &'static str,
    /// `number` (positive), `string`, `strings` (array of strings) or `bool`.
    pub ty: &'static str,
}

#[derive(Debug)]
pub struct Kind {
    pub name: &'static str,
    pub capability: Capability,
    /// How it is applied, or why it is not.
    pub maps_to: &'static str,
    pub required: &'static [&'static str],
    /// At least one of these must be present (when `required` is empty).
    pub one_of: &'static [&'static str],
    pub params: &'static [Param],
    /// A value of this kind cannot be read off a board or a picture: it has to be stated by a
    /// person (quoted from a natural-language input) or the rule is `unknown`.
    pub explicit_source: bool,
}

const MM: &str = "number";

pub const KINDS: &[Kind] = &[
    Kind {
        name: "diff_pair",
        capability: Capability::Supported,
        maps_to: "`--pairs` file: `pair P N gap=<mm> skew=<mm>` (routed first, coupled, then length-matched with meanders); KiCad: `diff_pair_gap` constraint",
        required: &[],
        one_of: &["gap_mm", "skew_mm"],
        params: &[Param { key: "gap_mm", ty: MM }, Param { key: "skew_mm", ty: MM }],
        explicit_source: false,
    },
    Kind {
        name: "length_match",
        capability: Capability::Supported,
        maps_to: "`--tune` file: `group NAME tolerance=<mm> [target=<mm>]` followed by the net names (meanders after routing)",
        required: &["tolerance_mm"],
        one_of: &[],
        params: &[Param { key: "tolerance_mm", ty: MM }, Param { key: "target_mm", ty: MM }],
        explicit_source: false,
    },
    Kind {
        name: "trace_width",
        capability: Capability::ViaKicad,
        maps_to: "`.kicad_dru` `track_width` constraint (the plugin carries per-net / per-layer widths into the DSN); no command-line option sets a width",
        required: &["width_mm"],
        one_of: &[],
        params: &[Param { key: "width_mm", ty: MM }, Param { key: "layer", ty: "string" }],
        explicit_source: false,
    },
    Kind {
        name: "clearance",
        capability: Capability::ViaKicad,
        maps_to: "`.kicad_dru` `clearance` constraint (the plugin carries class clearances into the DSN)",
        required: &["clearance_mm"],
        one_of: &[],
        params: &[Param { key: "clearance_mm", ty: MM }, Param { key: "layer", ty: "string" }],
        explicit_source: false,
    },
    Kind {
        name: "length_max",
        capability: Capability::ViaKicad,
        maps_to: "`.kicad_dru` `length (max)` constraint; the plugin lists nets that are longer after routing - the router does not enforce it",
        required: &["max_mm"],
        one_of: &[],
        params: &[Param { key: "max_mm", ty: MM }],
        explicit_source: false,
    },
    Kind {
        name: "impedance",
        capability: Capability::ViaKicad,
        maps_to: "`impedance_cli.py` of the KiCad plugin computes the trace widths from the board's stackup and writes `.kicad_dru` rules; needs the stackup. A number must be stated by a person",
        required: &["target_ohm"],
        one_of: &[],
        params: &[Param { key: "target_ohm", ty: MM }, Param { key: "differential", ty: "bool" }],
        explicit_source: true,
    },
    Kind {
        name: "layer_restriction",
        capability: Capability::NeedsCore,
        maps_to: "the engine has per-net-class active layers (DSN `use_layer`), but no rule file or option sets them: new core capability",
        required: &["allowed_layers"],
        one_of: &[],
        params: &[Param { key: "allowed_layers", ty: "strings" }],
        explicit_source: false,
    },
    Kind {
        name: "keepout",
        capability: Capability::NeedsCore,
        maps_to: "keepouts come from the design (DSN); a rule file cannot add one: new core capability",
        required: &["ref", "margin_mm"],
        one_of: &[],
        params: &[Param { key: "ref", ty: "string" }, Param { key: "margin_mm", ty: MM }, Param { key: "layers", ty: "strings" }],
        explicit_source: false,
    },
    Kind {
        name: "layer_preference",
        capability: Capability::Unsupported,
        maps_to: "no per-layer cost in the router: a preference cannot be expressed; new core capability if wanted",
        required: &["preferred_layers"],
        one_of: &[],
        params: &[Param { key: "preferred_layers", ty: "strings" }, Param { key: "avoid_layers", ty: "strings" }],
        explicit_source: false,
    },
    Kind {
        name: "current_capacity",
        capability: Capability::Unsupported,
        maps_to: "no current / temperature model; give the width as a `trace_width` rule. A number must be stated by a person",
        required: &["current_a"],
        one_of: &[],
        params: &[Param { key: "current_a", ty: MM }, Param { key: "max_temp_rise_c", ty: MM }],
        explicit_source: true,
    },
    Kind {
        name: "timing_delay",
        capability: Capability::Unsupported,
        maps_to: "the router matches lengths, not delays (propagation depends on the stackup). A number must be stated by a person",
        required: &[],
        one_of: &["max_delay_ps", "skew_ps"],
        params: &[Param { key: "max_delay_ps", ty: MM }, Param { key: "skew_ps", ty: MM }],
        explicit_source: true,
    },
];

pub fn kind(name: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|k| k.name == name)
}

/// Keys that would carry routing geometry. A draft never contains any, anywhere.
pub const GEOMETRY_KEYS: &[&str] = &[
    "x", "y", "xy", "pos", "position", "points", "point", "polyline", "polygon", "path", "paths", "waypoints", "vertices", "coordinates", "coords", "segments", "traces", "trace",
    "vias", "via", "wires", "wire", "geometry", "route", "routes", "ses", "session",
];
