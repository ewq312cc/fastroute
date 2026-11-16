//! `fastroute route` and the compatibility of the original flag interface.

mod common;
use common::*;
use serde_json::Value;
use std::path::Path;

const FAST: &str = "--router.optimizer.enabled=false";

fn summary(o: &std::process::Output) -> Value {
    serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("stdout is not one JSON summary ({e}): {}", stdout(o)))
}

#[test]
fn a_complete_route_writes_the_session_next_to_the_input_and_exits_0() {
    let (d, dsn) = work_copy("complete", &simple_dsn());
    let before = file_hash(&dsn);
    let o = run_in(&d, &["route", "route_simple.dsn", FAST, "--quiet"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    // the default output is <name>.routed.ses; the input is untouched
    let ses = d.join("route_simple.routed.ses");
    let text = std::fs::read_to_string(&ses).unwrap();
    assert!(text.starts_with("(session"), "{text}");
    assert_eq!(file_hash(&dsn), before);
    assert_eq!(std::fs::read_dir(&d).unwrap().count(), 2, "only the input and the result");
    // and what was written is a result that inspect reads as fully routed
    let out = d.join("inspect.json");
    let o = run(&["inspect", "-i", &s(&dsn), "-s", &s(&ses), "-o", &s(&out), "-q"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(json(&out)["routing"]["status"], "complete");
}

#[test]
fn json_summary_for_complete_partial_and_failed_runs() {
    // complete
    let (d, _) = work_copy("json-complete", &simple_dsn());
    let o = run_in(&d, &["route", "route_simple.dsn", FAST, "-q", "--json"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let v = summary(&o);
    assert_eq!((v["command"].as_str(), v["status"].as_str(), v["exit_code"].as_i64()), (Some("route"), Some("complete"), Some(0)));
    assert_eq!((v["unrouted"].as_u64(), v["violations"].as_u64()), (Some(0), Some(0)));
    assert_eq!((v["input"].as_str(), v["output"].as_str(), v["error"].clone()), (Some("route_simple.dsn"), Some("route_simple.routed.ses"), Value::Null));
    assert_eq!(v["fastroute_version"], env!("CARGO_PKG_VERSION"));
    assert!(v["seconds"].as_f64().unwrap() >= 0.0);
    // partial: the demo board has connections that stay unrouted (a copper plane without a way in
    // and a pre-routed clearance violation that the router must not touch)
    let (d, _) = work_copy("json-partial", &demo_dsn());
    let o = run_in(&d, &["route", "render_demo.dsn", FAST, "-q", "--json", "--max-time=60"]);
    assert_eq!(code(&o), 5, "{}", stderr(&o));
    let v = summary(&o);
    assert_eq!((v["status"].as_str(), v["exit_code"].as_i64()), (Some("partial"), Some(5)));
    assert!(v["unrouted"].as_u64().unwrap() > 0 && v["violations"].as_u64().is_some(), "{v}");
    assert!(d.join("render_demo.routed.ses").exists(), "a partial result is written");
    assert!(stderr(&o).contains("connections remain unrouted"), "{}", stderr(&o));
    // --allow-partial: the same result, exit code 0
    let o = run_in(&d, &["route", "render_demo.dsn", FAST, "-q", "--json", "--max-time=60", "--allow-partial", "--force"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(summary(&o)["status"], "partial");
    // failed: an unreadable design (the summary says so, the exit code is the input code)
    let bad = d.join("bad.dsn");
    std::fs::write(&bad, "(pcb x (structure").unwrap();
    let o = run_in(&d, &["route", "bad.dsn", "--json"]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    let v = summary(&o);
    assert_eq!((v["status"].as_str(), v["exit_code"].as_i64(), v["output"].clone(), v["unrouted"].clone()), (Some("failed"), Some(3), Value::Null, Value::Null));
    assert!(v["error"].as_str().unwrap().contains("bad.dsn"), "{v}");
    assert!(!d.join("bad.routed.ses").exists());
}

#[test]
fn an_existing_output_is_refused_before_routing_and_the_input_is_never_the_output() {
    let (d, dsn) = work_copy("overwrite", &simple_dsn());
    let existing = d.join("route_simple.routed.ses");
    std::fs::write(&existing, "precious").unwrap();
    let started = std::time::Instant::now();
    let o = run_in(&d, &["route", "route_simple.dsn", FAST]);
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert!(stderr(&o).contains("already exists") && stderr(&o).contains("--force"), "{}", stderr(&o));
    assert_eq!(std::fs::read(&existing).unwrap(), b"precious");
    assert!(!stderr(&o).contains("INFO"), "nothing was routed: {}", stderr(&o));
    assert!(started.elapsed().as_secs() < 5);
    // --force replaces it
    let o = run_in(&d, &["route", "route_simple.dsn", FAST, "--force", "-q"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(std::fs::read_to_string(&existing).unwrap().starts_with("(session"));
    // an explicit -o that exists is protected too; the input design never is a valid output
    let mine = d.join("mine.ses");
    std::fs::write(&mine, "mine").unwrap();
    assert_eq!(code(&run_in(&d, &["route", "route_simple.dsn", "-o", "mine.ses", FAST])), 4);
    assert_eq!(std::fs::read(&mine).unwrap(), b"mine");
    let before = file_hash(&dsn);
    let o = run_in(&d, &["route", "route_simple.dsn", "-o", "route_simple.dsn", "--force"]);
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert!(stderr(&o).contains("never overwrites its input"), "{}", stderr(&o));
    assert_eq!(file_hash(&dsn), before);
    // the report file is protected like the output
    std::fs::write(d.join("r.json"), "keep").unwrap();
    let o = run_in(&d, &["route", "route_simple.dsn", "-o", "other.ses", "--report", "r.json", FAST]);
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert_eq!(std::fs::read(d.join("r.json")).unwrap(), b"keep");
    assert!(!d.join("other.ses").exists());
    // a missing directory is an output error before routing too
    assert_eq!(code(&run_in(&d, &["route", "route_simple.dsn", "-o", "nope/x.ses"])), 4);
}

#[test]
fn quiet_and_verbose_control_the_progress_lines_but_not_warnings() {
    let (d, _) = work_copy("quiet", &simple_dsn());
    let loud = run_in(&d, &["route", "route_simple.dsn", FAST, "-o", "a.ses"]);
    assert_eq!(code(&loud), 0);
    assert!(stderr(&loud).contains("INFO") && stderr(&loud).contains("routing finished"), "{}", stderr(&loud));
    assert!(stderr(&loud).contains("routed completely"), "{}", stderr(&loud));
    let quiet = run_in(&d, &["route", "route_simple.dsn", FAST, "-o", "b.ses", "--quiet"]);
    assert_eq!(code(&quiet), 0);
    assert!(quiet.stdout.is_empty() && quiet.stderr.is_empty(), "{} / {}", stdout(&quiet), stderr(&quiet));
    // warnings still show with --quiet (the demo board has a clearance violation in its design)
    let (d2, _) = work_copy("quiet-warn", &demo_dsn());
    let o = run_in(&d2, &["route", "render_demo.dsn", FAST, "--quiet", "--allow-partial", "--max-time=60"]);
    assert_eq!(code(&o), 0);
    assert!(stderr(&o).contains("WARN") && !stderr(&o).contains("INFO"), "{}", stderr(&o));
    assert_eq!(code(&run_in(&d, &["route", "route_simple.dsn", "-o", "c.ses", "-v", "-q"])), 2);
}

#[test]
fn usage_and_input_errors_have_distinct_exit_codes_and_clear_messages() {
    let (d, _) = work_copy("errors", &simple_dsn());
    std::fs::write(d.join("empty.dsn"), "").unwrap();
    std::fs::write(d.join("notes.txt"), "hello").unwrap();
    std::fs::write(d.join("board.kicad_pcb"), "").unwrap();
    // (args, exit code, text in stderr)
    let cases: Vec<(Vec<&str>, i32, &str)> = vec![
        (vec!["route"], 2, "no input"),
        (vec!["route", "a.dsn", "b.dsn"], 2, "more than one input"),
        (vec!["route", "route_simple.dsn", "--bogus"], 2, "unknown option '--bogus'"),
        (vec!["route", "route_simple.dsn", "--max-passes", "many"], 2, "not a whole number"),
        (vec!["route", "route_simple.dsn", "--max-time", "soon"], 2, "not a number"),
        (vec!["route", "route_simple.dsn", "--mode", "turbo"], 2, "not fast, exact or quick"),
        (vec!["route", "route_simple.dsn", "--diagnose"], 2, "--diagnose needs --report"),
        (vec!["route", "route_simple.dsn", "--live-port", "9000"], 2, "--live-port needs --live"),
        (vec!["route", "route_simple.dsn", "--python", "py"], 2, "only applies to a KiCad board"),
        (vec!["route", "route_simple.dsn", "--clear"], 2, "only applies to a KiCad board"),
        (vec!["route", "/no/such.dsn"], 3, "/no/such.dsn"),
        (vec!["route", "empty.dsn"], 3, "empty"),
        (vec!["route", "notes.txt"], 3, "neither a Specctra design"),
        (vec!["route", "board.kicad_pcb"], 3, "the file is empty"),
        (vec!["route", "route_simple.dsn", "--initial-session", "/no/such.ses"], 3, "/no/such.ses"),
    ];
    for (args, want, text) in cases {
        let o = run_in(&d, &args);
        assert_eq!(code(&o), want, "{args:?}: {}", stderr(&o));
        assert!(stderr(&o).contains(text), "{args:?}: expected '{text}' in\n{}", stderr(&o));
        assert!(stderr(&o).starts_with("fastroute route:"), "{args:?}: {}", stderr(&o));
    }
    assert!(!d.join("route_simple.routed.ses").exists(), "no error left a result behind");
    // the usage hint
    assert!(stderr(&run_in(&d, &["route", "--nope"])).contains("fastroute route --help"));
}

#[test]
fn help_everywhere_is_consistent() {
    for cmd in ["route", "render", "inspect", "ai-pack"] {
        let o = run(&[cmd, "--help"]);
        assert_eq!(code(&o), 0, "{cmd}");
        let t = stdout(&o);
        assert!(t.starts_with(&format!("fastroute {cmd} - ")), "{cmd}: {t}");
        for needle in ["usage: fastroute ", "options:", "--help", "exit codes: 0 ok, 1 failed, 2 usage, 3 input, 4 output, 5 partial"] {
            assert!(t.contains(needle), "{cmd}: {needle}");
        }
        assert_eq!(stdout(&run(&["help", cmd])), t, "`help {cmd}` is `{cmd} --help`");
    }
    // the top-level help lists the commands and still documents the original flags
    let o = run(&["--help"]);
    assert_eq!(code(&o), 0);
    for needle in ["route ", "render ", "inspect ", "ai-pack ", "-de FILE", "-do FILE", "--router.autorouter.max_passes", "--quiet"] {
        assert!(stdout(&o).contains(needle), "{needle}");
    }
    assert_eq!(stdout(&run(&["help"])), stdout(&o));
    assert_eq!(stdout(&run(&["--version"])), format!("fastroute {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn the_original_flags_still_work_and_give_the_same_result() {
    let (d, _) = work_copy("legacy", &simple_dsn());
    // original interface: -de / -do (also -mp and --router.* settings)
    let o = run_in(&d, &["-de", "route_simple.dsn", "-do", "legacy.ses", "-mp", "5", FAST]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(stderr(&o).contains("INFO"), "the original interface keeps its progress output");
    // the new command produces the same session for the same settings
    let o = run_in(&d, &["route", "route_simple.dsn", "-o", "new.ses", "--max-passes", "5", FAST, "-q"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(file_hash(&d.join("legacy.ses")), file_hash(&d.join("new.ses")));
    // the original interface is not made safer behind the scripts' back: -do replaces a file as before
    std::fs::write(d.join("legacy.ses"), "old").unwrap();
    assert_eq!(code(&run_in(&d, &["-de", "route_simple.dsn", "-do", "legacy.ses", FAST])), 0);
    assert!(std::fs::read_to_string(d.join("legacy.ses")).unwrap().starts_with("(session"));
    // and it still fails the way it did
    let o = run_in(&d, &["-de", "/no/such.dsn"]);
    assert_eq!(code(&o), 1);
    assert!(stderr(&o).contains("fastroute: /no/such.dsn"), "{}", stderr(&o));
    assert_eq!(code(&run_in(&d, &["-do", "x.ses"])), 1);
    // --quiet is available on the original interface as well
    let o = run_in(&d, &["-de", "route_simple.dsn", "-do", "q.ses", "--quiet", FAST]);
    assert_eq!(code(&o), 0);
    assert!(!stderr(&o).contains("INFO"), "{}", stderr(&o));
}

#[test]
fn the_engine_flow_needs_no_browser_or_gui() {
    let (d, _) = work_copy("headless", &simple_dsn());
    // empty PATH, no display: no xdg-open, no python, nothing to start
    let o = run_in(&d, &["route", "route_simple.dsn", FAST, "-q"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(!stderr(&o).contains("live viewer"), "{}", stderr(&o));
}

#[cfg(unix)]
mod kicad_adapter {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A stand-in for KiCad's Python. `-c` (the pcbnew check) succeeds unless `no_pcbnew`;
    /// otherwise it records its arguments in `<script>.argv`, prints `stdout` and exits with `exit`.
    fn fake_python(dir: &Path, name: &str, no_pcbnew: bool, stdout: &str, exit: i32) -> std::path::PathBuf {
        let script = dir.join(name);
        let check = if no_pcbnew { "echo \"ModuleNotFoundError: No module named 'pcbnew'\" >&2; exit 1" } else { "exit 0" };
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nif [ \"$1\" = \"-c\" ]; then\n\
                 case \"$2\" in *ImportSpecctraSES*) if [ -f \"$0.old-api\" ]; then echo 7.0.11; exit 3; fi; exit 0;; esac\n\
                 {check}; fi\n\
                 echo \"$@\" > \"$0.argv\"\n\
                 n=0; [ -f \"$0.calls\" ] && read n < \"$0.calls\"; echo $((n+1)) > \"$0.calls\"\n\
                 echo '{stdout}'\nexit {exit}\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    fn argv(py: &Path) -> String {
        std::fs::read_to_string(format!("{}.argv", py.display())).unwrap_or_default()
    }

    fn calls(py: &Path) -> u32 {
        std::fs::read_to_string(format!("{}.calls", py.display())).ok().and_then(|t| t.trim().parse().ok()).unwrap_or(0)
    }

    fn setup(name: &str) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
        let d = tmp(name);
        let board = d.join("board.kicad_pcb");
        std::fs::write(&board, "(kicad_pcb (version 20240108))\n").unwrap();
        let cli = d.join("route_cli.py");
        std::fs::write(&cli, "# stand-in\n").unwrap();
        (d, board, cli)
    }

    #[test]
    fn a_kicad_board_is_routed_through_the_adapter_with_the_safe_defaults() {
        let (d, board, cli) = setup("kicad-ok");
        let py = fake_python(&d, "python", false, r#"{"status":"complete","exit_code":0,"unrouted":0,"violations":1,"error":null}"#, 0);
        let o = run(&["route", &s(&board), "--python", &s(&py), "--route-cli", &s(&cli), "--json", "--max-time", "60", "--threads", "2", "--router.min_trace_width_um=100"]);
        assert_eq!(code(&o), 0, "{}", stderr(&o));
        let v = summary(&o);
        assert_eq!((v["status"].as_str(), v["unrouted"].as_u64(), v["violations"].as_u64()), (Some("complete"), Some(0), Some(1)));
        // the default output is next to the input and is not the input
        assert_eq!(v["output"], s(&d.join("board.routed.kicad_pcb")));
        let a = argv(&py);
        let exe = env!("CARGO_BIN_EXE_fastroute");
        assert!(a.starts_with(&format!("{} {} -o {} --fastroute {exe} --json", s(&cli), s(&board), s(&d.join("board.routed.kicad_pcb")))), "{a}");
        // options reach the adapter; strictness about unrouted connections is on by default
        for needle in ["--max-time 60", "--threads 2", "--fail-on-partial", "-- --router.min_trace_width_um=100"] {
            assert!(a.contains(needle), "{needle} in {a}");
        }
        assert!(!a.contains("--force"), "{a}");
    }

    #[test]
    fn partial_failed_and_rejected_results_map_to_the_exit_codes() {
        let (d, board, cli) = setup("kicad-codes");
        let run_with = |py: &Path, extra: &[&str]| {
            let mut a: Vec<String> = vec!["route".into(), s(&board), "--python".into(), s(py), "--route-cli".into(), s(&cli), "--json".into(), "--force".into()];
            a.extend(extra.iter().map(|x| x.to_string()));
            run(&a)
        };
        // the adapter wrote the board but 3 connections remain: 5, or 0 with --allow-partial
        let partial = fake_python(&d, "py-partial", false, r#"{"status":"partial","exit_code":5,"unrouted":3,"violations":0,"error":null}"#, 5);
        let o = run_with(&partial, &[]);
        assert_eq!(code(&o), 5, "{}", stderr(&o));
        assert_eq!((summary(&o)["status"].as_str(), summary(&o)["unrouted"].as_u64()), (Some("partial"), Some(3)));
        assert!(argv(&partial).contains("--fail-on-partial") && argv(&partial).contains("--force"));
        let allow = fake_python(&d, "py-allow", false, r#"{"status":"partial","exit_code":0,"unrouted":3,"violations":0,"error":null}"#, 0);
        let o = run_with(&allow, &["--allow-partial"]);
        assert_eq!(code(&o), 0, "{}", stderr(&o));
        assert_eq!(summary(&o)["status"], "partial");
        assert!(!argv(&allow).contains("--fail-on-partial"), "{}", argv(&allow));
        // the adapter failed
        let failed = fake_python(&d, "py-failed", false, r#"{"status":"failed","exit_code":1,"output":null,"error":"fastroute crashed"}"#, 1);
        let o = run_with(&failed, &[]);
        assert_eq!(code(&o), 1);
        assert!(stderr(&o).contains("fastroute crashed"), "{}", stderr(&o));
        assert_eq!(summary(&o)["status"], "failed");
        // the adapter rejected the board (its exit code 2) is an input error here (3)
        let rejected = fake_python(&d, "py-rejected", false, r#"{"status":"failed","exit_code":2,"error":"board.kicad_pcb: not a board with an outline on Edge.Cuts"}"#, 2);
        let o = run_with(&rejected, &[]);
        assert_eq!(code(&o), 3, "{}", stderr(&o));
        assert!(stderr(&o).contains("Edge.Cuts"));
        // the adapter refused the output (its 4)
        let refused = fake_python(&d, "py-refused", false, r#"{"status":"failed","exit_code":4,"error":"x: already exists"}"#, 4);
        assert_eq!(code(&run_with(&refused, &[])), 4);
        // an adapter that prints no summary (an old route_cli.py) is reported as such, not trusted
        let old = fake_python(&d, "py-old", false, "routed in 3 s", 0);
        let o = run_with(&old, &[]);
        assert_eq!(code(&o), 1, "{}", stderr(&o));
        assert!(stderr(&o).contains("printed no summary"), "{}", stderr(&o));
    }

    #[test]
    fn a_missing_dependency_is_named_and_nothing_is_attempted() {
        let (d, board, cli) = setup("kicad-deps");
        // a Python without pcbnew
        let nopcb = fake_python(&d, "py-nopcbnew", true, "{}", 0);
        let o = run(&["route", &s(&board), "--python", &s(&nopcb), "--route-cli", &s(&cli)]);
        assert_eq!(code(&o), 3, "{}", stderr(&o));
        let e = stderr(&o);
        for needle in ["no Python with KiCad's pcbnew module", "No module named 'pcbnew'", "--python", "FASTROUTE_KICAD_PYTHON", "export one from KiCad"] {
            assert!(e.contains(needle), "{needle}\n{e}");
        }
        assert_eq!(calls(&nopcb), 0, "the adapter must not run without pcbnew");
        // no interpreter at all (empty PATH, nothing configured)
        let o = std::process::Command::new(env!("CARGO_BIN_EXE_fastroute"))
            .args(["route", &s(&board), "--route-cli", &s(&cli)])
            .env("PATH", "")
            .env_remove("FASTROUTE_KICAD_PYTHON")
            .output()
            .unwrap();
        assert_eq!(code(&o), 3, "{}", stderr(&o));
        assert!(stderr(&o).contains("python3 on PATH") && stderr(&o).contains("cannot run it"), "{}", stderr(&o));
        // the environment variable is used and named
        let o = std::process::Command::new(env!("CARGO_BIN_EXE_fastroute"))
            .args(["route", &s(&board), "--route-cli", &s(&cli)])
            .env("PATH", "")
            .env("FASTROUTE_KICAD_PYTHON", s(&nopcb))
            .output()
            .unwrap();
        assert_eq!(code(&o), 3);
        assert!(stderr(&o).contains("$FASTROUTE_KICAD_PYTHON"), "{}", stderr(&o));
        // the plugin script is missing
        let py = fake_python(&d, "py-ok", false, "{}", 0);
        let o = run(&["route", &s(&board), "--python", &s(&py), "--route-cli", "/no/such/route_cli.py"]);
        assert_eq!(code(&o), 3);
        assert!(stderr(&o).contains("/no/such/route_cli.py"), "{}", stderr(&o));
        assert_eq!(calls(&py), 0);
    }

    #[test]
    fn a_kicad_that_cannot_import_a_session_headless_is_refused_before_any_routing() {
        // Seen with a real KiCad 7.0.11: pcbnew.ImportSpecctraSES takes only a file name, so the
        // adapter would route (for minutes or hours) and then fail at the import. Say so first.
        let (d, board, cli) = setup("kicad-oldapi");
        let py = fake_python(&d, "py-old", false, "{}", 0);
        std::fs::write(format!("{}.old-api", py.display()), "").unwrap();
        let o = run(&["route", &s(&board), "--python", &s(&py), "--route-cli", &s(&cli), "--json"]);
        assert_eq!(code(&o), 3, "{}", stderr(&o));
        let e = stderr(&o);
        for needle in ["KiCad 7.0.11", "cannot import a Specctra session without the PCB editor", "KiCad 8 or newer", "export one from KiCad"] {
            assert!(e.contains(needle), "{needle}\n{e}");
        }
        assert_eq!(calls(&py), 0, "the adapter must not run");
        assert!(!d.join("board.routed.kicad_pcb").exists());
        assert_eq!(summary(&o)["status"], "failed");
        // a second Python that can do it is used instead
        let good = fake_python(&d, "py-good", false, r#"{"status":"complete","exit_code":0,"unrouted":0,"violations":0}"#, 0);
        let o = std::process::Command::new(env!("CARGO_BIN_EXE_fastroute"))
            .args(["route", &s(&board), "--route-cli", &s(&cli), "--json"])
            .env("PATH", s(&d))
            .env("FASTROUTE_KICAD_PYTHON", s(&good))
            .output()
            .unwrap();
        assert_eq!(code(&o), 0, "{}", stderr(&o));
    }

    #[test]
    fn the_output_checks_happen_in_this_program_before_the_adapter_starts() {
        let (d, board, cli) = setup("kicad-out");
        let py = fake_python(&d, "python", false, r#"{"status":"complete","exit_code":0,"unrouted":0,"violations":0}"#, 0);
        let existing = d.join("board.routed.kicad_pcb");
        std::fs::write(&existing, "precious").unwrap();
        let o = run(&["route", &s(&board), "--python", &s(&py), "--route-cli", &s(&cli)]);
        assert_eq!(code(&o), 4, "{}", stderr(&o));
        assert!(stderr(&o).contains("already exists") && stderr(&o).contains("--force"));
        assert_eq!(std::fs::read(&existing).unwrap(), b"precious");
        assert_eq!(calls(&py), 0, "the adapter must not start when the output is refused");
        // the input board is not an output, with or without --force
        let o = run(&["route", &s(&board), "-o", &s(&board), "--force", "--python", &s(&py), "--route-cli", &s(&cli)]);
        assert_eq!(code(&o), 4, "{}", stderr(&o));
        assert_eq!(calls(&py), 0);
        assert_eq!(std::fs::read_to_string(&board).unwrap(), "(kicad_pcb (version 20240108))\n");
        // options of the Specctra flow are refused for a board
        let o = run(&["route", &s(&board), "--report", "r.json", "--python", &s(&py), "--route-cli", &s(&cli)]);
        assert_eq!(code(&o), 2, "{}", stderr(&o));
        assert!(stderr(&o).contains("only applies to a Specctra design"), "{}", stderr(&o));
        let o = run(&["route", &s(&board), "--clear", "--keep-existing", "--python", &s(&py), "--route-cli", &s(&cli)]);
        assert_eq!(code(&o), 2);
    }
}
