//! Embedding providers: how the text of a block becomes a vector. The vector layer is for finding candidate passages; nothing
//! downstream reads a vector or a similarity (a pack, an IR and every verdict are made without them).
//!
//! Three providers, chosen in `<workspace>/embedding.yaml` (default: the first, with no file at all):
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
        EmbeddingConfig { embedding_version: one_zero(), provider: "hashing".into(), model: None, version: None, dim: None, command: None, url: None, api_key_env: None, allow_remote: false, batch: None, timeout_s: None, vector_weight: None }
    }
}

/// The configuration of the workspace: `embedding.yaml`, or the local default when there is none.
pub fn load_config(ws: &Workspace) -> Result<EmbeddingConfig> {
    let p = ws.embedding_config_path();
    if !p.exists() {
        return Ok(EmbeddingConfig::local_default());
    }
    let text = std::fs::read_to_string(&p).map_err(|e| Error::input(format!("{}: {e}", p.display())))?;
    let y: serde_yaml::Value = serde_yaml::from_str(&text).map_err(|e| Error::input(format!("{}: not valid YAML: {e}", p.display())))?;
    let v: Value = serde_json::to_value(&y).map_err(|e| Error::input(format!("{}: {e}", p.display())))?;
    let schema: Value = serde_json::from_str(CONFIG_SCHEMA_TEXT).expect("the embedded schema is valid JSON");
    let errors = fr_constraint::jsonschema::validate(&schema, &v);
    if let Some(e) = errors.first() {
        return Err(Error::input(format!("{}: not a valid embedding configuration: {}: {} ({} problem(s))", p.display(), e.path, e.message, errors.len())));
    }
    serde_json::from_value(v).map_err(|e| Error::input(format!("{}: {e}", p.display())))
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

pub fn describe(cfg: &EmbeddingConfig) -> Result<EmbedderInfo> {
    let need = |v: &Option<String>, what: &str| v.clone().filter(|s| !s.is_empty()).ok_or_else(|| Error::input(format!("embedding.yaml: provider {} needs `{what}`", cfg.provider)));
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
        other => return Err(Error::input(format!("embedding.yaml: unknown provider '{other}' (hashing, command, http)"))),
    };
    let ident = json!({"provider": cfg.provider, "model": model, "version": version, "dim": dim, "command": cfg.command, "url": cfg.url});
    let config_sha256 = sha256_hex(fr_constraint::hcp::canonical_json(&ident).as_bytes());
    let key = format!("{}:{model}:{version}:{dim}:{}", cfg.provider, &config_sha256[..8]);
    Ok(EmbedderInfo { provider: cfg.provider.clone(), model, version, dim, remote, config_sha256, key, host })
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
