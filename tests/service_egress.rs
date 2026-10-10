//! No SPARQL text handed to this crate reaches the network (ledger #1085, the history in
//! ledger #1083 and #145).
//!
//! The claim this file answers: ikigai-nl "builds its own evaluator" and is therefore exposed
//! wherever `oxigraph/http-client` is unified on (rudof_rdf does it for every native host that
//! links ikigai-shacl, so `ikigai-cli`). It does not hold for this crate's code. ikigai-nl has no
//! `oxigraph` in its library graph at all (`cargo tree -e normal -i oxigraph` is empty), builds
//! no `SparqlEvaluator`, and evaluates nothing in-process: `src/script_sparql.rs` only PARSES
//! (spargebra). What does reach an evaluator is the dry run in `urn:nl:sparql:check` (and so in
//! every `urn:nl:sparql` attempt), a sub-request carrying caller- or model-supplied text to the
//! host's store at `urn:iki:store:graph-{select,ask,construct,describe}`. That door is the
//! store's, and before ikigai-store 0.2.10 it fetched a `SERVICE` in such a build: the
//! `the_store_door_the_dry_run_uses_refuses_service` test below is that reproduction, and it
//! fails on 0.2.7 under the probe feature. This crate never forwards one, because the analysis
//! (a copy of ikigai-script's) refuses `SERVICE` anywhere in the algebra and any update
//! (`LOAD` included) before the dry run is built, and binds the preview from the same parse.
//!
//! The file runs in BOTH builds. Under `--features http-client-probe` (CI's `features:` job)
//! oxigraph's HTTP client is compiled into the test graph, as it is in a host, and
//! `oxigraph_alone_reaches_the_stub` proves it is live, so "the stub saw nothing" is not
//! vacuous. In the default build the same refusals are pinned, so neither build regresses the
//! other.
//!
//! The stub is a plain TCP listener on 127.0.0.1 at an ephemeral port that answers any request
//! with a SPARQL JSON result or an N-Triples document, so a leak is a SUCCESS that brings data
//! in from the network, and it counts every connection. No real host is ever named.

mod common;

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use common::*;
use ikigai_core::{Capability, Error, Verb};
use ikigai_nl::{Check, Draft, SpaceConfig, Status};
use spargebra::SparqlParser;

const TITLE: &str = "http://purl.org/dc/terms/title";

/// A local HTTP stub that counts the connections it accepts.
struct Stub {
    base: String,
    hits: Arc<AtomicUsize>,
}

impl Stub {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&hits);
        // Detached: the thread blocks in `accept` and dies with the test process.
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                counter.fetch_add(1, Ordering::SeqCst);
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut request_line = String::new();
                let _ = reader.read_line(&mut request_line);
                let mut content_length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        content_length = v.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0u8; content_length];
                let _ = std::io::Read::read_exact(&mut reader, &mut body);
                let (media, payload) = if request_line.contains("/load") {
                    (
                        "application/n-triples",
                        "<urn:stub:s> <urn:stub:p> \"from-the-network\" .\n".to_string(),
                    )
                } else {
                    (
                        "application/sparql-results+json",
                        r#"{"head":{"vars":["s","p","o"]},"results":{"bindings":[{"s":{"type":"uri","value":"urn:stub:s"},"p":{"type":"uri","value":"urn:stub:p"},"o":{"type":"literal","value":"from-the-network"}}]}}"#
                            .to_string(),
                    )
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: {media}\r\nContent-Length: {}\r\n\
                     Connection: close\r\n\r\n{payload}",
                    payload.len()
                );
            }
        });
        Stub { base, hits }
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }
}

/// Queries that would fetch from `s`, each otherwise a draft the check would pass and dry-run:
/// it reads LEDGER, which the caller may read, through a predicate the grounding knows.
fn queries(s: &str) -> Vec<String> {
    let ledger = format!("GRAPH <{LEDGER}> {{ ?item <{TITLE}> ?title }}");
    vec![
        format!("SELECT * WHERE {{ {ledger} SERVICE <{s}/sparql> {{ ?s ?p ?o }} }}"),
        format!("ASK {{ {ledger} SERVICE <{s}/sparql> {{ ?s ?p ?o }} }}"),
        format!(
            "CONSTRUCT {{ ?item <{TITLE}> ?o }} WHERE {{ {ledger} SERVICE <{s}/sparql> {{ ?s ?p ?o }} }}"
        ),
        format!("DESCRIBE ?item FROM <{LEDGER}> WHERE {{ {ledger} SERVICE <{s}/sparql> {{ ?s ?p ?o }} }}"),
        format!("SELECT * WHERE {{ {ledger} SERVICE SILENT <{s}/sparql> {{ ?s ?p ?o }} }}"),
        // The service named by a variable, bound at evaluation time: no IRI in the pattern.
        format!(
            "SELECT * WHERE {{ {ledger} VALUES ?svc {{ <{s}/sparql> }} SERVICE ?svc {{ ?s ?p ?o }} }}"
        ),
        // Buried in an expression, where a walk of graph patterns alone would not look.
        format!(
            "SELECT * WHERE {{ {ledger} FILTER EXISTS {{ SERVICE <{s}/sparql> {{ ?s ?p ?o }} }} }}"
        ),
        // In a sub-select.
        format!(
            "SELECT * WHERE {{ {ledger} {{ SELECT ?s WHERE {{ SERVICE <{s}/sparql> {{ ?s ?p ?o }} }} }} }}"
        ),
    ]
}

/// Updates that would fetch from `s`. A draft must be a query, so every one is refused; these
/// pin that `LOAD` and an update's `WHERE` are among what is refused.
fn updates(s: &str) -> Vec<String> {
    vec![
        format!("LOAD <{s}/load>"),
        format!("LOAD SILENT <{s}/load> INTO GRAPH <{LEDGER}>"),
        format!(
            "INSERT {{ GRAPH <{LEDGER}> {{ ?s ?p ?o }} }} WHERE {{ SERVICE <{s}/sparql> {{ ?s ?p ?o }} }}"
        ),
    ]
}

/// May read LEDGER, nothing else.
fn reader() -> Capability {
    Capability::scoped([format!("urn:cap:store:read:graph:{LEDGER}")])
}

fn check(h: &Host, text: &str) -> Check {
    let rep = issue(
        &h.kernel,
        Verb::Source,
        "urn:nl:sparql:check",
        &[("content", text), ("as", "application/json")],
        &reader(),
    )
    .unwrap_or_else(|e| panic!("urn:nl:sparql:check: {e}"));
    serde_json::from_slice(&rep.bytes).expect("the JSON face parses into the model")
}

/// The control: with the probe feature, oxigraph's own evaluator in THIS test graph fetches.
/// Without it the other tests' "the stub saw nothing" would prove nothing about a host's build.
#[cfg(feature = "http-client-probe")]
#[test]
fn oxigraph_alone_reaches_the_stub() {
    use oxigraph::sparql::{QueryResults, SparqlEvaluator};
    use oxigraph::store::Store;

    let stub = Stub::start();
    let store = Store::new().unwrap();
    let query = SparqlParser::new()
        .parse_query(&format!(
            "SELECT * WHERE {{ SERVICE <{}/sparql> {{ ?s ?p ?o }} }}",
            stub.base
        ))
        .unwrap();
    // ⚠ A plain evaluator ON PURPOSE: this is the reproduction of what a host's build does,
    // never a pattern for library code.
    let results = SparqlEvaluator::new()
        .for_query(query)
        .on_store(&store)
        .execute()
        .expect("the SERVICE call is made");
    let QueryResults::Solutions(rows) = results else {
        panic!("a SELECT answers solutions")
    };
    let rows: Vec<_> = rows.collect::<Result<_, _>>().expect("the stub answered");
    assert_eq!(
        rows.len(),
        1,
        "the stub's one row came back from the network"
    );
    assert!(stub.hits() > 0, "the probe build has a live HTTP client");
}

#[test]
fn the_check_refuses_every_service_and_never_runs_it() {
    let stub = Stub::start();
    let h = drafting_host(SpaceConfig::new(), &[]);
    // The harness can reach a dry run at all: the same draft without SERVICE previews.
    let plain = check(
        &h,
        &format!("SELECT * WHERE {{ GRAPH <{LEDGER}> {{ ?item <{TITLE}> ?title }} }}"),
    );
    assert!(plain.valid && plain.preview.is_some(), "{plain:#?}");

    for text in queries(&stub.base) {
        let c = check(&h, &text);
        assert!(!c.valid, "{text}\n{c:#?}");
        assert!(c.preview.is_none(), "{text}: never dry-run\n{c:#?}");
        assert!(
            c.errors.iter().any(|e| e.contains("SERVICE")),
            "{text}: the refusal names SERVICE\n{c:#?}"
        );
    }
    for text in updates(&stub.base) {
        let c = check(&h, &text);
        assert!(!c.valid && c.preview.is_none(), "{text}\n{c:#?}");
        assert!(!c.errors.is_empty(), "{text}\n{c:#?}");
    }
    assert_eq!(stub.hits(), 0, "no check sent the stub anything");
}

#[test]
fn a_drafted_service_is_never_run_or_saved() {
    let stub = Stub::start();
    let drafts: Vec<String> = queries(&stub.base).into_iter().take(3).collect();
    let drafts: Vec<&str> = drafts.iter().map(String::as_str).collect();
    let h = drafting_host(SpaceConfig::new(), &[("federate", &drafts)]);
    let cap = Capability::scoped([
        format!("urn:cap:store:read:graph:{LEDGER}"),
        "urn:cap:script:read:public".to_string(),
        "urn:cap:script:write:federate".to_string(),
        "urn:cap:script:read:federate".to_string(),
    ]);
    let rep = issue(
        &h.kernel,
        Verb::Sink,
        "urn:nl:sparql",
        &[
            ("ask", "federate"),
            ("save", "federate"),
            ("as", "application/json"),
        ],
        &cap,
    )
    .unwrap_or_else(|e| panic!("urn:nl:sparql: {e}"));
    let d: Draft = serde_json::from_slice(&rep.bytes).unwrap();
    assert_eq!(d.status, Status::Failed, "{d:#?}");
    assert!(d
        .attempts
        .iter()
        .all(|a| !a.valid && a.errors.iter().any(|e| e.contains("SERVICE"))));
    assert!(
        h.saved("federate").is_none(),
        "a refused draft is never saved"
    );
    assert_eq!(stub.hits(), 0, "no attempt sent the stub anything");
}

/// Where the dry run goes: the store's graph-scoped door. This is the reproduction of the
/// exposure the claim points at, and it is the STORE's: on ikigai-store 0.2.7 under the probe
/// feature this door fetched (and answered with the stub's row). From 0.2.10 it refuses before
/// evaluation, typed, naming `query`, in every build.
#[test]
fn the_store_door_the_dry_run_uses_refuses_service() {
    let stub = Stub::start();
    let h = drafting_host(SpaceConfig::new(), &[]);
    for text in queries(&stub.base) {
        if text.starts_with("DESCRIBE") {
            continue; // the scoped door refuses its FROM before anything else
        }
        let form = text.split_whitespace().next().unwrap().to_ascii_lowercase();
        let door = format!("urn:iki:store:graph-{form}");
        let result = issue(
            &h.kernel,
            Verb::Source,
            &door,
            &[("query", &text), ("graph", LEDGER)],
            &reader(),
        );
        assert!(
            matches!(&result, Err(Error::InvalidArgument { name, .. }) if name == "query"),
            "{door} {text}: {result:?}"
        );
    }
    assert_eq!(stub.hits(), 0, "the store door sent the stub nothing");
}

/// ikigai-store's exported door checks are the oracle: everything they refuse, this crate's
/// analysis refuses too, as `InvalidArgument` on `content`. ikigai-nl evaluates nothing, so the
/// store's `evaluator()` has nothing to wrap here; what it can share is the definition of
/// "refused", and this pins that the copy of ikigai-script's analysis has not drifted below it.
#[test]
fn the_analysis_refuses_everything_the_stores_door_checks_refuse() {
    use ikigai_nl::script_sparql::{analyze, SparqlDoor};
    use ikigai_store::service::{refuse_load, refuse_service, refuse_service_in_update};

    let base = "http://127.0.0.1:9";
    let door = SparqlDoor::store();
    for text in queries(base) {
        let parsed = SparqlParser::new().parse_query(&text).unwrap();
        assert!(
            refuse_service(&parsed, "content").is_err(),
            "oracle: {text}"
        );
        let refused = analyze(&text, &door);
        assert!(
            matches!(&refused, Err(Error::InvalidArgument { name, detail })
                if name == "content" && detail.contains("SERVICE")),
            "{text}: {refused:?}"
        );
    }
    for text in updates(base) {
        let parsed = SparqlParser::new().parse_update(&text).unwrap();
        assert!(
            refuse_service_in_update(&parsed, "content").is_err()
                || refuse_load(&parsed, "content").is_err(),
            "oracle: {text}"
        );
        let refused = analyze(&text, &door);
        assert!(
            matches!(&refused, Err(Error::InvalidArgument { name, .. }) if name == "content"),
            "{text}: {refused:?}"
        );
    }
}
