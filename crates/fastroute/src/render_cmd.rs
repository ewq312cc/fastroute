//! `fastroute render`: a PNG of the whole board or of one region, drawn from the vector
//! geometry (no browser, window system or network; see `docs/RENDERING.md`).

use std::path::{Path, PathBuf};

use fr_render::{check_grid, render, Annotations, BoardGeometry, RegionSpec, RenderSpec, Rendered, SizeSpec};

use crate::cmd::{check_output, opt, write_output, CmdError, Opt, Parsed, Spec};
use crate::extract;
use crate::load;
use crate::manifest::{relative_to, ImageEntry, Manifest, Selection};
use crate::reports::{self, ReportKind};

const OPTS: &[Opt] = &[
    opt("input", Some('i'), Some("FILE"), "Specctra design (.dsn) to draw (also the first positional argument)"),
    opt("session", Some('s'), Some("FILE"), "session (.ses) whose wiring is drawn on top of the design, e.g. the result of a route run"),
    opt("output", Some('o'), Some("FILE"), "PNG file to write (an existing file is only replaced with --force)"),
    opt("size", None, Some("N|WxH"), "image size in pixels: N = longer side (default 2048), WxH = exactly that size"),
    opt("px-per-mm", None, Some("S"), "scale in pixels per mm instead of --size (the image size follows from the region)"),
    opt("layers", Some('l'), Some("LIST"), "copper layers to draw: names (F.Cu,B.Cu), signal, top, bottom or all (default all)\none layer gives a single-layer image, several are overlaid"),
    opt("region-mm", None, Some("x,y,w,h"), "draw this rectangle: x,y = top-left corner in mm, y points down (KiCad's coordinates)"),
    opt("around", None, Some("REF"), "draw the whole package of this component plus --padding-mm"),
    opt("net", None, Some("NAME"), "draw everything of this net on the selected layers plus --padding-mm; repeat for\nseveral nets (a differential pair)"),
    opt("net-planes", None, None, "with --net: planes / zones of the net also count for the region (a pour covers the board)"),
    opt("from-report", None, Some("FILE"), "take problem locations from a report: fastroute --report, inspect.json or KiCad DRC JSON\n(markers DRC1.. for KiCad DRC); the region is built around them"),
    opt("select", None, Some("WHAT"), "which problems: all, unrouted, violations, drc or ids like OPEN1,CLR3 (default all; also works\nwithout --from-report on the problems of the board itself)"),
    opt("padding-mm", None, Some("N"), "margin around --around / --net / --select regions (default 2)"),
    opt("annotate", None, Some("LIST"), "labels: scale, layer, grid, grid=MM, nets, pads, refs, markers, all, none\n(default scale,layer,markers); labels never overlap, those that would are left out"),
    opt("no-airlines", None, None, "do not draw the unrouted connections"),
    opt("manifest", None, Some("FILE"), "where to write the manifest of this image (default: next to the image, <name>.manifest.json;\n'-' = standard output)"),
    opt("no-manifest", None, None, "do not write a manifest"),
    opt("force", None, None, "replace existing output files"),
    opt("quiet", Some('q'), None, "no summary line"),
];

pub const SPEC: Spec = Spec {
    name: "render",
    about: "draw the board (or a region of it) as a PNG, offline",
    usage: "render --input design.dsn --output board.png [options]",
    opts: OPTS,
    passthrough: &[],
    footer: "Coordinates are millimetres, x to the right, y downwards (as in KiCad and --report).\n\
             Every image gets a manifest (board.png -> board.manifest.json) with its region, scale and\n\
             the mm <-> pixel transform; see docs/AI_DATA_FORMAT.md.\n\
             Whole board:   fastroute render -i board.dsn -s routed.ses -o board.png --size 4096\n\
             One component: fastroute render -i board.dsn --around U1 --padding-mm 3 --px-per-mm 80 -o u1.png\n\
             A pair:        fastroute render -i board.dsn --net USB_DP --net USB_DN --layers F.Cu -o usb.png\n\
             From a report: fastroute render -i board.dsn --from-report drc.json --select DRC1,DRC2 -o drc.png",
};

/// How to draw one image; shared with `ai-pack`.
#[derive(Clone)]
pub struct Request {
    pub region: RegionSpec,
    pub selection: Selection,
    pub layers: Vec<usize>,
    pub size: SizeSpec,
    pub annotations: Annotations,
    pub airlines: bool,
}

pub fn parse_size(p: &Parsed) -> Result<SizeSpec, CmdError> {
    match (p.value("size"), p.f64("px-per-mm")?) {
        (Some(_), Some(_)) => Err(CmdError::usage("--size and --px-per-mm cannot be combined: one fixes the pixels, the other the scale")),
        (Some(s), None) => Ok(SizeSpec::parse(s)?),
        (None, Some(s)) => Ok(SizeSpec::PxPerMm(s)),
        (None, None) => Ok(SizeSpec::LongSide(2048)),
    }
}

/// The region options of `p` as a [`RegionSpec`] (at most one kind may be given). `report_ids`
/// are the issues of `--from-report`, the default selection when `--select` is not given.
pub fn region_from_args(p: &Parsed, geom: &BoardGeometry, report_ids: Option<&[String]>) -> Result<(RegionSpec, Selection), CmdError> {
    let kinds: Vec<&str> = ["region-mm", "around", "net"].into_iter().filter(|k| p.has(k)).chain((p.has("from-report") || p.has("select")).then_some("select")).collect();
    if kinds.len() > 1 {
        return Err(CmdError::usage(format!("choose one way to select the region, not {}", kinds.iter().map(|k| format!("--{}", if *k == "select" { "from-report/--select" } else { k })).collect::<Vec<_>>().join(" and "))));
    }
    let padding = p.f64("padding-mm")?;
    if padding.is_some_and(|v| v < 0.0) {
        return Err(CmdError::usage("--padding-mm must be 0 or more"));
    }
    if kinds.is_empty() {
        if padding.is_some() {
            return Err(CmdError::usage("--padding-mm only applies to --around, --net and --select"));
        }
        return Ok((RegionSpec::Board { margin_mm: None }, Selection { kind: "board".into(), targets: vec![], padding_mm: None }));
    }
    match kinds[0] {
        "region-mm" => {
            if padding.is_some() {
                return Err(CmdError::usage("--padding-mm does not apply to --region-mm (the rectangle is used as given)"));
            }
            let r = fr_render::parse_region_mm(p.value("region-mm").unwrap())?;
            Ok((RegionSpec::Rect(r), Selection { kind: "region".into(), targets: vec![], padding_mm: None }))
        }
        "around" => {
            let refdes = p.value("around").unwrap().to_string();
            let pad = padding.unwrap_or(2.0);
            Ok((RegionSpec::Component { refdes: refdes.clone(), padding_mm: pad }, Selection { kind: "component".into(), targets: vec![refdes], padding_mm: Some(pad) }))
        }
        "net" => {
            let names: Vec<String> = p.values("net").into_iter().map(str::to_string).collect();
            let pad = padding.unwrap_or(2.0);
            Ok((
                RegionSpec::Nets { names: names.clone(), padding_mm: pad, include_planes: p.flag("net-planes") },
                Selection { kind: "net".into(), targets: names, padding_mm: Some(pad) },
            ))
        }
        _ => {
            let pad = padding.unwrap_or(3.0);
            let ids: Vec<String> = match (p.value("select"), report_ids) {
                (None, Some([])) => return Err(CmdError::input("the report lists no problems, so there is no region to select")),
                (None, Some(ids)) => ids.to_vec(),
                (spec, _) => reports::select_issues(&geom.issues, spec.unwrap_or("all")).map_err(CmdError::usage)?.iter().map(|i| i.id.clone()).collect(),
            };
            Ok((RegionSpec::Issues { ids: ids.clone(), padding_mm: pad }, Selection { kind: "issues".into(), targets: ids, padding_mm: Some(pad) }))
        }
    }
}

/// Draws one image of `geom`.
pub fn render_one(geom: &BoardGeometry, req: &Request) -> Result<Rendered, CmdError> {
    let region = fr_render::resolve(geom, &req.region, &req.layers)?;
    let mut spec = RenderSpec::new(region, req.size, req.layers.clone());
    spec.annotations = req.annotations.clone();
    spec.airlines = req.airlines;
    check_grid(&spec)?;
    Ok(render(geom, &spec)?)
}

pub fn image_entry(file: &Path, manifest_dir: &Path, purpose: &str, selection: &Selection, png: &[u8], r: &Rendered) -> ImageEntry {
    ImageEntry {
        file: relative_to(manifest_dir, file),
        purpose: purpose.into(),
        selection: selection.clone(),
        bytes: png.len() as u64,
        sha256: load::sha256_hex(png),
        meta: r.meta.clone(),
    }
}

pub fn run(args: &[String]) -> Result<(), CmdError> {
    let p = SPEC.parse(args)?;
    if p.help {
        print!("{}", SPEC.help_text());
        return Ok(());
    }
    let input: PathBuf = match (p.path("input"), p.positional.as_slice()) {
        (Some(i), []) => i,
        (None, [one]) => PathBuf::from(one),
        (Some(_), _) => return Err(CmdError::usage(format!("unexpected argument '{}': the input is given with --input", p.positional[0]))),
        (None, []) => return Err(CmdError::usage("no input design (--input design.dsn)")),
        (None, _) => return Err(CmdError::usage("more than one input design given")),
    };
    let output = p.path("output").ok_or_else(|| CmdError::usage("no output file (--output board.png)"))?;
    let session = p.path("session");
    let manifest_path = match (p.path("manifest"), p.flag("no-manifest")) {
        (Some(_), true) => return Err(CmdError::usage("--manifest and --no-manifest contradict each other")),
        (Some(m), false) => Some(m),
        // every image comes with its metadata: the region, scale and transform are what make it usable
        (None, false) => Some(output.with_extension("manifest.json")),
        (None, true) => None,
    };
    let force = p.flag("force");
    // refuse early, before the board is loaded
    let mut inputs: Vec<&Path> = vec![&input];
    if let Some(s) = &session {
        inputs.push(s);
    }
    check_output(&output, &inputs, force)?;
    let to_stdout = manifest_path.as_deref() == Some(Path::new("-"));
    if let Some(m) = manifest_path.as_deref().filter(|_| !to_stdout) {
        let mut all = inputs.clone();
        all.push(&output);
        check_output(m, &all, force)?;
    }
    let size = parse_size(&p)?;
    let annotations = match p.value("annotate") {
        Some(a) => Annotations::parse(a).map_err(CmdError::usage)?,
        None => Annotations::default(),
    };

    let loaded = load::load(&input, session.as_deref())?;
    let mut analysis = extract::analyse(&loaded.board, &loaded.name, &loaded.synthetic);
    let mut report_ids: Option<Vec<String>> = None;
    if let Some(report) = p.path("from-report") {
        let parsed = reports::read_report(&report)?;
        report_ids = Some(parsed.issues.iter().map(|i| i.id.clone()).collect());
        match parsed.kind {
            ReportKind::KicadDrc => analysis.geometry.issues.extend(parsed.issues),
            _ => analysis.geometry.issues = parsed.issues,
        }
    }
    let geom = &analysis.geometry;
    let layers = geom.parse_layers(p.value("layers").unwrap_or("all"))?;
    let (region, selection) = region_from_args(&p, geom, report_ids.as_deref())?;
    let req = Request { region, selection, layers, size, annotations, airlines: !p.flag("no-airlines") };
    let rendered = render_one(geom, &req)?;
    let png = rendered.png_bytes()?;
    write_output(&output, &png, &inputs, force)?;

    let mut manifest = Manifest::new("fastroute render", Some(loaded.input.clone()), loaded.session.clone());
    manifest.warnings = loaded.warnings.clone();
    let dir = manifest_path.as_deref().filter(|_| !to_stdout).and_then(Path::parent).map(Path::to_path_buf).unwrap_or_else(|| output.parent().map(Path::to_path_buf).unwrap_or_default());
    let dir = if dir.as_os_str().is_empty() { PathBuf::from(".") } else { dir };
    manifest.images.push(image_entry(&output, &dir, "region", &req.selection, &png, &rendered));
    if let Some(m) = &manifest_path {
        if to_stdout {
            print!("{}", manifest.to_json());
        } else {
            write_output(m, manifest.to_json().as_bytes(), &all_paths(&inputs, &output), force)?;
        }
    }
    if !p.flag("quiet") {
        for w in &loaded.warnings {
            eprintln!("warning: {w}");
        }
        if !to_stdout {
            let m = &rendered.meta;
            println!(
                "wrote {} ({} x {} px, {:.2} px/mm, region x {:.2}..{:.2} mm, y {:.2}..{:.2} mm, layers {})",
                output.display(),
                m.width_px,
                m.height_px,
                m.px_per_mm,
                m.region_mm.x,
                m.region_mm.x + m.region_mm.width,
                m.region_mm.y,
                m.region_mm.y + m.region_mm.height,
                m.layers.join("+")
            );
        }
    }
    Ok(())
}

fn all_paths<'a>(inputs: &[&'a Path], output: &'a Path) -> Vec<&'a Path> {
    let mut v = inputs.to_vec();
    v.push(output);
    v
}
