//! Shared fixtures: the example pack and a legal design for it (the fictional Acme FM-100).
#![allow(dead_code)]

use fr_constraint::compile;
use fr_constraint::hcp;
use fr_constraint::ir::Ir;
use fr_constraint::snapshot::Snapshot;
use serde_json::{json, Value};
use std::path::PathBuf;

pub fn skill() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../skills/hardware-constraint-design")
}

pub fn example() -> Value {
    hcp::parse(&std::fs::read_to_string(skill().join("assets/example-device.yaml")).unwrap()).unwrap()
}

pub fn ir() -> Ir {
    compile::compile(&example()).expect("the example compiles")
}

fn pin(r: &str, p: &str, net: Option<&str>, x: f64, y: f64) -> Value {
    match net {
        Some(n) => json!({"ref": r, "pin": p, "net": n, "x": x, "y": y}),
        None => json!({"ref": r, "pin": p, "x": x, "y": y}),
    }
}

/// A design that satisfies every rule of the example that the validator can check.
pub fn legal_json() -> Value {
    json!({
        "snapshot_version": "1.0",
        "project": {"name": "fm100-demo"},
        "components": [
            {"ref": "U1", "device": "DEV-FM100", "value": "FM-100", "x": 30.0, "y": 28.0, "rotation_deg": 0.0},
            {"ref": "C1", "value": "10uF", "x": 24.0, "y": 22.0},
            {"ref": "C2", "value": "100nF", "x": 25.0, "y": 22.0},
            {"ref": "R1", "value": "10k", "x": 26.0, "y": 30.0},
            {"ref": "C3", "value": "1nF", "x": 27.0, "y": 30.0},
            {"ref": "SW1", "value": "BOOT", "x": 22.0, "y": 34.0},
            {"ref": "J1", "value": "USB", "x": 50.0, "y": 36.0}
        ],
        "pins": [
            pin("U1", "1", Some("GND"), 28.0, 22.0),
            pin("U1", "2", Some("3V3"), 28.0, 24.0),
            pin("U1", "3", Some("EN"), 28.0, 26.0),
            pin("U1", "4", Some("BOOT"), 28.0, 28.0),
            pin("U1", "5", Some("LED_DRIVE"), 28.0, 30.0),
            pin("U1", "6", Some("TXD"), 28.0, 32.0),
            pin("U1", "7", Some("RXD"), 28.0, 34.0),
            pin("U1", "8", Some("USB_DP"), 32.0, 34.0),
            pin("U1", "9", Some("USB_DM"), 32.0, 32.0),
            pin("U1", "10", None, 32.0, 30.0),
            pin("U1", "11", None, 32.0, 28.0),
            pin("U1", "12", Some("GND"), 32.0, 26.0),
            pin("C1", "1", Some("3V3"), 24.0, 22.0), pin("C1", "2", Some("GND"), 24.0, 23.0),
            pin("C2", "1", Some("3V3"), 25.0, 22.0), pin("C2", "2", Some("GND"), 25.0, 23.0),
            pin("R1", "1", Some("3V3"), 26.0, 30.0), pin("R1", "2", Some("EN"), 26.0, 31.0),
            pin("C3", "1", Some("EN"), 27.0, 30.0), pin("C3", "2", Some("GND"), 27.0, 31.0),
            pin("SW1", "1", Some("BOOT"), 22.0, 34.0), pin("SW1", "2", Some("GND"), 22.0, 35.0),
            pin("J1", "1", Some("USB_DP"), 50.0, 36.0), pin("J1", "2", Some("USB_DM"), 50.0, 37.0)
        ],
        "tracks": [
            {"net": "USB_DP", "layer": "F.Cu", "width_mm": 0.2, "x1": 32.0, "y1": 34.0, "x2": 50.0, "y2": 36.0},
            {"net": "USB_DM", "layer": "F.Cu", "width_mm": 0.2, "x1": 32.0, "y1": 32.0, "x2": 50.0, "y2": 34.0},
            {"net": "3V3", "layer": "F.Cu", "width_mm": 0.4, "x1": 28.0, "y1": 24.0, "x2": 24.0, "y2": 22.0}
        ],
        "outline": [[0.0, 0.0], [60.0, 0.0], [60.0, 40.0], [0.0, 40.0]],
        "mount_holes": [{"x": 3.0, "y": 3.0, "radius_mm": 1.6}],
        "assignments": [
            {"ref": "U1", "pin": "6", "function": "UART0_TX"},
            {"ref": "U1", "pin": "7", "function": "UART0_RX"},
            {"ref": "U1", "pin": "8", "function": "USB_DP"},
            {"ref": "U1", "pin": "9", "function": "USB_DM"},
            {"ref": "U1", "pin": "5", "function": "GPIO1"}
        ]
    })
}

pub fn legal() -> Snapshot {
    serde_json::from_value(legal_json()).unwrap()
}

/// Edits the legal design: `f` gets the JSON.
pub fn changed(f: impl FnOnce(&mut Value)) -> Snapshot {
    let mut v = legal_json();
    f(&mut v);
    serde_json::from_value(v).unwrap()
}
