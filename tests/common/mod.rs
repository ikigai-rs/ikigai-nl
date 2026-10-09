//! The test host: a kernel with everything the grounding composes.
//!
//! - **The store** (`ikigai-store`, in memory) with two named graphs, `LEDGER` and `BOOKS`,
//!   and a triple in its default graph that no scoped read can reach.
//! - **The vocabulary** (`ikigai_vocab::space()`), the real one.
//! - **A stand-in for `ikigai-script`**, because that crate is not published yet and a
//!   committed path or git override is not an option. It speaks the published JSON shapes
//!   field for field (`Catalog`, the script document) and enforces the same grants:
//!   `urn:cap:script:read:{name}`, or `urn:cap:script:read:public` for a public published
//!   script, and the catalog lists only what the caller may read. Its catalog is LIVE, as
//!   the real one is. ⚠ When `ikigai-script` publishes, replace this with the real crate
//!   as a dev-dependency: a stand-in is a second copy of a contract, and copies drift.
//!   It also takes a Sink (a draft or a publish), analyzing a SPARQL script with
//!   `ikigai_nl::script_sparql`, which is ikigai-script's own analysis copied byte for byte,
//!   and refusing a publisher who does not hold the derived authority, as the real one does.
//! - **A deterministic stub LLM** at `urn:llm:ask`, `urn:llm:{provider}:ask` and
//!   `urn:llm:select`, speaking ikigai-llm's JSON forms. It answers each ask with a canned
//!   draft per attempt and records every prompt it was sent. No model is ever called.
//! - **`ikigai_nl::space`** under the config the test chooses.
//!
//! Every kernel has a JSON Meta renderer: the grounding reads each action's arguments
//! from it, and the engine's own routing cannot be observed without one.

#![allow(dead_code)] // each test binary uses a different subset

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::executor::block_on;
use ikigai_core::{
    ActionSpec, ArgRef, ArgSpec, Capability, Description, Endpoint, EndpointSpace, Error, Exact,
    Fallback, Invocation, Iri, Kernel, ReprType, Representation, Request, Space, UriTemplate, Verb,
};
use ikigai_nl::{Grounding, SpaceConfig};
use ikigai_store::{space as store_space, DurableStore};

pub const LEDGER: &str = "urn:example:ledger";
pub const BOOKS: &str = "urn:example:books";
pub const IK: &str = "https://ikigai-rs.dev/ns#";

/// Six triples in LEDGER (three items typed `urn:example:Item`, each with a title — one
/// of them through `ik:summary`, so a focused vocabulary has a term the graph uses), and
/// two in BOOKS.
pub fn seed() -> String {
    format!(
        "INSERT DATA {{ \
           GRAPH <{LEDGER}> {{ \
             <urn:example:item:1> a <urn:example:Item> ; <http://purl.org/dc/terms/title> \"first\" . \
             <urn:example:item:2> a <urn:example:Item> ; <http://purl.org/dc/terms/title> \"second\" . \
             <urn:example:item:3> a <urn:example:Item> ; <{IK}summary> \"third\" . \
           }} \
           GRAPH <{BOOKS}> {{ \
             <urn:example:book:1> a <http://schema.org/Book> ; <http://schema.org/name> \"Bounded\" . \
           }} \
           <urn:example:host:1> <urn:example:p> \"in the default graph\" \
         }}"
    )
}

/// A script the stand-in holds.
#[derive(Clone)]
pub struct Script {
    pub name: &'static str,
    pub state: &'static str,
    pub public: bool,
    pub source: &'static str,
}

/// The scripts every test host holds: one public, one private, one draft.
pub const SCRIPTS: [Script; 3] = [
    Script {
        name: "stale-urgent",
        state: "published",
        public: true,
        source: "(source \"urn:iki:ledger:next\")",
    },
    Script {
        name: "book-titles",
        state: "published",
        public: false,
        source: "(source \"urn:iki:store:graph-select\")",
    },
    Script {
        name: "half-done",
        state: "draft",
        public: true,
        source: "(+ 1 2)",
    },
];

fn version(script: &Script) -> String {
    ikigai_nl::model::sha256(script.source.as_bytes())
}

fn may_read(cap: &Capability, script: &Script) -> bool {
    cap.allows(&format!("urn:cap:script:read:{}", script.name))
        || (script.public
            && script.state == "published"
            && cap.allows("urn:cap:script:read:public"))
}

fn json(value: serde_json::Value) -> Representation {
    Representation::new(
        ReprType::new("application/json"),
        serde_json::to_vec_pretty(&value).unwrap(),
    )
}

struct StandInCatalog;

#[async_trait]
impl Endpoint for StandInCatalog {
    async fn invoke(&self, inv: &Invocation<'_>) -> ikigai_core::Result<Representation> {
        let scripts: Vec<_> = SCRIPTS
            .iter()
            .filter(|s| may_read(inv.capability, s))
            .map(|s| {
                serde_json::json!({
                    "name": s.name,
                    "iri": format!("urn:script:{}", s.name),
                    "state": s.state,
                    "public": s.public,
                    "version": version(s),
                    "lastRun": null,
                    "error": null,
                })
            })
            .collect();
        // LIVE, as ikigai-script's is: no `.cacheable()`.
        Ok(json(serde_json::json!({ "schema": 1, "scripts": scripts })))
    }

    fn name(&self) -> &str {
        "script-catalog"
    }

    fn describe(&self) -> Description {
        Description::new("script-catalog").verb(Verb::Meta).action(
            ActionSpec::new(Verb::Source)
                .summary("The catalog.")
                .input(
                    ArgSpec::new("as")
                        .class("http://www.w3.org/2001/XMLSchema#string")
                        .one_of(["application/json"])
                        .optional(),
                )
                .output("application/json")
                .requires("urn:cap:script:read:*"),
        )
    }
}

/// A script the stand-in was sent with a Sink.
#[derive(Clone, Debug)]
pub struct SavedScript {
    pub version: String,
    pub state: String,
    pub language: String,
    pub requires: Vec<String>,
    pub source: String,
}

/// What the stand-in's Sink has kept, by name.
pub type Saved = Arc<Mutex<BTreeMap<String, SavedScript>>>;

struct StandInScript {
    saved: Saved,
}

fn arg<'a>(inv: &'a Invocation<'_>, name: &str) -> Option<&'a str> {
    match inv.request.args.get(name) {
        Some(ArgRef::Inline(bytes)) => std::str::from_utf8(bytes).ok(),
        _ => None,
    }
}

/// The real Sink's rules, in the real order: the write grant, the language, the DERIVED
/// authority (refused as `InvalidArgument` when the text does not analyze), no elevation
/// (refused as `Denied`), then `if-version` (refused as `Conflict`).
fn sink(inv: &Invocation<'_>, saved: &Saved, name: &str) -> ikigai_core::Result<Representation> {
    let write = format!("urn:cap:script:write:{name}");
    if !inv.capability.allows(&write) {
        return Err(Error::Denied(format!(
            "publishing a script needs `{write}`, which this capability does not hold"
        )));
    }
    let source = arg(inv, "content").ok_or_else(|| Error::MissingArgument("content".into()))?;
    let language = arg(inv, "language").unwrap_or("lisp");
    let requires: Vec<String> = match language {
        "sparql" => ikigai_nl::script_sparql::analyze(
            source,
            &ikigai_nl::script_sparql::SparqlDoor::store(),
        )?
        .requires()
        .into_iter()
        .collect(),
        _ => vec!["urn:cap:lisp".to_string()],
    };
    let missing: Vec<&String> = requires
        .iter()
        .filter(|scope| !held(inv.capability, scope))
        .collect();
    if !missing.is_empty() {
        return Err(Error::Denied(format!(
            "a script cannot be given more than its publisher holds, and this capability does \
             not hold {missing:?}"
        )));
    }
    let state = arg(inv, "state").unwrap_or("published").to_string();
    let mut saved = saved.lock().unwrap();
    let at = saved
        .get(name)
        .map(|s| s.version.clone())
        .or_else(|| SCRIPTS.iter().find(|s| s.name == name).map(version))
        .unwrap_or_else(|| "none".to_string());
    if let Some(expected) = arg(inv, "if-version") {
        if expected != at {
            return Err(Error::Conflict(format!(
                "urn:script:{name} is at {at}, not {expected}"
            )));
        }
    }
    let v = ikigai_nl::model::sha256(source.as_bytes());
    saved.insert(
        name.to_string(),
        SavedScript {
            version: v.clone(),
            state,
            language: language.to_string(),
            requires,
            source: source.to_string(),
        },
    );
    Ok(Representation::new(
        ReprType::new("text/plain"),
        format!("urn:script:{name}:version:{v}\n").into_bytes(),
    ))
}

/// "Holds", reading a trailing `*` as "some grant under this prefix", as the script host
/// does for a derived family.
fn held(cap: &Capability, scope: &str) -> bool {
    match scope.strip_suffix('*') {
        Some(prefix) => {
            cap.is_root()
                || cap
                    .scopes()
                    .is_some_and(|held| held.iter().any(|s| s.starts_with(prefix)))
        }
        None => cap.allows(scope),
    }
}

#[async_trait]
impl Endpoint for StandInScript {
    async fn invoke(&self, inv: &Invocation<'_>) -> ikigai_core::Result<Representation> {
        let name = inv.bindings.get("name").unwrap_or_default();
        if inv.request.verb == Verb::Sink {
            return sink(inv, &self.saved, name);
        }
        let kept = self.saved.lock().unwrap().get(name).cloned();
        if let Some(kept) = kept {
            if !inv
                .capability
                .allows(&format!("urn:cap:script:read:{name}"))
            {
                return Err(Error::Denied(format!(
                    "this capability holds none of urn:cap:script:read:{name}"
                )));
            }
            return Ok(json(serde_json::json!({
                "iri": format!("urn:script:{name}"),
                "schema": 1,
                "name": name,
                "version": kept.version,
                "state": kept.state,
                "public": false,
                "language": kept.language,
                "requires": kept.requires,
                "source": kept.source,
            })));
        }
        let script = SCRIPTS.iter().find(|s| s.name == name);
        match script {
            Some(s) if may_read(inv.capability, s) => {
                let v = version(s);
                Ok(json(serde_json::json!({
                    "iri": format!("urn:script:{}", s.name),
                    "schema": 1,
                    "name": s.name,
                    "version": v,
                    "state": s.state,
                    "public": s.public,
                    "granted": ["urn:cap:lisp"],
                    "exclusions": [],
                    "publisher": "urn:script:principal:unstamped",
                    "updated": null,
                    "history": [],
                    "versionIri": format!("urn:script:{}:version:{v}", s.name),
                    "language": "lisp",
                    "requires": ["urn:cap:lisp"],
                    "source": s.source,
                }))
                .cacheable())
            }
            Some(_) => Err(Error::Denied(format!(
                "this capability holds none of urn:cap:script:read:{name} — nor, for a \
                 public script, urn:cap:script:read:public"
            ))),
            None => Err(Error::NotFound(format!("urn:script:{name}"))),
        }
    }

    fn name(&self) -> &str {
        "script"
    }

    fn describe(&self) -> Description {
        Description::new("script")
            .verb(Verb::Meta)
            .action(
                ActionSpec::new(Verb::Source)
                    .summary("The script, without running it.")
                    .input(
                        ArgSpec::new("name")
                            .class("http://www.w3.org/2001/XMLSchema#string")
                            .binding(),
                    )
                    .input(
                        ArgSpec::new("as")
                            .class("http://www.w3.org/2001/XMLSchema#string")
                            .one_of(["application/json"])
                            .optional(),
                    )
                    .output("application/json")
                    .requires("urn:cap:script:read:*"),
            )
            .action(
                ActionSpec::new(Verb::Sink)
                    .summary("Publish or replace the script, or save a draft.")
                    .input(
                        ArgSpec::new("name")
                            .class("http://www.w3.org/2001/XMLSchema#string")
                            .binding(),
                    )
                    .input(ArgSpec::new("content").class("http://www.w3.org/2001/XMLSchema#string"))
                    .input(
                        ArgSpec::new("language")
                            .class("http://www.w3.org/2001/XMLSchema#string")
                            .one_of(["lisp", "sparql"])
                            .optional(),
                    )
                    .input(
                        ArgSpec::new("state")
                            .class("http://www.w3.org/2001/XMLSchema#string")
                            .one_of(["published", "draft"])
                            .optional(),
                    )
                    .input(
                        ArgSpec::new("if-version")
                            .class("http://www.w3.org/2001/XMLSchema#string")
                            .optional(),
                    )
                    .output("text/plain")
                    .requires("urn:cap:script:write:*"),
            )
    }
}

/// The stand-in for `ikigai-script`'s read doors and its script Sink.
pub fn scripts(saved: Saved) -> EndpointSpace {
    EndpointSpace::new()
        .bind(Exact::new("urn:script:catalog"), StandInCatalog)
        .bind(
            UriTemplate::parse("urn:script:{name}").unwrap(),
            StandInScript { saved },
        )
}

// ------------------------------------------------------------------------- the stub model

/// One prompt the stub was sent.
#[derive(Clone, Debug)]
pub struct Call {
    /// The door it arrived at (`urn:llm:ask`, `urn:llm:big:ask`).
    pub backend: String,
    pub prompt: String,
}

/// The stub's script and its record.
#[derive(Default)]
pub struct Stub {
    /// For an ask (matched as a substring of the prompt), the draft each attempt answers:
    /// the nth call for that ask gets the nth draft (the last one repeats).
    pub drafts: Vec<(String, Vec<String>)>,
    pub calls: Vec<Call>,
}

/// What the stub drafts for an ask it has no script for: a good query over LEDGER.
pub const DEFAULT_DRAFT: &str = "SELECT ?item ?title WHERE { GRAPH <urn:example:ledger> { \
    ?item <http://purl.org/dc/terms/title> ?title } } ORDER BY ?item";

struct StubAsk {
    stub: Arc<Mutex<Stub>>,
}

#[async_trait]
impl Endpoint for StubAsk {
    async fn invoke(&self, inv: &Invocation<'_>) -> ikigai_core::Result<Representation> {
        let prompt = arg(inv, "prompt")
            .ok_or_else(|| Error::MissingArgument("prompt".into()))?
            .to_string();
        let backend = inv.request.target.as_str().to_string();
        let model = match inv.bindings.get("provider") {
            Some(p) => format!("stub-{p}"),
            None => "stub-local".to_string(),
        };
        let mut stub = self.stub.lock().unwrap();
        let draft = match stub
            .drafts
            .iter()
            .find(|(ask, _)| prompt.contains(ask.as_str()))
        {
            Some((ask, drafts)) => {
                let seen = stub
                    .calls
                    .iter()
                    .filter(|c| c.prompt.contains(ask.as_str()))
                    .count();
                drafts[seen.min(drafts.len() - 1)].clone()
            }
            None => DEFAULT_DRAFT.to_string(),
        };
        stub.calls.push(Call { backend, prompt });
        // ikigai-llm's `as=application/json` envelope, the draft fenced as a model would.
        Ok(json(serde_json::json!({
            "text": format!("Here is the query.\n```sparql\n{draft}\n```\n"),
            "model": model,
            "finish_reason": "stop",
            "usage": null,
        })))
    }

    fn name(&self) -> &str {
        "llm-stub-ask"
    }

    fn describe(&self) -> Description {
        Description::new("llm-stub-ask").verb(Verb::Meta).action(
            ActionSpec::new(Verb::Source)
                .summary("A canned draft.")
                .input(
                    ArgSpec::new("provider")
                        .class("http://www.w3.org/2001/XMLSchema#string")
                        .binding(),
                )
                .input(ArgSpec::new("prompt").class("http://www.w3.org/2001/XMLSchema#string"))
                .input(
                    ArgSpec::new("temperature")
                        .class("http://www.w3.org/2001/XMLSchema#decimal")
                        .optional(),
                )
                .input(
                    ArgSpec::new("as")
                        .class("http://www.w3.org/2001/XMLSchema#string")
                        .optional(),
                )
                .output("application/json"),
        )
    }
}

struct StubSelect;

#[async_trait]
impl Endpoint for StubSelect {
    async fn invoke(&self, inv: &Invocation<'_>) -> ikigai_core::Result<Representation> {
        let needs = arg(inv, "needs").unwrap_or_default();
        let provider = if needs.contains("premium") {
            "big"
        } else {
            "small"
        };
        Ok(json(serde_json::json!({
            "backend": format!("urn:llm:{provider}:ask"),
            "provider": provider,
            "model": format!("stub-{provider}"),
        })))
    }

    fn name(&self) -> &str {
        "llm-stub-select"
    }

    fn describe(&self) -> Description {
        Description::new("llm-stub-select").verb(Verb::Meta).action(
            ActionSpec::new(Verb::Source)
                .summary("A canned selection.")
                .input(ArgSpec::new("needs").class("http://www.w3.org/2001/XMLSchema#string"))
                .input(
                    ArgSpec::new("as")
                        .class("http://www.w3.org/2001/XMLSchema#string")
                        .optional(),
                )
                .output("application/json"),
        )
    }
}

fn llm(stub: Arc<Mutex<Stub>>) -> EndpointSpace {
    EndpointSpace::new()
        .bind(Exact::new("urn:llm:ask"), StubAsk { stub: stub.clone() })
        .bind(Exact::new("urn:llm:select"), StubSelect)
        .bind(
            UriTemplate::parse("urn:llm:{provider}:ask").unwrap(),
            StubAsk { stub },
        )
}

/// A test host and what its doubles recorded.
pub struct Host {
    pub kernel: Kernel,
    pub stub: Arc<Mutex<Stub>>,
    pub saved: Saved,
}

impl Host {
    /// Every prompt the stub was sent, in order.
    pub fn prompts(&self) -> Vec<Call> {
        self.stub.lock().unwrap().calls.clone()
    }

    /// What the script stand-in kept under `name`.
    pub fn saved(&self, name: &str) -> Option<SavedScript> {
        self.saved.lock().unwrap().get(name).cloned()
    }
}

/// A host whose stub model answers each ask in `drafts` with its drafts, in order.
pub fn drafting_host(config: SpaceConfig, drafts: &[(&str, &[&str])]) -> Host {
    let stub = Arc::new(Mutex::new(Stub {
        drafts: drafts
            .iter()
            .map(|(ask, d)| (ask.to_string(), d.iter().map(|s| s.to_string()).collect()))
            .collect(),
        calls: Vec::new(),
    }));
    let saved: Saved = Arc::new(Mutex::new(BTreeMap::new()));
    let kernel = kernel(config, stub.clone(), saved.clone());
    Host {
        kernel,
        stub,
        saved,
    }
}

/// A kernel over `nl` beside the store (seeded), the vocabulary, the script stand-in and
/// the stub model.
pub fn host(config: SpaceConfig) -> Kernel {
    drafting_host(config, &[]).kernel
}

fn kernel(config: SpaceConfig, stub: Arc<Mutex<Stub>>, saved: Saved) -> Kernel {
    let store = DurableStore::in_memory().expect("an in-memory store");
    let root = Fallback::new(vec![
        Arc::new(ikigai_nl::space(config)) as Arc<dyn Space>,
        Arc::new(store_space(store)) as Arc<dyn Space>,
        Arc::new(ikigai_vocab::space()) as Arc<dyn Space>,
        Arc::new(scripts(saved)) as Arc<dyn Space>,
        Arc::new(llm(stub)) as Arc<dyn Space>,
    ]);
    let kernel = Kernel::with_meta_renderer(Arc::new(root), Arc::new(ikigai_vocab::TurtleRenderer));
    issue(
        &kernel,
        Verb::Sink,
        "urn:iki:store:update",
        &[("content", &seed())],
        &Capability::root(),
    )
    .expect("seeding the store");
    kernel
}

pub fn issue(
    kernel: &Kernel,
    verb: Verb,
    iri: &str,
    args: &[(&str, &str)],
    cap: &Capability,
) -> Result<Representation, Error> {
    let mut request = Request::new(verb, Iri::parse(iri).unwrap());
    for (name, value) in args {
        request = request.with_arg(*name, ArgRef::Inline(value.as_bytes().to_vec()));
    }
    block_on(kernel.issue(request, cap))
}

pub fn text(rep: &Representation) -> String {
    String::from_utf8(rep.bytes.clone()).unwrap()
}

/// The grounding's JSON face, parsed.
pub fn grounding(kernel: &Kernel, cap: &Capability, args: &[(&str, &str)]) -> Grounding {
    let mut all = vec![("as", "application/json")];
    all.extend_from_slice(args);
    let rep = issue(kernel, Verb::Source, "urn:nl:grounding", &all, cap)
        .unwrap_or_else(|e| panic!("the grounding: {e}"));
    serde_json::from_slice(&rep.bytes).expect("the JSON face parses into the model")
}

/// The grounding's Turtle face.
pub fn turtle(kernel: &Kernel, cap: &Capability, args: &[(&str, &str)]) -> String {
    let rep = issue(kernel, Verb::Source, "urn:nl:grounding", args, cap)
        .unwrap_or_else(|e| panic!("the grounding: {e}"));
    text(&rep)
}

/// May read LEDGER, and public scripts.
pub fn alice() -> Capability {
    Capability::scoped([
        format!("urn:cap:store:read:graph:{LEDGER}"),
        "urn:cap:script:read:public".to_string(),
    ])
}

/// May read BOOKS, and the private `book-titles` script.
pub fn bob() -> Capability {
    Capability::scoped([
        format!("urn:cap:store:read:graph:{BOOKS}"),
        "urn:cap:script:read:book-titles".to_string(),
    ])
}

/// Holds nothing.
pub fn anonymous() -> Capability {
    Capability::scoped(Vec::<String>::new())
}

/// `cap` plus the kernel's inspect grant, so a test can ask `urn:kernel:cached` and
/// `urn:kernel:uncached` as that caller.
pub fn inspecting(scopes: &[String]) -> Capability {
    let mut all = scopes.to_vec();
    all.push("urn:cap:kernel:inspect".to_string());
    Capability::scoped(all)
}
