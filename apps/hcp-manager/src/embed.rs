//! Embedding providers: how the text of a block becomes a vector. The vector layer is for finding candidate passages; nothing
//! downstream reads a vector or a similarity (a pack, an IR and every verdict are made without them).
//!
//! Four providers, chosen in `<workspace>/embedding.yaml` (default: the first, with no file at all; the file may also hold several named
//! profiles, of which one is in force):
//!
//! * `hashing`: local, built in, deterministic. A feature-hashing embedding of words, word pairs and character trigrams. It is
//!   **lexical**: it finds passages that share words or word parts with the query (spelling variants, parts of identifiers, phrases),
//!   not passages that merely mean the same. It needs no model, no network and no download.
//! * `command`: a local program that you name (an argument list, no shell). It reads `{"texts": [...]}` on standard input and
//!   writes `{"model", "version", "dim", "vectors": [[...]]}` on standard output. This is how a real model (a sentence transformer,
//!   a llama.cpp server wrapper) is plugged in. What the program does with the text is its own business; hcp-manager only starts it.
//! * `http`: an OpenAI-style embeddings endpoint (`POST {"model", "input": [...]}`, answer `{"data": [{"index", "embedding"}]}`
//!   or `{"embeddings": [...]}`). **An endpoint on this machine (localhost) is local. Any other endpoint is remote**, and a remote
//!   provider never receives anything unless all of these hold: the configuration says `allow_remote: true`, the command is run with
//!   `--authorize-remote`, and, per document, the inventory says `embed_remote: true`. The query of a search is sent only under the
//!   first two. The API key is read from the environment variable that the configuration names; it is never written anywhere.
//!
//! * `openai`: an OpenAI-compatible embeddings service (`POST <base_url>/embeddings`, `{"model", "input": [...]}`, answer
//!   `{"data": [{"index", "embedding"}]}`): `base_url` with or without `/v1`, a batch size, a timeout, a limited number of retries (timeouts,
//!   429, 5xx), the API key from an environment variable or an external command, a dimension that is checked on every answer. Same
//!   remote rules as `http`; the key and the text of your documents never appear in an error message.
//!
//! The configuration is part of the identity of the vector index (`embedder_key`): another provider, model, version, dimension,
//! command or URL makes the existing vectors stale.

use crate::error::{Error, Result};
use crate::workspace::Workspace;
use fr_constraint::hcp::sha256_hex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const CONFIG_SCHEMA_TEXT: &str = include_str!("../../../skills/hardware-constraint-design/schemas/embedding-config-v1.schema.json");

pub const DEFAULT_DIM: usize = 384;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmbeddingConfig {
    #[serde(default = "one_zero")]
    pub embedding_version: String,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dim: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub allow_remote: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_s: Option<u64>,
    /// How much the similarity counts against the keyword score in a hybrid search: a block's score is its keyword score relative to
    /// the best keyword hit plus this weight times its similarity above the noise floor (0 is keywords only). Default 0.5 for the lexical `hashing` provider (a weaker signal than the words themselves), 1.0 for a
    /// model you plug in. It does not change the vectors, so it is not part of their identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector_weight: Option<f64>,
    /// provider `openai`: the service address, with or without `/v1`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// provider `openai`: the path of the endpoint when it is not `<base_url>/embeddings` (after the version prefix)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embeddings_path: Option<String>,
    /// provider `openai`: send `dimensions: dim` with each request
    #[serde(default)]
    pub send_dimensions: bool,
    /// An external program that prints the API key (no shell). The key is read when a request is made and goes nowhere else.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_command: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retries: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_backoff_ms: Option<u64>,
}

impl EmbeddingConfig {
    pub fn effective_vector_weight(&self) -> f64 {
        self.vector_weight.unwrap_or(if self.provider == "hashing" { 0.5 } else { 1.0 }).clamp(0.0, 4.0)
    }
}

fn one_zero() -> String {
    "1.0".into()
}

impl EmbeddingConfig {
    pub fn local_default() -> EmbeddingConfig {
        EmbeddingConfig { embedding_version: one_zero(), provider: "hashing".into(), model: None, version: None, dim: None, command: None, url: None, api_key_env: None, allow_remote: false, batch: None, timeout_s: None, vector_weight: None, base_url: None, embeddings_path: None, send_dimensions: false, api_key_command: None, retries: None, retry_backoff_ms: None }
    }
}

/// The embedding settings in force: the profile (its name and its provider configuration) and the store backend the file asks for.
#[derive(Debug, Clone, Serialize)]
pub struct Settings {
    /// `default` for a flat file or no file
    pub profile: String,
    pub config: EmbeddingConfig,
    /// every profile the file defines
    pub profiles: Vec<String>,
    /// `store.backend` of the file, if it says
    pub backend: Option<String>,
}

fn read_config_value(ws: &Workspace) -> Result<Option<Value>> {
    let p = ws.embedding_config_path();
    if !p.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&p).map_err(|e| Error::input(format!("{}: {e}", p.display())))?;
    let y: serde_yaml::Value = serde_yaml::from_str(&text).map_err(|e| Error::input(format!("{}: not valid YAML: {e}", p.display())))?;
    let v: Value = serde_json::to_value(&y).map_err(|e| Error::input(format!("{}: {e}", p.display())))?;
    let schema: Value = serde_json::from_str(CONFIG_SCHEMA_TEXT).expect("the embedded schema is valid JSON");
    let errors = fr_constraint::jsonschema::validate(&schema, &v);
    if let Some(e) = errors.first() {
        return Err(Error::input(format!("{}: not a valid embedding configuration: {}: {} ({} problem(s))", p.display(), e.path, e.message, errors.len())));
    }
    Ok(Some(v))
}

/// The settings of the workspace: `embedding.yaml` (one provider written flat is the profile `default`, or several named `profiles` of
/// which `--profile` or `active_profile` picks exactly one), or the local default when there is no file. A name that does not exist is
/// an error, never a fallback to another profile.
pub fn load_settings(ws: &Workspace) -> Result<Settings> {
    let Some(v) = read_config_value(ws)? else {
        if let Some(p) = ws.profile.as_deref().filter(|p| *p != "default") {
            return Err(Error::input(format!("no embedding profile '{p}': there is no embedding.yaml, only the local default")));
        }
        return Ok(Settings { profile: "default".into(), config: EmbeddingConfig::local_default(), profiles: vec!["default".into()], backend: None });
    };
    let file = ws.embedding_config_path().display().to_string();
    let backend = v["store"]["backend"].as_str().map(str::to_string);
    if let Some(profiles) = v.get("profiles").and_then(Value::as_object) {
        if v.get("provider").is_some() {
            return Err(Error::input(format!("{file}: give either one provider written flat or `profiles`, not both")));
        }
        let names: Vec<String> = profiles.keys().cloned().collect();
        let want = match (ws.profile.clone().or_else(|| v["active_profile"].as_str().map(str::to_string)), names.len()) {
            (Some(w), _) => w,
            (None, 1) => names[0].clone(),
            (None, 0) => return Err(Error::input(format!("{file}: `profiles` is empty"))),
            (None, _) => return Err(Error::input(format!("{file}: there are several profiles ({}): name the one in force with `active_profile` or --profile", names.join(", ")))),
        };
        let cfg = profiles.get(&want).ok_or_else(|| Error::input(format!("{file}: no embedding profile '{want}' (the file has: {})", names.join(", "))))?;
        let config: EmbeddingConfig = serde_json::from_value(cfg.clone()).map_err(|e| Error::input(format!("{file}: profile {want}: {e}")))?;
        return Ok(Settings { profile: want, config, profiles: names, backend });
    }
    if v.get("provider").is_none() {
        if backend.is_some() {
            // only `store:` is given: the default provider with the store that was asked for
            if let Some(p) = ws.profile.as_deref().filter(|p| *p != "default") {
                return Err(Error::input(format!("{file}: no embedding profile '{p}'")));
            }
            return Ok(Settings { profile: "default".into(), config: EmbeddingConfig::local_default(), profiles: vec!["default".into()], backend });
        }
        return Err(Error::input(format!("{file}: needs `provider` (one provider written flat) or `profiles`")));
    }
    if let Some(p) = ws.profile.as_deref().filter(|p| *p != "default") {
        return Err(Error::input(format!("{file}: no embedding profile '{p}': the file has one provider written flat, which is the profile `default`")));
    }
    let config: EmbeddingConfig = serde_json::from_value(v.clone()).map_err(|e| Error::input(format!("{file}: {e}")))?;
    Ok(Settings { profile: "default".into(), config, profiles: vec!["default".into()], backend })
}

/// The provider configuration in force (see [`load_settings`]).
pub fn load_config(ws: &Workspace) -> Result<EmbeddingConfig> {
    Ok(load_settings(ws)?.config)
}

/// What an embedder is, known from the configuration alone (no program is started, nothing is sent).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EmbedderInfo {
    pub provider: String,
    pub model: String,
    pub version: String,
    pub dim: usize,
    /// Is the text sent to a machine other than this one?
    pub remote: bool,
    pub config_sha256: String,
    /// `provider:model:version:dim:<config digest>`: the identity of the vectors made with it
    pub key: String,
    /// The host of the endpoint, for messages
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// The URL that requests go to (provider `openai`: after the version prefix is normalised)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
}

fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let auth = rest.split(['/', '?', '#']).next()?;
    let auth = auth.rsplit('@').next()?;
    let host = if let Some(stripped) = auth.strip_prefix('[') { stripped.split(']').next()? } else { auth.split(':').next()? };
    Some(host.to_ascii_lowercase())
}

fn is_loopback(host: &str) -> bool {
    host == "localhost" || host == "::1" || host.starts_with("127.")
}

/// The embeddings URL of an OpenAI-compatible service. `base_url` may or may not contain the version prefix: `https://h`,
/// `https://h/v1` and `https://h/v1/embeddings` all mean `https://h/v1/embeddings`; a prefix of another shape (`/api/paas/v4`) is
/// kept and `/embeddings` is appended; `embeddings_path` replaces everything after the host. No credentials in the URL.
pub fn openai_endpoint(base: &str, path_override: Option<&str>) -> Result<String> {
    // an address with credentials in it is never echoed back
    let shown = match base.split_once("://") {
        Some((s, rest)) if rest.split('/').next().is_some_and(|a| a.contains('@')) => format!("{s}://***@{}", rest.split_once('@').map(|x| x.1).unwrap_or("")),
        _ => base.to_string(),
    };
    let bad = |why: &str| Error::input(format!("embedding.yaml: the base_url '{shown}' {why}"));
    let (scheme, rest) = base.split_once("://").ok_or_else(|| bad("is not http(s)://host/..."))?;
    if scheme != "http" && scheme != "https" {
        return Err(bad("is not http(s)://host/..."));
    }
    if base.contains(['?', '#']) {
        return Err(bad("has a query or a fragment: give only the address (https://host or https://host/v1)"));
    }
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    if authority.is_empty() {
        return Err(bad("has no host"));
    }
    if authority.contains('@') {
        return Err(bad("has credentials in the address: the key goes in `api_key_env` or `api_key_command`, never in the URL"));
    }
    let origin = format!("{scheme}://{authority}");
    if let Some(over) = path_override {
        return Ok(format!("{origin}{over}"));
    }
    let path = path.trim_end_matches('/');
    let last = path.rsplit('/').next().unwrap_or("");
    let versioned = last.len() >= 2 && last.starts_with('v') && last[1..].starts_with(|c: char| c.is_ascii_digit());
    let full = if path.ends_with("/embeddings") {
        path.to_string()
    } else if versioned {
        format!("{path}/embeddings")
    } else {
        format!("{path}/v1/embeddings")
    };
    Ok(format!("{origin}{full}"))
}

pub fn describe(cfg: &EmbeddingConfig) -> Result<EmbedderInfo> {
    let need = |v: &Option<String>, what: &str| v.clone().filter(|s| !s.is_empty()).ok_or_else(|| Error::input(format!("embedding.yaml: provider {} needs `{what}`", cfg.provider)));
    let mut endpoint: Option<String> = None;
    let (model, version, dim, remote, host) = match cfg.provider.as_str() {
        "hashing" => {
            let dim = cfg.dim.unwrap_or(DEFAULT_DIM);
            if !(16..=4096).contains(&dim) {
                return Err(Error::input(format!("embedding.yaml: dim {dim} is outside 16..4096")));
            }
            (cfg.model.clone().unwrap_or_else(|| "hash-ngram".into()), cfg.version.clone().unwrap_or_else(|| "1".into()), dim, false, None)
        }
        "command" => {
            let cmd = cfg.command.as_ref().filter(|c| !c.is_empty()).ok_or_else(|| Error::input("embedding.yaml: provider command needs `command` (the program and its arguments)"))?;
            let _ = cmd;
            let dim = cfg.dim.ok_or_else(|| Error::input("embedding.yaml: provider command needs `dim`"))?;
            (need(&cfg.model, "model")?, need(&cfg.version, "version")?, dim, false, None)
        }
        "http" => {
            let url = need(&cfg.url, "url")?;
            let host = host_of(&url).ok_or_else(|| Error::input(format!("embedding.yaml: the url '{url}' is not http(s)://host/...")))?;
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return Err(Error::input(format!("embedding.yaml: the url '{url}' is not http(s)")));
            }
            let dim = cfg.dim.ok_or_else(|| Error::input("embedding.yaml: provider http needs `dim`"))?;
            (need(&cfg.model, "model")?, cfg.version.clone().unwrap_or_else(|| "1".into()), dim, !is_loopback(&host), Some(host))
        }
        "openai" => {
            let base = need(&cfg.base_url, "base_url")?;
            let url = openai_endpoint(&base, cfg.embeddings_path.as_deref())?;
            let host = host_of(&url).ok_or_else(|| Error::input(format!("embedding.yaml: the base_url '{base}' is not http(s)://host/...")))?;
            let dim = cfg.dim.ok_or_else(|| Error::input("embedding.yaml: provider openai needs `dim` (the number of numbers per vector; it is checked on every answer)"))?;
            if cfg.api_key_env.is_some() && cfg.api_key_command.is_some() {
                return Err(Error::input("embedding.yaml: give the API key either with `api_key_env` or with `api_key_command`, not both"));
            }
            endpoint = Some(url);
            (need(&cfg.model, "model")?, cfg.version.clone().unwrap_or_else(|| "1".into()), dim, !is_loopback(&host), Some(host))
        }
        other => return Err(Error::input(format!("embedding.yaml: unknown provider '{other}' (hashing, command, http, openai)"))),
    };
    let mut ident = json!({"provider": cfg.provider, "model": model, "version": version, "dim": dim, "command": cfg.command, "url": if cfg.provider == "openai" { json!(endpoint) } else { json!(cfg.url) }});
    if cfg.provider == "openai" && cfg.send_dimensions {
        ident["send_dimensions"] = json!(true);
    }
    let config_sha256 = sha256_hex(fr_constraint::hcp::canonical_json(&ident).as_bytes());
    let key = format!("{}:{model}:{version}:{dim}:{}", cfg.provider, &config_sha256[..8]);
    Ok(EmbedderInfo { provider: cfg.provider.clone(), model, version, dim, remote, config_sha256, key, host, endpoint })
}

pub trait Embedder {
    fn info(&self) -> &EmbedderInfo;
    /// One unit-length vector per text, in order.
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
    fn batch(&self) -> usize {
        64
    }
}

/// The embedder of the configuration. A remote one is refused unless the configuration and the caller both authorise it.
pub fn make(cfg: &EmbeddingConfig, authorize_remote: bool, base: &std::path::Path) -> Result<Box<dyn Embedder>> {
    let info = describe(cfg)?;
    if info.remote {
        let host = info.host.clone().unwrap_or_default();
        if !cfg.allow_remote {
            return Err(Error::input(format!("embedding: {host} is a remote service and the text of your documents would be sent to it: set `allow_remote: true` in embedding.yaml if that is what you want")));
        }
        if !authorize_remote {
            return Err(Error::input(format!("embedding: {host} is a remote service: nothing is sent unless this command is run with --authorize-remote")));
        }
    }
    Ok(match cfg.provider.as_str() {
        "hashing" => Box::new(Hashing { info }),
        "command" => Box::new(CommandProvider { info, argv: cfg.command.clone().unwrap_or_default(), cwd: base.to_path_buf(), timeout: Duration::from_secs(cfg.timeout_s.unwrap_or(120)), batch: cfg.batch.unwrap_or(32).max(1) }),
        "openai" => Box::new(OpenAi {
            endpoint: info.endpoint.clone().unwrap_or_default(),
            info,
            key_env: cfg.api_key_env.clone(),
            key_cmd: cfg.api_key_command.clone(),
            cwd: base.to_path_buf(),
            timeout: cfg.timeout_s.unwrap_or(60),
            batch: cfg.batch.unwrap_or(32).clamp(1, 2048),
            retries: cfg.retries.unwrap_or(2).min(10),
            backoff_ms: cfg.retry_backoff_ms.unwrap_or(500),
            send_dimensions: cfg.send_dimensions,
        }),
        _ => Box::new(Http { info, url: cfg.url.clone().unwrap_or_default(), key_env: cfg.api_key_env.clone(), timeout: cfg.timeout_s.unwrap_or(120), batch: cfg.batch.unwrap_or(32).max(1) }),
    })
}

// ---- hashing ---------------------------------------------------------------------------------------------------------------

struct Hashing {
    info: EmbedderInfo,
}

impl Embedder for Hashing {
    fn info(&self) -> &EmbedderInfo {
        &self.info
    }
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| hash_embedding(t, self.info.dim)).collect())
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Words, word pairs and character trigrams, hashed into `dim` signed buckets with a sub-linear weight, unit length. The same
/// text gives the same vector on every machine.
pub fn hash_embedding(text: &str, dim: usize) -> Vec<f32> {
    let words = crate::search::tokens(text);
    let mut feats: BTreeMap<String, f64> = BTreeMap::new();
    let mut add = |f: String, w: f64| {
        *feats.entry(f).or_insert(0.0) += w;
    };
    for (i, w) in words.iter().enumerate() {
        add(format!("w:{w}"), 1.0);
        if let Some(n) = words.get(i + 1) {
            add(format!("b:{w} {n}"), 0.8);
        }
        let chars: Vec<char> = format!("<{w}>").chars().collect();
        if w.chars().count() >= 3 {
            for t in chars.windows(3) {
                add(format!("c:{}", t.iter().collect::<String>()), 0.35);
            }
        }
    }
    let mut v = vec![0f64; dim];
    for (f, w) in &feats {
        let h = fnv1a(f.as_bytes());
        let sign = if (h >> 40) & 1 == 0 { 1.0 } else { -1.0 };
        v[(h % dim as u64) as usize] += sign * (1.0 + w.ln_1p());
    }
    let norm = v.iter().map(|x| x * x).sum::<f64>().sqrt();
    if norm == 0.0 {
        return vec![0.0; dim];
    }
    v.iter().map(|x| (x / norm) as f32).collect()
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn unit(mut v: Vec<f32>, what: &str) -> Result<Vec<f32>> {
    let n = v.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>().sqrt();
    if !n.is_finite() || n == 0.0 {
        return Err(Error::input(format!("{what}: a vector of length zero or not a number")));
    }
    for x in &mut v {
        *x = (*x as f64 / n) as f32;
    }
    Ok(v)
}

fn check_vectors(info: &EmbedderInfo, n: usize, vectors: Vec<Vec<f32>>, who: &str) -> Result<Vec<Vec<f32>>> {
    if vectors.len() != n {
        return Err(Error::input(format!("{who}: {n} texts were sent, {} vectors came back", vectors.len())));
    }
    vectors
        .into_iter()
        .map(|v| {
            if v.len() != info.dim {
                return Err(Error::input(format!("{who}: a vector of {} numbers, the configuration says dim {}", v.len(), info.dim)));
            }
            unit(v, who)
        })
        .collect()
}

// ---- a local program -----------------------------------------------------------------------------------------------------

struct CommandProvider {
    info: EmbedderInfo,
    argv: Vec<String>,
    /// the program runs in the workspace directory, so that a relative path in `command` is relative to the workspace
    cwd: std::path::PathBuf,
    timeout: Duration,
    batch: usize,
}

/// Runs a program with `input` on its standard input and returns its standard output; kills it when the time is up.
fn run_program(argv: &[String], cwd: Option<&std::path::Path>, input: Vec<u8>, timeout: Duration) -> Result<Vec<u8>> {
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(c) = cwd {
        cmd.current_dir(c);
    }
    let mut child = cmd.spawn().map_err(|e| Error::input(format!("cannot start `{}`: {e}", argv[0])))?;
    let mut stdin = child.stdin.take().expect("piped");
    let mut stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let out_t = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = stdout.read_to_end(&mut b);
        b
    });
    let err_t = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = stderr.read_to_end(&mut b);
        b
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Error::failed(format!("`{}` did not answer within {} s: stopped", argv[0], timeout.as_secs())));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => return Err(Error::failed(format!("`{}`: {e}", argv[0]))),
        }
    };
    let _ = writer.join();
    let out = out_t.join().unwrap_or_default();
    let err = err_t.join().unwrap_or_default();
    if !status.success() {
        return Err(Error::failed(format!("`{}` failed ({status}): {}", argv[0], String::from_utf8_lossy(&err).trim().lines().last().unwrap_or(""))));
    }
    Ok(out)
}

fn vectors_of(v: &Value, key: &str) -> Option<Vec<Vec<f32>>> {
    v.get(key)?.as_array()?.iter().map(|row| row.as_array()?.iter().map(|x| x.as_f64().map(|f| f as f32)).collect()).collect()
}

impl Embedder for CommandProvider {
    fn info(&self) -> &EmbedderInfo {
        &self.info
    }
    fn batch(&self) -> usize {
        self.batch
    }
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let who = format!("the embedding program `{}`", self.argv[0]);
        let out = run_program(&self.argv, Some(&self.cwd), serde_json::to_vec(&json!({"texts": texts})).expect("serialises"), self.timeout)?;
        let v: Value = serde_json::from_slice(&out).map_err(|e| Error::input(format!("{who}: its answer is not JSON: {e}")))?;
        // what the program says it is must be what the configuration says: another model is another index
        for (field, want) in [("model", self.info.model.as_str()), ("version", self.info.version.as_str())] {
            if let Some(got) = v.get(field).and_then(Value::as_str) {
                if got != want {
                    return Err(Error::input(format!("{who}: it reports {field} '{got}', the configuration says '{want}'")));
                }
            }
        }
        if let Some(d) = v.get("dim").and_then(Value::as_u64) {
            if d as usize != self.info.dim {
                return Err(Error::input(format!("{who}: it reports dim {d}, the configuration says {}", self.info.dim)));
            }
        }
        let vectors = vectors_of(&v, "vectors").ok_or_else(|| Error::input(format!("{who}: its answer has no `vectors` (a list of lists of numbers)")))?;
        check_vectors(&self.info, texts.len(), vectors, &who)
    }
}

// ---- a service over http(s) ----------------------------------------------------------------------------------------------

struct Http {
    info: EmbedderInfo,
    url: String,
    key_env: Option<String>,
    timeout: u64,
    batch: usize,
}

static TMP_N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

impl Embedder for Http {
    fn info(&self) -> &EmbedderInfo {
        &self.info
    }
    fn batch(&self) -> usize {
        self.batch
    }
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let who = format!("the embedding service {}", self.info.host.clone().unwrap_or_default());
        let body = serde_json::to_vec(&json!({"model": self.info.model, "input": texts})).expect("serialises");
        // the body and the key go through files in a private directory, not through the argument list (which other users can read)
        let dir = std::env::temp_dir().join(format!("hcp-manager-embed-{}-{}", std::process::id(), TMP_N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)));
        std::fs::create_dir_all(&dir).map_err(|e| Error::output(format!("{}: {e}", dir.display())))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
        let result = (|| {
            let body_file = dir.join("body.json");
            std::fs::write(&body_file, &body).map_err(|e| Error::output(format!("{}: {e}", body_file.display())))?;
            let mut args: Vec<String> = ["--fail", "--silent", "--show-error", "--max-time"].iter().map(|s| s.to_string()).collect();
            args.push(self.timeout.to_string());
            if self.info.host.as_deref().is_some_and(is_loopback) {
                args.extend(["--noproxy".into(), "*".into()]);
            }
            args.extend(["-X".into(), "POST".into(), "-H".into(), "Content-Type: application/json".into()]);
            if let Some(env) = &self.key_env {
                let key = std::env::var(env).map_err(|_| Error::input(format!("{who}: the environment variable {env} (api_key_env) is not set")))?;
                let hdr = dir.join("headers.txt");
                std::fs::write(&hdr, format!("Authorization: Bearer {key}\n")).map_err(|e| Error::output(format!("{}: {e}", hdr.display())))?;
                args.extend(["-H".into(), format!("@{}", hdr.display())]);
            }
            args.extend(["--data-binary".into(), format!("@{}", body_file.display()), self.url.clone()]);
            let out = Command::new("curl").args(&args).output().map_err(|e| Error::failed(format!("cannot run curl: {e}")))?;
            if !out.status.success() {
                return Err(Error::failed(format!("{who}: {}", String::from_utf8_lossy(&out.stderr).trim())));
            }
            let v: Value = serde_json::from_slice(&out.stdout).map_err(|e| Error::input(format!("{who}: the answer is not JSON: {e}")))?;
            if let Some(data) = v.get("data").and_then(Value::as_array) {
                let mut rows: Vec<(u64, Vec<f32>)> = Vec::new();
                for (i, d) in data.iter().enumerate() {
                    let e = d.get("embedding").and_then(Value::as_array).ok_or_else(|| Error::input(format!("{who}: an item of `data` has no `embedding`")))?;
                    rows.push((d.get("index").and_then(Value::as_u64).unwrap_or(i as u64), e.iter().map(|x| x.as_f64().unwrap_or(f64::NAN) as f32).collect()));
                }
                rows.sort_by_key(|r| r.0);
                return Ok(rows.into_iter().map(|r| r.1).collect::<Vec<_>>());
            }
            vectors_of(&v, "embeddings").ok_or_else(|| Error::input(format!("{who}: the answer has neither `data` nor `embeddings`")))
        })();
        let _ = std::fs::remove_dir_all(&dir);
        check_vectors(&self.info, texts.len(), result?, &who)
    }
}

// ---- an OpenAI-compatible service -----------------------------------------------------------------------------------------

struct OpenAi {
    info: EmbedderInfo,
    endpoint: String,
    key_env: Option<String>,
    key_cmd: Option<Vec<String>>,
    cwd: std::path::PathBuf,
    timeout: u64,
    batch: usize,
    retries: u32,
    backoff_ms: u64,
    send_dimensions: bool,
}

struct Reply {
    status: u16,
    body: Vec<u8>,
    retry_after_s: Option<u64>,
}

enum Transport {
    Timeout,
    Refused(String),
    Other(String),
}

/// Anything of `secret` in `text` becomes `***`, and the text is cut: what a service says back is shown only in this form.
fn scrub(text: &str, secret: Option<&str>) -> String {
    let mut t = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some(s) = secret.filter(|s| s.len() >= 4) {
        t = t.replace(s, "***");
    }
    if t.chars().count() > 160 {
        t = t.chars().take(160).collect::<String>() + "...";
    }
    t
}

fn curl_post(url: &str, body: &[u8], key: Option<&str>, timeout_s: u64, loopback: bool) -> std::result::Result<Reply, Transport> {
    // the body and the key go through files in a private directory, not through the argument list (which other users can read)
    let dir = std::env::temp_dir().join(format!("hcp-manager-embed-{}-{}", std::process::id(), TMP_N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)));
    std::fs::create_dir_all(&dir).map_err(|e| Transport::Other(format!("{}: {e}", dir.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    let result = (|| {
        let (body_file, hdr, out_file, resp_hdr) = (dir.join("body.json"), dir.join("headers.txt"), dir.join("response.json"), dir.join("response-headers.txt"));
        std::fs::write(&body_file, body).map_err(|e| Transport::Other(e.to_string()))?;
        let mut headers = String::from("Content-Type: application/json\nAccept: application/json\n");
        if let Some(k) = key {
            headers.push_str(&format!("Authorization: Bearer {k}\n"));
        }
        std::fs::write(&hdr, headers).map_err(|e| Transport::Other(e.to_string()))?;
        let mut args: Vec<String> = vec!["--silent".into(), "--show-error".into(), "--max-time".into(), timeout_s.to_string()];
        if loopback {
            args.extend(["--noproxy".into(), "*".into()]);
        }
        args.extend(["-X".into(), "POST".into(), "-H".into(), format!("@{}", hdr.display()), "--data-binary".into(), format!("@{}", body_file.display())]);
        args.extend(["-o".into(), out_file.display().to_string(), "-D".into(), resp_hdr.display().to_string(), "-w".into(), "%{http_code}".into(), url.to_string()]);
        let out = Command::new("curl").args(&args).output().map_err(|e| Transport::Other(format!("cannot run curl: {e}")))?;
        if !out.status.success() {
            let msg = scrub(&String::from_utf8_lossy(&out.stderr), key);
            return Err(match out.status.code() {
                Some(28) => Transport::Timeout,
                Some(6 | 7) => Transport::Refused(msg),
                _ => Transport::Other(msg),
            });
        }
        let status: u16 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(0);
        let retry_after_s = std::fs::read_to_string(&resp_hdr).ok().and_then(|h| h.lines().rev().find_map(|l| l.to_ascii_lowercase().strip_prefix("retry-after:").and_then(|v| v.trim().parse::<u64>().ok())));
        Ok(Reply { status, body: std::fs::read(&out_file).unwrap_or_default(), retry_after_s })
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

impl OpenAi {
    fn who(&self) -> String {
        format!("the embedding service {}", self.info.host.clone().unwrap_or_default())
    }

    /// The API key: from an external command or from the environment; never from a file, never kept.
    fn key(&self) -> Result<Option<String>> {
        if let Some(cmd) = &self.key_cmd {
            let out = run_program(cmd, Some(&self.cwd), Vec::new(), Duration::from_secs(15)).map_err(|e| Error::input(format!("{}: the api_key_command could not give a key ({})", self.who(), scrub(&e.message, None))))?;
            let k = String::from_utf8_lossy(&out).trim().to_string();
            if k.is_empty() {
                return Err(Error::input(format!("{}: the api_key_command printed no key", self.who())));
            }
            return Ok(Some(k));
        }
        match &self.key_env {
            Some(env) => match std::env::var(env) {
                Ok(k) if !k.trim().is_empty() => Ok(Some(k.trim().to_string())),
                _ => Err(Error::input(format!("{}: the environment variable {env} (api_key_env) is not set", self.who()))),
            },
            None => Ok(None),
        }
    }

    /// One request, with the retries the configuration allows for a timeout, a 429 and a 5xx answer.
    fn request(&self, texts: &[String], key: Option<&str>) -> Result<Vec<Vec<f32>>> {
        let who = self.who();
        let mut body = json!({"model": self.info.model, "input": texts, "encoding_format": "float"});
        if self.send_dimensions {
            body["dimensions"] = json!(self.info.dim);
        }
        let body = serde_json::to_vec(&body).expect("serialises");
        let loopback = self.info.host.as_deref().is_some_and(is_loopback);
        let mut last = String::new();
        for attempt in 0..=self.retries {
            let reply = match curl_post(&self.endpoint, &body, key, self.timeout, loopback) {
                Ok(r) => r,
                Err(Transport::Timeout) => {
                    last = format!("no answer within {} s", self.timeout);
                    self.wait(attempt, None);
                    continue;
                }
                Err(Transport::Refused(m)) => return Err(Error::failed(format!("{who}: cannot be reached ({m})"))),
                Err(Transport::Other(m)) => return Err(Error::failed(format!("{who}: {m}"))),
            };
            let said = || {
                let v: Value = serde_json::from_slice(&reply.body).unwrap_or(Value::Null);
                let m = v["error"]["message"].as_str().or_else(|| v["error"].as_str()).or_else(|| v["message"].as_str()).map(str::to_string).unwrap_or_else(|| String::from_utf8_lossy(&reply.body).into_owned());
                scrub(&m, key)
            };
            match reply.status {
                200..=299 => return self.parse(&reply.body),
                401 | 403 => return Err(Error::input(format!("{who}: the credentials were refused (HTTP {}){}", reply.status, match (&self.key_env, &self.key_cmd) { (Some(e), _) => format!(": check the key in {e}"), (_, Some(_)) => ": check the key that api_key_command prints".to_string(), _ => ": the service wants an API key (api_key_env or api_key_command)".to_string() }))),
                404 => return Err(Error::input(format!("{who}: HTTP 404: there is no embeddings endpoint at {}: check base_url (with or without /v1) and embeddings_path", self.endpoint))),
                429 | 500..=599 => {
                    last = format!("HTTP {}{}", reply.status, if reply.status == 429 { " (rate limited)" } else { "" });
                    self.wait(attempt, reply.retry_after_s);
                }
                s => return Err(Error::input(format!("{who}: the request was rejected (HTTP {s}): {}{}", said(), if s == 400 || s == 413 || s == 422 { " (a smaller `batch` may help)" } else { "" }))),
            }
        }
        Err(Error::failed(format!("{who}: gave up after {} attempt(s): {last}", self.retries + 1)))
    }

    fn wait(&self, attempt: u32, retry_after: Option<u64>) {
        if attempt >= self.retries {
            return;
        }
        let ms = retry_after.map(|s| (s * 1000).min(30_000)).unwrap_or_else(|| self.backoff_ms.saturating_mul(1 << attempt.min(10)));
        std::thread::sleep(Duration::from_millis(ms));
    }

    fn parse(&self, body: &[u8]) -> Result<Vec<Vec<f32>>> {
        let who = self.who();
        let v: Value = serde_json::from_slice(body).map_err(|_| Error::input(format!("{who}: the answer is not JSON")))?;
        let data = v.get("data").and_then(Value::as_array).ok_or_else(|| Error::input(format!("{who}: the answer has no `data` list (an OpenAI-compatible embeddings answer is {{\"data\": [{{\"index\", \"embedding\"}}]}})")))?;
        let mut rows: Vec<(usize, Vec<f32>)> = Vec::with_capacity(data.len());
        for (i, d) in data.iter().enumerate() {
            let e = d.get("embedding").and_then(Value::as_array).ok_or_else(|| Error::input(format!("{who}: an item of `data` has no `embedding` list of numbers")))?;
            let vec: Vec<f32> = e.iter().map(|x| x.as_f64().map(|f| f as f32).unwrap_or(f32::NAN)).collect();
            let index = match d.get("index") {
                Some(ix) => ix.as_u64().ok_or_else(|| Error::input(format!("{who}: an `index` that is not a number")))? as usize,
                None => i,
            };
            rows.push((index, vec));
        }
        // the answers come in any order; `index` says which text each belongs to
        rows.sort_by_key(|r| r.0);
        for (pos, (index, _)) in rows.iter().enumerate() {
            if *index != pos {
                return Err(Error::input(format!("{who}: the `index` values of the answer are not 0..{} once each (found {index} at place {pos})", rows.len().saturating_sub(1))));
            }
        }
        if let Some(bad) = rows.iter().find(|r| r.1.len() != self.info.dim) {
            return Err(Error::input(format!("{who}: it returned vectors of {} numbers, the profile says dim {}: the model, or its `dimensions`, changed; set `dim` to what the service returns and run `index build` (vectors of another size are kept apart, never mixed)", bad.1.len(), self.info.dim)));
        }
        Ok(rows.into_iter().map(|r| r.1).collect())
    }
}

impl Embedder for OpenAi {
    fn info(&self) -> &EmbedderInfo {
        &self.info
    }
    fn batch(&self) -> usize {
        self.batch
    }
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }
        let key = self.key()?;
        let mut all = Vec::with_capacity(texts.len());
        for part in texts.chunks(self.batch) {
            let got = self.request(part, key.as_deref())?;
            if got.len() != part.len() {
                return Err(Error::input(format!("{}: {} texts were sent, {} vectors came back", self.who(), part.len(), got.len())));
            }
            all.extend(got);
        }
        check_vectors(&self.info, texts.len(), all, &self.who())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hashing_embedding_is_deterministic_unit_length_and_lexical() {
        let a = hash_embedding("IO0 is sampled at reset", 384);
        assert_eq!(a, hash_embedding("IO0 is sampled at reset", 384), "the same text gives the same vector");
        assert_eq!(a.len(), 384);
        assert!((cosine(&a, &a) - 1.0).abs() < 1e-5);
        let near = hash_embedding("the IO0 pin is sampled at reset time", 384);
        let far = hash_embedding("Soldering the pad to the ground of the board", 384);
        assert!(cosine(&a, &near) > cosine(&a, &far) + 0.2, "{} vs {}", cosine(&a, &near), cosine(&a, &far));
        // parts of words count: a variant spelling is closer than an unrelated word
        let v1 = hash_embedding("decoupling capacitor", 384);
        let v2 = hash_embedding("decoupled capacitors", 384);
        let v3 = hash_embedding("antenna keepout", 384);
        assert!(cosine(&v1, &v2) > cosine(&v1, &v3) + 0.2);
        assert!(hash_embedding("", 384).iter().all(|x| *x == 0.0), "no text, no vector");
        // CJK characters are features too
        assert!(cosine(&hash_embedding("天线禁布区", 384), &hash_embedding("天线 禁布区 尺寸", 384)) > 0.3);
    }

    #[test]
    fn an_openai_base_url_is_normalised_whatever_prefix_it_has() {
        let e = |b: &str, p: Option<&str>| openai_endpoint(b, p).unwrap();
        assert_eq!(e("https://h", None), "https://h/v1/embeddings");
        assert_eq!(e("https://h/", None), "https://h/v1/embeddings");
        assert_eq!(e("https://h/v1", None), "https://h/v1/embeddings");
        assert_eq!(e("https://h/v1/", None), "https://h/v1/embeddings");
        assert_eq!(e("https://h/v1/embeddings", None), "https://h/v1/embeddings");
        assert_eq!(e("https://h:8443/api/v4", None), "https://h:8443/api/v4/embeddings");
        assert_eq!(e("https://h/openai/v1beta", None), "https://h/openai/v1beta/embeddings");
        assert_eq!(e("https://h/prefix", None), "https://h/prefix/v1/embeddings", "no version in the prefix: the usual one is added");
        assert_eq!(e("https://h/ignored", Some("/x/embed")), "https://h/x/embed");
        assert_eq!(e("http://[::1]:9/v1", None), "http://[::1]:9/v1/embeddings");
        for bad in ["h/v1", "ftp://h", "https://", "https://u:p@h/v1", "https://h/v1?x=1", "https://h/v1#f"] {
            assert!(openai_endpoint(bad, None).is_err(), "{bad}");
        }
        // credentials are not echoed back
        assert!(!openai_endpoint("https://user:secret@h/v1", None).unwrap_err().message.contains("secret"));
    }

    #[test]
    fn what_a_service_says_is_shown_cut_and_without_the_key() {
        assert_eq!(scrub("bad key sk-abcdef123456 rejected", Some("sk-abcdef123456")), "bad key *** rejected");
        assert_eq!(scrub("a
 b	 c", None), "a b c");
        assert!(scrub(&"x".repeat(1000), None).chars().count() <= 163);
        assert_eq!(scrub("abc", Some("ab")), "abc", "a secret too short to be a key is not blanked out of ordinary words");
    }

    #[test]
    fn the_configuration_decides_what_is_remote() {
        let mut c = EmbeddingConfig::local_default();
        assert!(!describe(&c).unwrap().remote);
        assert!(describe(&c).unwrap().key.starts_with("hashing:hash-ngram:1:384:"));
        c.provider = "http".into();
        c.model = Some("m".into());
        c.dim = Some(8);
        c.url = Some("https://api.example.com/v1/embeddings".into());
        assert!(describe(&c).unwrap().remote);
        for local in ["http://127.0.0.1:8080/e", "http://localhost/e", "http://[::1]:9/e", "http://user@localhost:11434/api"] {
            c.url = Some(local.into());
            assert!(!describe(&c).unwrap().remote, "{local}");
        }
        // a remote service is refused unless the configuration says so and the caller authorises it
        c.url = Some("https://api.example.com/v1/embeddings".into());
        let here = std::path::Path::new(".");
        assert!(make(&c, true, here).err().unwrap().message.contains("allow_remote"));
        c.allow_remote = true;
        assert!(make(&c, false, here).err().unwrap().message.contains("--authorize-remote"));
        assert!(make(&c, true, here).is_ok());
        // the identity of the vectors follows what makes them different
        let k1 = describe(&c).unwrap().key;
        c.model = Some("other".into());
        assert_ne!(k1, describe(&c).unwrap().key);
        c.allow_remote = false;
        c.model = Some("m".into());
        assert_eq!(k1, describe(&c).unwrap().key, "authorisation is not part of the identity");
    }
}
