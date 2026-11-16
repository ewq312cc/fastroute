//! Helpers shared by the CLI integration tests.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

pub fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub fn demo_dsn() -> PathBuf {
    manifest_dir().join("testdata/render_demo.dsn")
}

/// A synthetic design in millimetres (`resolution mm 1000`), 50 x 40 mm.
pub fn mm_dsn() -> PathBuf {
    manifest_dir().join("../fr-io/testdata/dsn/synth_planes.dsn")
}

/// A fresh empty directory for one test.
pub fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("fastroute-it-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Runs fastroute with no browser, no display and an empty PATH: whatever it does has to work
/// on a headless machine.
pub fn run<S: AsRef<std::ffi::OsStr>>(args: &[S]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fastroute"))
        .args(args)
        .env("PATH", "")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .output()
        .expect("run fastroute")
}

pub fn run_in<S: AsRef<std::ffi::OsStr>>(dir: &Path, args: &[S]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_fastroute")).args(args).current_dir(dir).env("PATH", "").output().expect("run fastroute")
}

pub fn code(o: &Output) -> i32 {
    o.status.code().expect("exited normally")
}

pub fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

pub fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

pub fn json(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// (width, height) of a PNG, checking its signature.
pub fn png_size(path: &Path) -> (u32, u32) {
    let b = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&b[..8], b"\x89PNG\r\n\x1a\n", "{} is not a PNG", path.display());
    (u32::from_be_bytes(b[16..20].try_into().unwrap()), u32::from_be_bytes(b[20..24].try_into().unwrap()))
}

pub fn file_hash(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

pub fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// Validates `value` against the subset of JSON Schema the published schemas use:
/// local `$ref`, `type` (also a list), `enum`, `const`, `required`, `properties`,
/// `additionalProperties` (false or a schema), `items`, `minItems`, `minimum`.
pub fn validate(schema: &Value, value: &Value, path: &str, errors: &mut Vec<String>) {
    validate_in(schema, schema, value, path, errors)
}

fn resolve<'a>(root: &'a Value, reference: &str) -> &'a Value {
    let pointer = reference.strip_prefix('#').unwrap_or_else(|| panic!("only local $ref is supported: {reference}"));
    root.pointer(pointer).unwrap_or_else(|| panic!("unresolved $ref {reference}"))
}

fn validate_in(root: &Value, schema: &Value, value: &Value, path: &str, errors: &mut Vec<String>) {
    if let Some(r) = schema.get("$ref").and_then(Value::as_str) {
        return validate_in(root, resolve(root, r), value, path, errors);
    }
    if let Some(types) = schema.get("type") {
        let list: Vec<&str> = match types {
            Value::String(t) => vec![t.as_str()],
            Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
            _ => vec![],
        };
        let ok = list.iter().any(|t| match *t {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "number" => value.is_number(),
            "integer" => value.is_i64() || value.is_u64() || value.as_f64().is_some_and(|f| f.fract() == 0.0 && value.is_number() && !value.is_f64()),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            _ => false,
        });
        if !ok {
            errors.push(format!("{path}: expected {list:?}, got {value}"));
            return;
        }
    }
    if let Some(c) = schema.get("const") {
        if c != value {
            errors.push(format!("{path}: expected the constant {c}, got {value}"));
        }
    }
    if let Some(e) = schema.get("enum").and_then(Value::as_array) {
        if !e.contains(value) {
            errors.push(format!("{path}: {value} is not one of {e:?}"));
        }
    }
    if let (Some(min), Some(v)) = (schema.get("minimum").and_then(Value::as_f64), value.as_f64()) {
        if v < min {
            errors.push(format!("{path}: {v} is below the minimum {min}"));
        }
    }
    if let (Some(obj), Some(props)) = (value.as_object(), schema.get("properties").and_then(Value::as_object)) {
        for r in schema.get("required").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            if !obj.contains_key(r) {
                errors.push(format!("{path}: missing required property '{r}'"));
            }
        }
        for (k, v) in obj {
            match props.get(k) {
                Some(s) => validate_in(root, s, v, &format!("{path}.{k}"), errors),
                None => match schema.get("additionalProperties") {
                    Some(Value::Bool(false)) => errors.push(format!("{path}: unexpected property '{k}'")),
                    Some(s @ Value::Object(_)) => validate_in(root, s, v, &format!("{path}.{k}"), errors),
                    _ => {}
                },
            }
        }
    } else if let (Some(obj), Some(ap)) = (value.as_object(), schema.get("additionalProperties")) {
        if ap.is_object() {
            for (k, v) in obj {
                validate_in(root, ap, v, &format!("{path}.{k}"), errors);
            }
        }
    }
    if let (Some(arr), Some(items)) = (value.as_array(), schema.get("items")) {
        if let Some(n) = schema.get("minItems").and_then(Value::as_u64) {
            if (arr.len() as u64) < n {
                errors.push(format!("{path}: needs at least {n} items"));
            }
        }
        for (i, v) in arr.iter().enumerate() {
            validate_in(root, items, v, &format!("{path}[{i}]"), errors);
        }
    }
}

/// Fails the test with every violation if `value` does not match the schema file.
pub fn assert_valid(schema_file: &str, value: &Value) {
    let schema: Value = serde_json::from_slice(&std::fs::read(manifest_dir().join("../../docs/schema").join(schema_file)).unwrap()).unwrap();
    let mut errors = Vec::new();
    validate(&schema, value, "$", &mut errors);
    assert!(errors.is_empty(), "{schema_file}:\n{}", errors.join("\n"));
}

/// The session of the demo board routed by the legacy command line (done once per test binary).
pub fn routed_demo_ses() -> PathBuf {
    static SES: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    SES.get_or_init(|| {
        let d = tmp("routed-demo");
        let ses = d.join("demo.ses");
        let o = std::process::Command::new(env!("CARGO_BIN_EXE_fastroute"))
            .args(["-de", &s(&demo_dsn()), "-do", &s(&ses), "--max-time=60", "--router.optimizer.enabled=false"])
            .output()
            .expect("route the demo board");
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        ses
    })
    .clone()
}
