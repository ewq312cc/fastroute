//! Shared by the tests of the embedding providers and the store backends: a workspace with fixture documents, a command runner
//! and a mock OpenAI-compatible embeddings service that records what it receives.
#![allow(dead_code)]

use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_hcp-manager")
}

pub fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("hcp-manager-t-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

pub fn run(ws: &Path, args: &[&str]) -> Output {
    run_env(ws, args, &[])
}

/// With extra environment variables, and without any proxy (the mock is on this machine, or on this machine's other address).
pub fn run_env(ws: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut c = Command::new(bin());
    c.arg("--workspace").arg(ws).args(args).env("HCP_TODAY", "2026-11-16").env("NO_PROXY", "*").env("no_proxy", "*");
    for k in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy", "ALL_PROXY"] {
        c.env_remove(k);
    }
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().unwrap()
}

pub fn code(o: &Output) -> i32 {
    o.status.code().unwrap()
}
pub fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}
pub fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}
pub fn jsn(o: &Output) -> Value {
    serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("not JSON ({e}): {}\n{}", out(o), err(o)))
}

const DS: [&str; 4] = [
    "Acme AC-100 Datasheet\n\n1 Overview\n\nThe AC-100 is a fictional wireless module for tests.\n",
    "3.3 Strapping pins\n\nIO0 is sampled at reset; the internal pull-up selects normal boot.\nAn external driver on IO0 changes the boot mode.\n",
    "5.1 Power supply\n\nAdd a bulk capacitor of at least 10 uF at the supply input. Decoupling capacitors of 100 nF belong next to each supply pin.\n",
    "6.2 Antenna keep-out\n\nKeep the area around the antenna free of copper on all layers.\n",
];
const DSU: [&str; 2] = [
    "Acme AC-100U Datasheet\n\n1 Overview\n\nThe AC-100U is a fictional wireless module with an external antenna connector.\n",
    "6.2 Antenna keep-out\n\nThe AC-100U needs no antenna keep-out: keep the area around the connector free of components.\n",
];
const HDG: &str = "<html><body><h2>4.1 Reset</h2><p>The EN pin needs a pull-up resistor and a capacitor to ground for a clean reset.</p><h2>4.2 Supply</h2><p>Place decoupled capacitors close to the module and keep the supply traces short.</p></body></html>";

pub struct Fx {
    pub ws: PathBuf,
    pub dir: PathBuf,
}

/// A workspace with three documents added and cached (no index yet): the datasheet of the AC-100, the one of the similar AC-100U, and a guide.
pub fn fixture(name: &str) -> Fx {
    let dir = tmp(name);
    let ws = dir.join("ws");
    for (f, t) in [("ac100-ds.txt", DS.join("\x0c")), ("ac100u-ds.txt", DSU.join("\x0c")), ("hdg.html", HDG.to_string())] {
        std::fs::write(dir.join(f), t).unwrap();
    }
    assert_eq!(code(&run(&ws, &["init"])), 0);
    let add = |id: &str, level: &str, file: &str, applies: &str| {
        let o = run(&ws, &["sources", "add", "--id", id, "--title", id, "--level", level, "--version", "v1.0", "--file", dir.join(file).to_str().unwrap(), "--applies-to", applies]);
        assert_eq!(code(&o), 0, "{}", err(&o));
    };
    add("DOC-AC-100-DS", "official_manual", "ac100-ds.txt", "AC-100");
    add("DOC-AC-100U-DS", "official_manual", "ac100u-ds.txt", "AC-100U");
    add("DOC-AC-100-HDG", "official_guide", "hdg.html", "AC-100");
    let o = run(&ws, &["sources", "sync"]);
    assert_eq!(code(&o), 0, "{}{}", out(&o), err(&o));
    Fx { ws, dir }
}

pub fn write_embedding(ws: &Path, yaml: &str) {
    std::fs::write(ws.join("embedding.yaml"), yaml).unwrap();
}

pub fn search(ws: &Path, q: &str, extra: &[&str]) -> Value {
    let mut a = vec!["--json", "search", q];
    a.extend_from_slice(extra);
    let o = run(ws, &a);
    assert_eq!(code(&o), 0, "{}{}", out(&o), err(&o));
    jsn(&o)
}

pub fn curl_here() -> bool {
    Command::new("curl").arg("--version").output().map(|o| o.status.success()).unwrap_or(false)
}

/// An address of this machine that is not the loopback one (None where there is none).
pub fn lan_address() -> Option<String> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then(|| ip.to_string())
}

// ---- a mock OpenAI-compatible service ---------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Request {
    pub method: String,
    pub path: String,
    /// lower-cased names
    pub headers: HashMap<String, String>,
    pub body: Value,
}

impl Request {
    pub fn inputs(&self) -> Vec<String> {
        self.body["input"].as_array().map(|a| a.iter().filter_map(|t| t.as_str().map(str::to_string)).collect()).unwrap_or_default()
    }
}

pub struct Response {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
    pub delay_ms: u64,
}

impl Response {
    pub fn json(status: u16, v: Value) -> Response {
        Response { status, headers: vec![], body: v.to_string(), delay_ms: 0 }
    }
}

pub type Handler = Arc<dyn Fn(&Request, usize) -> Response + Send + Sync>;

pub struct Mock {
    pub port: u16,
    pub requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

/// The vector the mock "embeds" a text as: a fixed function of its bytes, `dim` numbers (not unit length: the provider normalises).
pub fn fake_vector(text: &str, dim: usize) -> Vec<f64> {
    let mut v = vec![0.01f64; dim];
    for (i, b) in text.bytes().enumerate() {
        v[(b as usize + i) % dim] += 1.0;
    }
    v
}

/// The normal service: `dim` is taken from the model name (`m8` -> 8, `m16` -> 16, anything else 8); the answers come back in reverse order
/// when `reverse` is set (with their `index`).
pub fn fake_service(reverse: bool) -> Handler {
    Arc::new(move |r: &Request, _n| {
        let dim = r.body["model"].as_str().and_then(|m| m.strip_prefix('m')).and_then(|d| d.parse().ok()).unwrap_or(8usize);
        let mut data: Vec<Value> = r.inputs().iter().enumerate().map(|(i, t)| json!({"object": "embedding", "index": i, "embedding": fake_vector(t, dim)})).collect();
        if reverse {
            data.reverse();
        }
        Response::json(200, json!({"object": "list", "data": data, "model": r.body["model"], "usage": {"prompt_tokens": 1, "total_tokens": 1}}))
    })
}

impl Mock {
    pub fn start(bind: &str, handler: Handler) -> Mock {
        let listener = TcpListener::bind(format!("{bind}:0")).unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (requests, stop) = (Arc::new(Mutex::new(Vec::new())), Arc::new(AtomicBool::new(false)));
        let (reqs, st) = (requests.clone(), stop.clone());
        let handle = std::thread::spawn(move || {
            while !st.load(Ordering::SeqCst) {
                let Ok((mut sock, _)) = listener.accept() else {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                    continue;
                };
                sock.set_nonblocking(false).unwrap();
                let mut buf = Vec::new();
                let mut chunk = [0u8; 8192];
                let (mut header_end, mut want) = (None, 0usize);
                loop {
                    let n = sock.read(&mut chunk).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if header_end.is_none() {
                        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            header_end = Some(p + 4);
                            let head = String::from_utf8_lossy(&buf[..p]).to_ascii_lowercase();
                            want = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse().ok()).unwrap_or(0);
                        }
                    }
                    if let Some(h) = header_end {
                        if buf.len() >= h + want {
                            break;
                        }
                    }
                }
                let h = header_end.unwrap_or(buf.len());
                let head = String::from_utf8_lossy(&buf[..h.min(buf.len())]).into_owned();
                let mut lines = head.lines();
                let first = lines.next().unwrap_or("");
                let mut parts = first.split_whitespace();
                let (method, path) = (parts.next().unwrap_or("").to_string(), parts.next().unwrap_or("").to_string());
                let headers: HashMap<String, String> = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string())).collect();
                let req = Request { method, path, headers, body: serde_json::from_slice(&buf[h..]).unwrap_or(Value::Null) };
                let n = {
                    let mut g = reqs.lock().unwrap();
                    g.push(req.clone());
                    g.len() - 1
                };
                let resp = handler(&req, n);
                if resp.delay_ms > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(resp.delay_ms));
                }
                let mut head = format!("HTTP/1.1 {} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n", resp.status, resp.body.len());
                for (k, v) in &resp.headers {
                    head.push_str(&format!("{k}: {v}\r\n"));
                }
                head.push_str("\r\n");
                let _ = sock.write_all(head.as_bytes());
                let _ = sock.write_all(resp.body.as_bytes());
            }
        });
        Mock { port, requests, stop, handle: Some(handle) }
    }

    pub fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    pub fn all(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }

    pub fn texts(&self) -> Vec<String> {
        self.all().iter().flat_map(Request::inputs).collect()
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Every file under `dir` (recursively), read as bytes: for "this string is nowhere in the workspace".
pub fn all_files(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                v.extend(all_files(&p));
            } else if let Ok(b) = std::fs::read(&p) {
                v.push((p, b));
            }
        }
    }
    v
}

pub fn contains(hay: &[u8], needle: &str) -> bool {
    !needle.is_empty() && hay.windows(needle.len()).any(|w| w == needle.as_bytes())
}
