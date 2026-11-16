//! The OpenAI-compatible embedding provider against a mock of the protocol (a real HTTP service on this machine that records every request):
//! the shape of the request and of the answer, `base_url` with and without `/v1`, out-of-order `index`, empty and malformed answers, changed
//! dimensions, rate limits, errors and timeouts, batches, the key (never in a file, an error or an output), the three authorisations for a
//! remote service, model and profile switching, and the keyword fallback when the service is not there.
//!
//! "The OpenAI protocol works" here means: this program speaks the documented request and answer shapes to this mock. No real service was
//! used; nothing in this file says anything about the behaviour of OpenAI or of any other provider's service.

mod common;
use common::*;
use serde_json::{json, Value};

const KEY: &str = "sk-TESTSECRET-0123456789abcdef";

fn cfg(port: u16, model: &str, extra: &str) -> String {
    format!("provider: openai\nbase_url: http://127.0.0.1:{port}/v1\nmodel: {model}\ndim: 8\nretry_backoff_ms: 5\n{extra}")
}

fn skip() -> bool {
    if !curl_here() {
        eprintln!("curl not found: skipped");
        return true;
    }
    false
}

fn embedding_test(ws: &std::path::Path, env: &[(&str, &str)]) -> std::process::Output {
    run_env(ws, &["--json", "embedding", "test"], env)
}

#[test]
fn base_url_with_or_without_the_version_prefix_reaches_the_same_endpoint() {
    if skip() {
        return;
    }
    let f = fixture("o-paths");
    let mock = Mock::start("127.0.0.1", fake_service(false));
    let p = mock.port;
    for (base, path_override, want) in [
        (format!("http://127.0.0.1:{p}"), None, "/v1/embeddings"),
        (format!("http://127.0.0.1:{p}/"), None, "/v1/embeddings"),
        (format!("http://127.0.0.1:{p}/v1"), None, "/v1/embeddings"),
        (format!("http://127.0.0.1:{p}/v1/"), None, "/v1/embeddings"),
        (format!("http://127.0.0.1:{p}/v1/embeddings"), None, "/v1/embeddings"),
        (format!("http://127.0.0.1:{p}/api/paas/v4"), None, "/api/paas/v4/embeddings"),
        (format!("http://127.0.0.1:{p}/openai/v1beta"), None, "/openai/v1beta/embeddings"),
        (format!("http://127.0.0.1:{p}/proxy"), None, "/proxy/v1/embeddings"),
        (format!("http://127.0.0.1:{p}/ignored"), Some("/custom/embed"), "/custom/embed"),
    ] {
        let before = mock.count();
        let extra = path_override.map(|o| format!("embeddings_path: {o}\n")).unwrap_or_default();
        write_embedding(&f.ws, &format!("provider: openai\nbase_url: {base}\nmodel: m8\ndim: 8\n{extra}"));
        let shown = jsn(&run(&f.ws, &["--json", "embedding", "show"]));
        assert_eq!(shown["embedder"]["endpoint"], format!("http://127.0.0.1:{p}{want}"), "{base}");
        let o = embedding_test(&f.ws, &[]);
        assert_eq!(code(&o), 0, "{base}: {}", err(&o));
        let r = mock.all();
        assert!(r.len() > before && r[before..].iter().all(|x| x.path == want && x.method == "POST"), "{base} -> {want}: {:?}", r[before..].iter().map(|x| &x.path).collect::<Vec<_>>());
    }
    // credentials in the address are refused, and so is a query string
    for bad in ["http://user:pw@127.0.0.1:1/v1", "http://127.0.0.1:1/v1?key=x"] {
        write_embedding(&f.ws, &format!("provider: openai\nbase_url: {bad}\nmodel: m8\ndim: 8\n"));
        let o = run(&f.ws, &["--json", "embedding", "show"]);
        assert_eq!(code(&o), 3, "{bad}");
        assert!(!err(&o).contains("pw"), "the password is not echoed: {}", err(&o));
    }
}

#[test]
fn the_request_and_the_answer_follow_the_openai_shapes() {
    if skip() {
        return;
    }
    let f = fixture("o-shapes");
    // the service answers in reverse order, each item with its index
    let mock = Mock::start("127.0.0.1", fake_service(true));
    write_embedding(&f.ws, &cfg(mock.port, "m8", "api_key_env: HCP_TEST_KEY\n"));
    let o = embedding_test(&f.ws, &[("HCP_TEST_KEY", KEY)]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    let req = &mock.all()[0];
    assert_eq!(req.headers["authorization"], format!("Bearer {KEY}"));
    assert_eq!(req.headers["content-type"], "application/json");
    assert_eq!(req.body["model"], "m8");
    assert_eq!(req.body["encoding_format"], "float");
    assert_eq!(req.body["input"].as_array().unwrap().len(), 3);
    assert!(req.body.get("dimensions").is_none(), "dimensions are only sent when asked for");
    // the vectors were put back in the order of the texts: the similarity is what the fake vectors give
    let texts = req.inputs();
    let unit = |t: &str| {
        let v = fake_vector(t, 8);
        let n = v.iter().map(|x| x * x).sum::<f64>().sqrt();
        v.iter().map(|x| x / n).collect::<Vec<f64>>()
    };
    let cos = |a: &[f64], b: &[f64]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f64>();
    let want = (cos(&unit(&texts[0]), &unit(&texts[1])), cos(&unit(&texts[0]), &unit(&texts[2])));
    let got = jsn(&o);
    assert!((got["similarity"]["related_sentences"].as_f64().unwrap() - want.0).abs() < 1e-4, "{got} vs {want:?}");
    assert!((got["similarity"]["unrelated_sentences"].as_f64().unwrap() - want.1).abs() < 1e-4, "{got} vs {want:?}");
    // `dimensions` on request
    write_embedding(&f.ws, &cfg(mock.port, "m8", "send_dimensions: true\n"));
    assert_eq!(code(&embedding_test(&f.ws, &[])), 0);
    assert_eq!(mock.all().last().unwrap().body["dimensions"], 8);
    // and a service that does not need a key gets no Authorization header
    assert!(!mock.all().last().unwrap().headers.contains_key("authorization"));
}

#[test]
fn malformed_answers_are_errors_with_a_reason() {
    if skip() {
        return;
    }
    let f = fixture("o-bad-answers");
    let cases: Vec<(&str, Handler2)> = vec![
        ("empty data", Box::new(|_| json!({"data": []}))),
        ("no data", Box::new(|_| json!({"object": "list"}))),
        ("duplicate index", Box::new(|r| json!({"data": r.inputs().iter().map(|t| json!({"index": 0, "embedding": fake_vector(t, 8)})).collect::<Vec<_>>()}))),
        ("index out of range", Box::new(|r| json!({"data": r.inputs().iter().enumerate().map(|(i, t)| json!({"index": i + 5, "embedding": fake_vector(t, 8)})).collect::<Vec<_>>()}))),
        ("no embedding", Box::new(|r| json!({"data": r.inputs().iter().enumerate().map(|(i, _)| json!({"index": i})).collect::<Vec<_>>()}))),
        ("wrong dimension", Box::new(|r| json!({"data": r.inputs().iter().enumerate().map(|(i, t)| json!({"index": i, "embedding": fake_vector(t, 16)})).collect::<Vec<_>>()}))),
        ("not a number", Box::new(|r| json!({"data": r.inputs().iter().enumerate().map(|(i, _)| json!({"index": i, "embedding": ["a", "b"]})).collect::<Vec<_>>()}))),
    ];
    for (name, make) in cases {
        let mock = Mock::start("127.0.0.1", std::sync::Arc::new(move |r: &Request, _| Response::json(200, make(r))));
        write_embedding(&f.ws, &cfg(mock.port, "m8", ""));
        let o = embedding_test(&f.ws, &[]);
        assert_eq!(code(&o), 3, "{name}: {}{}", out(&o), err(&o));
        let e = err(&o);
        match name {
            "wrong dimension" => assert!(e.contains("16 numbers") && e.contains("dim 8") && e.contains("never mixed"), "{e}"),
            "empty data" => assert!(e.contains("vectors came back") || e.contains("texts were sent"), "{e}"),
            "duplicate index" | "index out of range" => assert!(e.contains("`index`"), "{e}"),
            _ => assert!(e.contains("embedding") || e.contains("data") || e.contains("vector"), "{name}: {e}"),
        }
    }
    // not JSON at all
    let mock = Mock::start("127.0.0.1", std::sync::Arc::new(|_: &Request, _| Response { status: 200, headers: vec![], body: "<html>hello</html>".into(), delay_ms: 0 }));
    write_embedding(&f.ws, &cfg(mock.port, "m8", ""));
    let o = embedding_test(&f.ws, &[]);
    assert_eq!(code(&o), 3);
    assert!(err(&o).contains("not JSON") && !err(&o).contains("hello"), "{}", err(&o));
}

type Handler2 = Box<dyn Fn(&Request) -> Value + Send + Sync>;

#[test]
fn errors_limits_and_timeouts_are_handled_and_retried_a_limited_number_of_times() {
    if skip() {
        return;
    }
    let f = fixture("o-errors");
    let run_with = |h: Handler, extra: &str| {
        let mock = Mock::start("127.0.0.1", h);
        write_embedding(&f.ws, &cfg(mock.port, "m8", extra));
        let o = embedding_test(&f.ws, &[("HCP_TEST_KEY", KEY)]);
        let n = mock.count();
        (o, n)
    };
    let ok = fake_service(false);
    // 401: not retried, the key is named only by where it comes from
    let (o, n) = run_with(std::sync::Arc::new(|_, _| Response::json(401, json!({"error": {"message": format!("Incorrect API key provided: {KEY}")}}))), "api_key_env: HCP_TEST_KEY\n");
    assert_eq!((code(&o), n), (3, 1), "{}", err(&o));
    assert!(err(&o).contains("credentials were refused") && err(&o).contains("HCP_TEST_KEY") && !err(&o).contains(KEY) && !out(&o).contains(KEY), "{}", err(&o));
    // 404: says what to check
    let (o, n) = run_with(std::sync::Arc::new(|_, _| Response::json(404, json!({"error": "no such route"}))), "");
    assert_eq!((code(&o), n), (3, 1));
    assert!(err(&o).contains("no embeddings endpoint") && err(&o).contains("base_url"), "{}", err(&o));
    // 400: the service's words are shown cut and without the key
    let long = "x".repeat(400);
    let (o, n) = run_with(std::sync::Arc::new(move |_, _| Response::json(400, json!({"error": {"message": format!("bad input {KEY} {long}")}}))), "api_key_env: HCP_TEST_KEY\n");
    assert_eq!((code(&o), n), (3, 1));
    let e = err(&o);
    assert!(e.contains("HTTP 400") && e.contains("***") && !e.contains(KEY) && e.len() < 400 && e.contains("`batch`"), "{e}");
    // 429 then success: retried, and it worked
    let ok2 = ok.clone();
    let (o, n) = run_with(std::sync::Arc::new(move |r, i| if i < 2 { Response { status: 429, headers: vec![("Retry-After".into(), "0".into())], body: "{}".into(), delay_ms: 0 } } else { ok2(r, i) }), "retries: 2\n");
    assert_eq!((code(&o), n), (0, 3), "{}", err(&o));
    // 429 for ever: a limited number of attempts, then the reason
    let (o, n) = run_with(std::sync::Arc::new(|_, _| Response::json(429, json!({"error": "slow down"}))), "retries: 2\n");
    assert_eq!((code(&o), n), (1, 3));
    assert!(err(&o).contains("gave up after 3 attempt(s)") && err(&o).contains("rate limited"), "{}", err(&o));
    // 503 then success
    let ok3 = ok.clone();
    let (o, n) = run_with(std::sync::Arc::new(move |r, i| if i == 0 { Response::json(503, json!({})) } else { ok3(r, i) }), "retries: 1\n");
    assert_eq!((code(&o), n), (0, 2), "{}", err(&o));
    // retries: 0 means one attempt
    let (o, n) = run_with(std::sync::Arc::new(|_, _| Response::json(500, json!({}))), "retries: 0\n");
    assert_eq!((code(&o), n), (1, 1));
    // a service that is too slow: the time limit, then one retry, then the reason
    let (o, n) = run_with(std::sync::Arc::new(|_, _| Response { status: 200, headers: vec![], body: "{}".into(), delay_ms: 2500 }), "timeout_s: 1\nretries: 1\n");
    // (the mock answers one request at a time and is still busy with the first when the program gives up, so only its message counts here)
    assert!(code(&o) == 1 && n >= 1, "{}", err(&o));
    assert!(err(&o).contains("no answer within 1 s") && err(&o).contains("2 attempt(s)"), "{}", err(&o));
    // nothing listens: said at once, no retries of a refused connection
    write_embedding(&f.ws, "provider: openai\nbase_url: http://127.0.0.1:1/v1\nmodel: m8\ndim: 8\n");
    let o = embedding_test(&f.ws, &[]);
    assert_eq!(code(&o), 1);
    assert!(err(&o).contains("cannot be reached"), "{}", err(&o));
    // the key variable is not set: said before anything is sent
    let mock = Mock::start("127.0.0.1", ok);
    write_embedding(&f.ws, &cfg(mock.port, "m8", "api_key_env: HCP_TEST_KEY_NOT_SET\n"));
    let o = embedding_test(&f.ws, &[]);
    assert_eq!(code(&o), 3);
    assert!(err(&o).contains("HCP_TEST_KEY_NOT_SET") && mock.count() == 0, "{}", err(&o));
}

#[test]
fn texts_go_in_batches_of_the_configured_size() {
    if skip() {
        return;
    }
    let f = fixture("o-batch");
    let mock = Mock::start("127.0.0.1", fake_service(false));
    write_embedding(&f.ws, &cfg(mock.port, "m8", "batch: 2\n"));
    let o = run(&f.ws, &["--json", "index", "build"]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    let st = jsn(&run(&f.ws, &["--json", "index", "status"]));
    let blocks: usize = st["documents"].as_array().unwrap().iter().map(|d| d["chunks"].as_u64().unwrap() as usize).sum();
    let sizes: Vec<usize> = mock.all().iter().map(|r| r.inputs().len()).collect();
    assert!(blocks > 4 && sizes.iter().all(|s| *s <= 2) && sizes.iter().sum::<usize>() == blocks, "{blocks} blocks in batches of {sizes:?}");
    assert_eq!(st["vector_index"], "ready");
}

#[test]
fn the_key_is_in_no_output_no_file_and_no_error() {
    if skip() {
        return;
    }
    let f = fixture("o-key");
    // the service echoes the key in an error: it is hidden
    let mock = Mock::start("127.0.0.1", fake_service(false));
    write_embedding(&f.ws, &cfg(mock.port, "m8", "api_key_env: HCP_TEST_KEY\n"));
    let env = [("HCP_TEST_KEY", KEY)];
    let mut outputs = vec![];
    for args in [vec!["--json", "embedding", "show"], vec!["embedding", "show"], vec!["--json", "embedding", "test"], vec!["--json", "index", "build"], vec!["--json", "index", "status"]] {
        let o = run_env(&f.ws, &args, &env);
        assert_eq!(code(&o), 0, "{args:?}: {}", err(&o));
        outputs.push(format!("{}{}", out(&o), err(&o)));
    }
    let s = run_env(&f.ws, &["--json", "search", "antenna keep-out", "--device", "AC-100", "--mode", "hybrid"], &env);
    assert_eq!(code(&s), 0);
    outputs.push(format!("{}{}", out(&s), err(&s)));
    assert!(outputs.iter().all(|o| !o.contains(KEY)), "the key appeared in an output");
    assert!(mock.all().iter().all(|r| r.headers["authorization"] == format!("Bearer {KEY}")), "and it was sent, as a bearer token, to the service");
    // not in a file of the workspace (the configuration holds only the NAME of the variable)
    for (p, b) in all_files(&f.ws) {
        assert!(!contains(&b, KEY), "the key is in {}", p.display());
    }
    // an external command that prints the key: the program name is shown, its arguments are not
    write_embedding(&f.ws, &cfg(mock.port, "m8", "api_key_command: [\"sh\", \"-c\", \"printf %s sk-FROM-COMMAND-999\"]\n"));
    let shown = out(&run(&f.ws, &["--json", "embedding", "show"]));
    assert!(!shown.contains("sk-FROM-COMMAND-999") && shown.contains("\"sh\""), "{shown}");
    assert_eq!(code(&embedding_test(&f.ws, &[])), 0);
    assert_eq!(mock.all().last().unwrap().headers["authorization"], "Bearer sk-FROM-COMMAND-999");
    // a command that fails or prints nothing
    write_embedding(&f.ws, &cfg(mock.port, "m8", "api_key_command: [\"sh\", \"-c\", \"exit 3\"]\n"));
    let o = embedding_test(&f.ws, &[]);
    assert_eq!(code(&o), 3);
    assert!(err(&o).contains("api_key_command"), "{}", err(&o));
    write_embedding(&f.ws, &cfg(mock.port, "m8", "api_key_env: A_B\napi_key_command: [\"true\"]\n"));
    assert_eq!(code(&run(&f.ws, &["embedding", "show"])), 3, "both sources of a key is a mistake");
}

#[test]
fn a_remote_service_receives_nothing_unless_all_three_authorisations_are_given() {
    if skip() {
        return;
    }
    let Some(ip) = lan_address() else {
        eprintln!("this machine has no address other than the loopback one: skipped");
        return;
    };
    let f = fixture("o-remote");
    let mock = Mock::start("0.0.0.0", fake_service(false));
    let conf = |allow: bool| format!("provider: openai\nbase_url: http://{ip}:{}/v1\nmodel: m8\ndim: 8\nallow_remote: {allow}\n", mock.port);
    write_embedding(&f.ws, &conf(false));
    let shown = jsn(&run(&f.ws, &["--json", "embedding", "show"]));
    assert_eq!(shown["text_leaves_this_machine"], json!(true), "{shown}");
    // 1. the configuration does not allow it
    let o = run(&f.ws, &["--json", "index", "build", "--authorize-remote"]);
    assert_eq!(code(&o), 5);
    assert!(jsn(&o)["embedder_problem"].as_str().unwrap().contains("allow_remote"));
    assert_eq!(code(&run(&f.ws, &["embedding", "test", "--authorize-remote"])), 3);
    // 2. the configuration allows it, the command does not
    write_embedding(&f.ws, &conf(true));
    assert_eq!(code(&run(&f.ws, &["index", "build"])), 5);
    assert_eq!(code(&run(&f.ws, &["embedding", "test"])), 3);
    let s = jsn(&run(&f.ws, &["--json", "search", "antenna", "--device", "AC-100", "--mode", "hybrid"]));
    assert_eq!((s["mode_used"].as_str(), s["degraded"].as_str().is_some_and(|d| d.contains("remote"))), (Some("keyword"), true), "{s}");
    // 3. both, but no document is marked: nothing is embedded
    let o = run(&f.ws, &["--json", "index", "build", "--authorize-remote"]);
    assert_eq!(code(&o), 5);
    assert!(jsn(&o)["documents"].as_array().unwrap().iter().all(|d| d["vectors"]["state"] == "skipped"));
    assert_eq!(mock.count(), 0, "no request has been made so far");
    // a marked document is sent, an unmarked one is not
    let p = f.ws.join("sources.yaml");
    let mut y: serde_yaml::Value = serde_yaml::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
    for d in y["documents"].as_sequence_mut().unwrap() {
        if d["document_id"].as_str() == Some("DOC-AC-100-DS") {
            d["embed_remote"] = serde_yaml::Value::Bool(true);
        }
    }
    std::fs::write(&p, serde_yaml::to_string(&y).unwrap()).unwrap();
    let o = run(&f.ws, &["--json", "index", "build", "--authorize-remote"]);
    assert_eq!(code(&o), 5, "the other documents were skipped");
    let sent = mock.texts();
    assert!(!sent.is_empty() && sent.iter().all(|t| !t.contains("EN pin") && !t.contains("AC-100U")), "only the marked document: {sent:?}");
}

#[test]
fn another_model_or_dimension_is_a_new_generation_and_the_old_one_is_kept_until_pruned() {
    if skip() {
        return;
    }
    let f = fixture("o-generations");
    let mock = Mock::start("127.0.0.1", fake_service(false));
    let p = mock.port;
    // two named profiles; one is in force, a command can name the other
    write_embedding(
        &f.ws,
        &format!(
            "active_profile: small\nprofiles:\n  small: {{provider: openai, base_url: \"http://127.0.0.1:{p}\", model: m8, dim: 8, retry_backoff_ms: 5}}\n  large: {{provider: openai, base_url: \"http://127.0.0.1:{p}/v1\", model: m16, dim: 16, retry_backoff_ms: 5}}\n"
        ),
    );
    let shown = jsn(&run(&f.ws, &["--json", "embedding", "show"]));
    assert_eq!((shown["profile"].as_str(), shown["profiles"].as_array().unwrap().len()), (Some("small"), 2));
    assert_eq!(code(&run(&f.ws, &["index", "build"])), 0);
    let q = |extra: &[&str]| {
        let mut a = vec!["--json", "search", "antenna keep-out copper", "--device", "AC-100", "--mode", "vector", "--limit", "5"];
        a.extend_from_slice(extra);
        jsn(&run(&f.ws, &a))
    };
    let small_hits = q(&[]);
    assert!(small_hits["vector"]["embedder"].as_str().unwrap().starts_with("openai:m8:1:8:"), "{small_hits}");
    // the other profile has no vectors yet: its search says so, and does not use the vectors of the first (another size, another model)
    let o = run(&f.ws, &["--profile", "large", "search", "antenna", "--device", "AC-100", "--mode", "vector"]);
    assert_eq!(code(&o), 3, "{}", err(&o));
    let st = jsn(&run(&f.ws, &["--json", "--profile", "large", "index", "status"]));
    assert_eq!(st["vector_index"], "stale", "{st}");
    assert!(st["documents"][0]["vector"]["reason"].as_str().unwrap().contains("another embedding configuration"), "{st}");
    // building with it makes a second generation; the first is still there
    assert_eq!(code(&run(&f.ws, &["--profile", "large", "index", "build"])), 0);
    let st = jsn(&run(&f.ws, &["--json", "--profile", "large", "index", "status"]));
    let gens = st["embedder"]["generations"].as_array().unwrap();
    assert_eq!(gens.len(), 2, "{st}");
    assert_eq!(gens.iter().find(|g| g["in_force"] == true).unwrap()["dim"], 16);
    // each profile finds its own vectors, unchanged
    assert_eq!(q(&[])["hits"], small_hits["hits"], "the older generation was not touched by the build of the new one");
    assert!(q(&["--profile", "large"])["vector"]["embedder"].as_str().unwrap().starts_with("openai:m16:1:16:"));
    // pruning with `large` in force removes the small generation, and only then
    assert_eq!(jsn(&run(&f.ws, &["--json", "--profile", "large", "index", "prune"]))["removed_generations"], 1);
    assert_eq!(run(&f.ws, &["search", "antenna", "--device", "AC-100", "--mode", "vector"]).status.code(), Some(3), "the small generation is gone");
    // an unknown profile is an error, never a fallback
    let o = run(&f.ws, &["--profile", "nope", "embedding", "show"]);
    assert_eq!(code(&o), 3);
    assert!(err(&o).contains("no embedding profile 'nope'") && err(&o).contains("small"), "{}", err(&o));
    // several profiles and none named: an error that says so
    write_embedding(&f.ws, "profiles:\n  a: {provider: hashing}\n  b: {provider: hashing, dim: 64}\n");
    let o = run(&f.ws, &["embedding", "show"]);
    assert_eq!(code(&o), 3);
    assert!(err(&o).contains("several profiles"), "{}", err(&o));
    assert_eq!(code(&run(&f.ws, &["--profile", "b", "embedding", "show"])), 0);
    // one provider written flat and profiles together are a mistake
    write_embedding(&f.ws, "provider: hashing\nprofiles:\n  a: {provider: hashing}\n");
    assert_eq!(code(&run(&f.ws, &["embedding", "show"])), 3);
}

#[test]
fn a_service_that_is_not_there_leaves_keyword_search_and_the_deterministic_path_as_they_are() {
    let f = fixture("o-offline");
    write_embedding(&f.ws, "provider: openai\nbase_url: http://127.0.0.1:1/v1\nmodel: m8\ndim: 8\nretries: 0\n");
    let o = run(&f.ws, &["--json", "index", "build"]);
    assert_eq!(code(&o), 5, "the vectors could not be made: partial");
    let st = jsn(&run(&f.ws, &["--json", "index", "status"]));
    assert_eq!(st["keyword_index"], "ready", "{st}");
    let s = search(&f.ws, "antenna keep-out copper", &["--device", "AC-100", "--mode", "hybrid"]);
    assert_eq!(s["mode_used"], "keyword", "{s}");
    assert!(s["degraded"].is_string() && !s["hits"].as_array().unwrap().is_empty());
    assert_eq!(code(&run(&f.ws, &["search", "antenna", "--device", "AC-100", "--mode", "vector"])), 3);
    let k = search(&f.ws, "antenna keep-out copper", &["--device", "AC-100"]);
    assert_eq!(k["hits"], s["hits"], "keyword search is what a hybrid search falls back to");
}

#[test]
fn the_schema_of_the_configuration_is_enforced() {
    let f = fixture("o-schema");
    for (yaml, why) in [
        ("provider: openai\nmodel: m\ndim: 8\n", "base_url"),
        ("provider: openai\nbase_url: http://h\ndim: 8\n", "model"),
        ("provider: openai\nbase_url: http://h\nmodel: m\n", "dim"),
        ("provider: openai\nbase_url: ftp://h\nmodel: m\ndim: 8\n", "base_url"),
        ("provider: openai\nbase_url: http://h\nmodel: m\ndim: 8\nretries: 99\n", "retries"),
        ("provider: openai\nbase_url: http://h\nmodel: m\ndim: 8\napi_key: sk-literal\n", "unexpected property"),
        ("profiles:\n  a: {provider: openai, model: m}\n", "base_url"),
        ("store:\n  backend: cloud\n", "backend"),
    ] {
        write_embedding(&f.ws, yaml);
        let o = run(&f.ws, &["embedding", "show"]);
        assert_eq!(code(&o), 3, "{yaml}");
        assert!(err(&o).contains(why), "{yaml}: {}", err(&o));
        assert!(!err(&o).contains("sk-literal") || why == "unexpected property", "{}", err(&o));
    }
}

#[allow(dead_code)]
fn unused(_: Value) {}

#[test]
fn the_example_configuration_is_valid_and_every_profile_in_it_describes_itself() {
    let f = fixture("o-example");
    let example = std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("config.example.yaml")).unwrap();
    write_embedding(&f.ws, &example);
    let shown = jsn(&run(&f.ws, &["--json", "embedding", "show"]));
    assert_eq!((shown["profile"].as_str(), shown["profiles"].as_array().unwrap().len()), (Some("local"), 5), "{shown}");
    let mut remote = vec![];
    for p in ["local", "openai", "local-server", "other-vendor", "program"] {
        let s = jsn(&run(&f.ws, &["--profile", p, "--json", "embedding", "show"]));
        assert_eq!(s["profile"], p);
        if s["text_leaves_this_machine"] == json!(true) {
            remote.push(p);
            assert_eq!(s["remote_requires"].as_array().unwrap().len(), 3);
        }
    }
    assert_eq!(remote, ["openai", "other-vendor"], "the service on localhost is local, the other two are not");
    // the shipped example allows nothing remote: with it as it is, a build for the remote profile sends nothing and says why
    let o = run(&f.ws, &["--profile", "openai", "--json", "index", "build", "--authorize-remote"]);
    assert_eq!(code(&o), 5);
    assert!(jsn(&o)["embedder_problem"].as_str().unwrap().contains("allow_remote"), "{}", out(&o));
    let shown = jsn(&run(&f.ws, &["--profile", "other-vendor", "--json", "embedding", "show"]));
    assert_eq!(shown["embedder"]["endpoint"], "https://embeddings.example.com/api/paas/v4/embeddings");
    assert!(!shown.to_string().contains("embeddings/example"), "the arguments of api_key_command are not shown: {shown}");
}
