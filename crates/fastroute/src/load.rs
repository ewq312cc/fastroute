//! Loading a board for the read-only commands (`render`, `inspect`, `ai-pack`): a Specctra
//! design (`.dsn`) plus, optionally, the wiring of a session (`.ses`).
//!
//! The preparation mirrors what `fastroute route` does before it starts routing (class-pair
//! clearances, the contact model, stitching vias), so connectivity and clearance numbers agree
//! with `--report`. Two routing-only changes are left out or hidden because they are not the
//! user's data: the keepouts that keep wiring inside the outline are not inserted, and the
//! short bridge traces that join trace ends to pad centres are remembered in
//! [`Loaded::synthetic`] so that geometry and statistics skip them.

use std::collections::HashSet;
use std::path::Path;

use fr_engine::board::RoutingBoard;
use fr_engine::ids::ItemId;
use fr_settings::{available_processors, headless_merger, CliSettings, DsnFileSettings, EnvironmentSettings, RouterSettings};
use sha2::{Digest, Sha256};

use crate::cmd::CmdError;

/// Path, size and SHA-256 of an input file.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FileInfo {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

impl FileInfo {
    pub fn of(path: &Path, data: &[u8]) -> FileInfo {
        FileInfo { path: path.display().to_string(), bytes: data.len() as u64, sha256: sha256_hex(data) }
    }
}

pub fn sha256_hex(data: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(data);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

struct SessionSummary {
    wires: i32,
    vias: i32,
    errors: i32,
}

pub struct Loaded {
    pub board: RoutingBoard,
    pub settings: RouterSettings,
    /// The design name (file stem of the DSN).
    pub name: String,
    /// Trace items that only exist for the connectivity model (see the module docs).
    pub synthetic: HashSet<ItemId>,
    /// Parser and loader warnings, as text.
    pub warnings: Vec<String>,
    pub input: FileInfo,
    pub session: Option<FileInfo>,
}

/// Reads a DSN file (and the session on top of it, if given).
pub fn load(dsn_path: &Path, session_path: Option<&Path>) -> Result<Loaded, CmdError> {
    let data = std::fs::read(dsn_path).map_err(|e| CmdError::input(format!("{}: {e}", dsn_path.display())))?;
    check_is_dsn(dsn_path, &data)?;
    let dsn = fr_dsn::Dsn::parse(&data).map_err(|e| CmdError::input(format!("{}: not a readable Specctra design: {e}", dsn_path.display())))?;
    let mut warnings: Vec<String> = dsn.warnings.iter().map(|w| w.to_string()).collect();

    let cli = CliSettings::parse::<&str>(&[]);
    let env = EnvironmentSettings::from_process_env();
    let procs = available_processors();
    let dsn_settings = DsnFileSettings::from_dsn(&dsn);
    let mut settings = headless_merger(&cli, &env, Some(&dsn_settings), None, procs).merge(procs);
    let (session, session_data) = match session_path {
        Some(p) => {
            let bytes = std::fs::read(p).map_err(|e| CmdError::input(format!("{}: {e}", p.display())))?;
            check_is_ses(p, &bytes)?;
            (Some(FileInfo::of(p, &bytes)), Some(bytes))
        }
        None => (None, None),
    };
    // A session is the complete result of a route run: the SES writer emits every trace and via
    // except the system-fixed ones, including the wiring the design already had (written back as
    // `(type protect)`). So the session replaces the design's routable wiring instead of adding
    // to it; importing on top would double every trace and via.
    let (mut board, replaced) = if session_data.is_some() {
        board_without_routes(&data, &mut settings).map_err(|e| CmdError::input(format!("{}: {e}", dsn_path.display())))?
    } else {
        let b = fr_io::post_load::load_from_specctra_dsn(&data, &mut settings).map_err(|e| CmdError::input(format!("{}: {e}", dsn_path.display())))?;
        (b, 0)
    };
    fr_io::network::extend_class_pair_clearances(&mut board, &dsn);
    let wiring_ids = |b: &RoutingBoard| -> HashSet<ItemId> {
        b.get_items().into_iter().filter(|&k| b.item(k).is_trace() || b.item(k).is_via()).map(|k| b.item(k).id()).collect()
    };
    let before = wiring_ids(&board);
    let summary = fr_io::post_load::prepare_for_routing(&mut board, &mut settings, session_data.as_deref());
    let session_summary = summary.map(|s| SessionSummary { wires: s.wires_imported, vias: s.vias_imported, errors: s.errors_encountered });
    if let (Some(s), Some(p)) = (&session_summary, session_path) {
        if s.wires == 0 && s.vias == 0 && s.errors > 0 {
            return Err(CmdError::input(format!("{}: not a readable session file ({} errors, nothing imported)", p.display(), s.errors)));
        }
        if s.errors > 0 {
            warnings.push(format!("{}: {} errors while reading the session; {} wires and {} vias imported", p.display(), s.errors, s.wires, s.vias));
        }
        if replaced > 0 {
            warnings.push(format!("the session replaces the {replaced} traces and vias of the design (a session holds the complete wiring)"));
        }
        // the session reader fixes what it imports; as in `route`, it is ordinary wiring here
        let imported: Vec<_> = wiring_ids(&board).difference(&before).copied().collect();
        fr_engine::diffpair::release_pairs(&mut board, &imported);
    }
    fr_engine::pipeline::deferred_post_load_processing(&mut board);
    board.fallback_vias_own_class = true;
    board.set_overlap_contacts(true);
    board.mark_stitching_vias();
    let traces_before: HashSet<ItemId> = board.get_traces().into_iter().map(|k| board.item(k).id()).collect();
    board.bridge_trace_ends_to_drill_centers();
    let synthetic: HashSet<ItemId> = board.get_traces().into_iter().map(|k| board.item(k).id()).filter(|id| !traces_before.contains(id)).collect();

    let name = dsn_path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(Loaded { board, settings, name, synthetic, warnings, input: FileInfo::of(dsn_path, &data), session })
}

/// Loads the design like `load_from_specctra_dsn`, without its pre-routed traces and vias
/// (except `(type fix)` ones); returns the board and how many were left out. The wiring is cut
/// out of the s-expression before the design is interpreted, so the scope order the loader
/// keeps (which refers to wires and vias by position) stays consistent.
fn board_without_routes(data: &[u8], settings: &mut RouterSettings) -> Result<(RoutingBoard, usize), String> {
    use fr_dsn::sexpr::Sexpr;
    let mut top = fr_dsn::sexpr::parse(data).map_err(|e| e.to_string())?;
    let Some(Sexpr::List(pcb)) = top.first_mut().filter(|s| s.as_list().is_some_and(|l| l.is("pcb"))) else {
        return Err("not a Specctra design".into());
    };
    let mut left_out = 0;
    for item in pcb.items.iter_mut() {
        if let Sexpr::List(wiring) = item {
            if wiring.is("wiring") {
                let before = wiring.items.len();
                wiring.items.retain(keep_in_session_mode);
                left_out += before - wiring.items.len();
            }
        }
    }
    let pcb = &*pcb;
    let dsn = fr_dsn::Dsn::from_pcb(pcb).map_err(|e| e.to_string())?;
    let order = fr_io::SourceOrder::from_pcb(pcb, &dsn);
    let design = fr_io::load_with_order(&dsn, &order).map_err(|e| e.to_string())?;
    let mut board = RoutingBoard::from_basic(fr_io::build_board(design));
    fr_io::post_load::apply_router_settings_for_loaded_board(&mut board, settings);
    fr_io::post_load::apply_immediate_post_load_processing(&mut board);
    Ok((board, left_out))
}

/// In `(wiring ...)`: traces (`path`, `polyline_path`) and vias go unless they are `(type fix)`;
/// copper areas and everything else stay.
fn keep_in_session_mode(item: &fr_dsn::sexpr::Sexpr) -> bool {
    let Some(l) = item.as_list() else { return true };
    let fixed = l.sublists().any(|s| s.is("type") && s.args().first().and_then(|a| a.as_atom()).is_some_and(|a| a.text.eq_ignore_ascii_case("fix")));
    match l.head() {
        Some("via") => fixed,
        Some("wire") => fixed || !l.sublists().any(|s| s.is("path") || s.is("polyline_path")),
        _ => true,
    }
}

/// The session reader skips what it does not understand, so a wrong file would load as
/// "no wiring" without a word; check the file really is a session.
fn check_is_ses(path: &Path, data: &[u8]) -> Result<(), CmdError> {
    let head: String = String::from_utf8_lossy(&data[..data.len().min(256)]).trim_start_matches('\u{feff}').trim_start().chars().take(20).collect();
    if !head.starts_with("(session") {
        return Err(CmdError::input(format!("{}: not a Specctra session (a .ses file starts with \"(session\"); --session takes the result of a route run", path.display())));
    }
    Ok(())
}

/// A KiCad board or an empty file is not a Specctra design; say what to do instead.
fn check_is_dsn(path: &Path, data: &[u8]) -> Result<(), CmdError> {
    let head: String = String::from_utf8_lossy(&data[..data.len().min(256)]).trim_start_matches('\u{feff}').trim_start().chars().take(40).collect();
    if data.is_empty() {
        return Err(CmdError::input(format!("{}: the file is empty", path.display())));
    }
    if head.starts_with("(kicad_pcb") || path.extension().is_some_and(|e| e.eq_ignore_ascii_case("kicad_pcb")) {
        return Err(CmdError::input(format!(
            "{}: this is a KiCad board, but fastroute reads Specctra designs (.dsn). Export one first (KiCad: File > Export > Specctra DSN, or `python3 route_cli.py --export-only --work-dir DIR board.kicad_pcb` of the KiCad plugin) and pass it with --input; \
             the routed result is a session (.ses) that --session adds on top",
            path.display()
        )));
    }
    if !head.starts_with("(pcb") && !head.starts_with("(PCB") {
        return Err(CmdError::input(format!("{}: not a Specctra design (a .dsn file starts with \"(pcb\")", path.display())));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_answers() {
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn non_dsn_input_is_explained() {
        let p = Path::new("x.kicad_pcb");
        let e = check_is_dsn(p, b"(kicad_pcb (version 20240108)").unwrap_err();
        assert!(e.message.contains("Specctra DSN") && e.message.contains("--export-only"), "{}", e.message);
        assert!(check_is_dsn(Path::new("a.dsn"), b"").unwrap_err().message.contains("empty"));
        assert!(check_is_dsn(Path::new("a.dsn"), b"hello").unwrap_err().message.contains("not a Specctra"));
        assert!(check_is_dsn(Path::new("a.dsn"), b"\xef\xbb\xbf  (pcb x)").is_ok());
        assert!(check_is_ses(Path::new("a.ses"), b"(session a.ses (base_design a))").is_ok());
        assert!(check_is_ses(Path::new("a.ses"), b"(pcb a)").unwrap_err().message.contains("not a Specctra session"));
        assert!(check_is_ses(Path::new("a.ses"), b"").is_err());
    }
}
