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
    // (unique per call: the tests of one binary run in parallel)
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let d = std::env::temp_dir().join(format!("fastroute-it-{}-{name}-{n}", std::process::id()));
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

/// The JSON Schema interpreter of the program itself (src/jsonschema.rs): the tests check
/// real output with the same code that enforces the published schemas at run time.
mod schema {
    include!("../../src/jsonschema.rs");
}

pub fn validate(schema_doc: &Value, value: &Value, _path: &str, errors: &mut Vec<String>) {
    errors.extend(schema::validate(schema_doc, value));
}

/// Fails the test with every violation if `value` does not match the schema file.
pub fn assert_valid(schema_file: &str, value: &Value) {
    let schema: Value = serde_json::from_slice(&std::fs::read(manifest_dir().join("../../docs/schema").join(schema_file)).unwrap()).unwrap();
    let mut errors = Vec::new();
    validate(&schema, value, "$", &mut errors);
    assert!(errors.is_empty(), "{schema_file}:\n{}", errors.join("\n"));
}

/// The session of the demo board routed by the legacy command line, and the `--report` of that
/// run (done once per test binary).
pub fn routed_demo() -> (PathBuf, PathBuf) {
    static ROUTED: std::sync::OnceLock<(PathBuf, PathBuf)> = std::sync::OnceLock::new();
    ROUTED
        .get_or_init(|| {
            let d = tmp("routed-demo");
            let (ses, report) = (d.join("demo.ses"), d.join("demo-report.json"));
            let o = std::process::Command::new(env!("CARGO_BIN_EXE_fastroute"))
                .args(["-de", &s(&demo_dsn()), "-do", &s(&ses), &format!("--report={}", s(&report)), "--max-time=60", "--router.optimizer.enabled=false"])
                .output()
                .expect("route the demo board");
            assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
            (ses, report)
        })
        .clone()
}

pub fn routed_demo_ses() -> PathBuf {
    routed_demo().0
}

/// RGBA pixels of a PNG file: (width, height, bytes).
pub fn png_pixels(path: &Path) -> (u32, u32, Vec<u8>) {
    let mut dec = png::Decoder::new(std::fs::File::open(path).unwrap());
    dec.set_transformations(png::Transformations::EXPAND);
    let mut r = dec.read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).unwrap();
    assert_eq!(info.color_type, png::ColorType::Rgba, "{}", path.display());
    buf.truncate(info.buffer_size());
    (info.width, info.height, buf)
}

pub fn simple_dsn() -> PathBuf {
    manifest_dir().join("testdata/route_simple.dsn")
}

/// A scratch directory holding a copy of `dsn`, so that default outputs land next to it.
pub fn work_copy(name: &str, dsn: &Path) -> (PathBuf, PathBuf) {
    let d = tmp(name);
    let copy = d.join(dsn.file_name().unwrap());
    std::fs::copy(dsn, &copy).unwrap();
    (d, copy)
}
