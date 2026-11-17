//! The design snapshot: what the validator knows about a design, as plain data (millimetres, x to
//! the right, y down). A KiCad board is turned into one by `kicad::read_pcb`; any tool can write
//! one. The validator reads nothing else about the design.

use serde::{Deserialize, Serialize};

pub const SNAPSHOT_VERSION: &str = "1.0";

pub const SNAPSHOT_SCHEMA_TEXT: &str = include_str!("../../../skills/hardware-constraint-design/schemas/design-snapshot-v1.schema.json");

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub snapshot_version: String,
    pub project: Project,
    #[serde(default)]
    pub components: Vec<Component>,
    #[serde(default)]
    pub pins: Vec<PinUse>,
    #[serde(default)]
    pub tracks: Vec<Track>,
    #[serde(default)]
    pub vias: Vec<Via>,
    #[serde(default)]
    pub zones: Vec<Zone>,
    #[serde(default)]
    pub outline: Vec<[f64; 2]>,
    #[serde(default)]
    pub mount_holes: Vec<MountHole>,
    /// What the design intends a pin to do (from the schematic or the project), optional.
    #[serde(default)]
    pub assignments: Vec<Assignment>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_kind: Option<String>,
    /// SHA-256 of the file the snapshot was made from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_sha256: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Component {
    #[serde(rename = "ref")]
    pub reference: String,
    /// The HCP device this is (`DEV-...`), when the design says so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footprint: Option<String>,
    /// resistor, capacitor, inductor, switch, led, connector, ic, other (inferred from the
    /// reference when absent)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub x: f64,
    pub y: f64,
    #[serde(default)]
    pub rotation_deg: f64,
    #[serde(default = "top")]
    pub side: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
    #[serde(default)]
    pub locked: bool,
}

fn top() -> String {
    "top".into()
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PinUse {
    #[serde(rename = "ref")]
    pub reference: String,
    pub pin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub net: Option<String>,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Track {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub net: Option<String>,
    pub layer: String,
    pub width_mm: f64,
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Via {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub net: Option<String>,
    pub x: f64,
    pub y: f64,
    pub diameter_mm: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Zone {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub net: Option<String>,
    pub layer: String,
    pub polygon: Vec<[f64; 2]>,
    /// A keep-out (rule area) of the design itself, not copper.
    #[serde(default)]
    pub keepout: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uuid: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MountHole {
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "ref")]
    pub reference: Option<String>,
    pub x: f64,
    pub y: f64,
    pub radius_mm: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Assignment {
    #[serde(rename = "ref")]
    pub reference: String,
    pub pin: String,
    pub function: String,
}

impl Snapshot {
    pub fn component(&self, reference: &str) -> Option<&Component> {
        self.components.iter().find(|c| c.reference == reference)
    }

    pub fn pin(&self, reference: &str, pin: &str) -> Option<&PinUse> {
        self.pins.iter().find(|p| p.reference == reference && p.pin == pin)
    }

    /// The kind of a component: as given, else guessed from its reference.
    pub fn kind_of(c: &Component) -> String {
        if let Some(k) = &c.kind {
            return k.clone();
        }
        let r: String = c.reference.chars().take_while(|ch| ch.is_ascii_alphabetic()).collect::<String>().to_ascii_uppercase();
        let v = c.value.clone().unwrap_or_default().to_ascii_lowercase();
        let f = c.footprint.clone().unwrap_or_default().to_ascii_lowercase();
        match r.as_str() {
            "R" | "RN" | "RV" => "resistor",
            "C" => "capacitor",
            "L" | "FB" => "inductor",
            "SW" | "S" | "BTN" | "B" => "switch",
            "D" | "LED" if v.contains("led") || f.contains("led") || r == "LED" => "led",
            "J" | "P" | "CN" | "X" | "H" => "connector",
            "U" | "IC" | "Q" => "ic",
            _ => "other",
        }
        .to_string()
    }
}

/// A component value like `100nF`, `4.7k`, `4k7`, `10u`, `0.1uF`, `220R` in base units (ohm or
/// farad); `None` if it cannot be read.
pub fn parse_value(text: &str) -> Option<f64> {
    let mut t: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if t.to_ascii_lowercase().ends_with("ohm") {
        t.truncate(t.len() - 3);
    } else if t.ends_with(['F', 'f', 'H', 'Ω']) {
        t.pop();
    }
    let at = t.find(|c: char| !(c.is_ascii_digit() || c == '.'));
    let (number, mult) = match at {
        None => (t.clone(), 1.0),
        Some(i) => {
            let ch = t[i..].chars().next()?;
            let mult = match ch {
                'p' => 1e-12,
                'n' => 1e-9,
                'u' | 'µ' | 'μ' => 1e-6,
                'm' => 1e-3,
                'k' | 'K' => 1e3,
                'M' => 1e6,
                'G' => 1e9,
                'R' | 'r' => 1.0,
                _ => return None,
            };
            let (left, right) = (&t[..i], &t[i + ch.len_utf8()..]);
            if left.is_empty() || !right.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            (if right.is_empty() { left.to_string() } else { format!("{left}.{right}") }, mult)
        }
    };
    number.parse::<f64>().ok().map(|x| x * mult)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_read() {
        let close = |a: Option<f64>, b: f64| a.is_some_and(|a| (a - b).abs() <= b.abs() * 1e-9);
        assert!(close(parse_value("100nF"), 100e-9));
        assert!(close(parse_value("0.1uF"), 0.1e-6));
        assert!(close(parse_value("10u"), 10e-6));
        assert!(close(parse_value("4.7k"), 4700.0));
        assert!(close(parse_value("4k7"), 4700.0));
        assert!(close(parse_value("10K"), 10_000.0));
        assert!(close(parse_value("220R"), 220.0));
        assert!(close(parse_value("1M"), 1e6));
        assert!(close(parse_value("22"), 22.0));
        assert!(close(parse_value("1uF"), 1e-6));
        assert_eq!(parse_value("DNP"), None);
        assert_eq!(parse_value(""), None);
    }

    #[test]
    fn kinds_follow_the_reference() {
        let c = |r: &str, v: &str| Component { reference: r.into(), value: Some(v.into()), ..Default::default() };
        assert_eq!(Snapshot::kind_of(&c("R12", "10k")), "resistor");
        assert_eq!(Snapshot::kind_of(&c("C3", "100n")), "capacitor");
        assert_eq!(Snapshot::kind_of(&c("SW1", "BOOT")), "switch");
        assert_eq!(Snapshot::kind_of(&c("U1", "FM-100")), "ic");
        assert_eq!(Snapshot::kind_of(&c("D1", "LED")), "led");
        assert_eq!(Snapshot::kind_of(&c("D2", "1N4148")), "other");
        assert_eq!(Snapshot::kind_of(&c("J1", "USB")), "connector");
    }
}
