//! Runs the Python tests of the KiCad adapter (`integrations/kicad/tests/test_route_cli.py`):
//! the output policy of `route_cli.py` (the input board is never overwritten by default, an
//! existing output needs --force, --json, exit codes). They use stand-ins for `pcbnew` and the
//! router, so no KiCad is needed - only a Python 3, and the test is skipped without one.

use std::path::Path;
use std::process::Command;

#[test]
fn route_cli_py_output_policy() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let python = ["python3", "python"].into_iter().find(|p| Command::new(p).arg("--version").output().is_ok_and(|o| o.status.success()));
    let Some(python) = python else {
        eprintln!("skipped: no Python 3 found");
        return;
    };
    let out = Command::new(python)
        .args(["-I", "-m", "unittest", "discover", "-s"])
        .arg(root.join("integrations/kicad/tests"))
        .args(["-p", "test_*.py"])
        .current_dir(&root)
        .output()
        .expect("run the Python tests");
    assert!(out.status.success(), "{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
}
