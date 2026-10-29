//! The test host: a kernel with everything the grounding composes.
//!
//! - **The store** (`ikigai-store`, in memory) with two named graphs, `LEDGER` and `BOOKS`,
//!   and a triple in its default graph that no scoped read can reach.
//! - **The vocabulary** (`ikigai_vocab::space()`), the real one.
//! - **`ikigai-script`'s real space** (`ikigai_script::space`, a `MemoryBackend`, the store's
//!   SPARQL door, no ceiling), holding three scripts published by root: one public, one
//!   private, one draft. The catalog, each script's read rule, a draft's privacy and the
//!   Sink a draft is saved with (its analysis, its derived authority, no elevation,
//!   `if-version`) are the host's own, so this crate is tested against the contract it
//!   composes, not a second copy of it (ledger #1244 retired the hand-written stand-in).
//! - **A deterministic stub LLM** at `urn:llm:ask`, `urn:llm:{provider}:ask` and
//!   `urn:llm:select`, speaking ikigai-llm's JSON forms. It answers each ask with a canned
//!   draft per attempt and records every prompt it was sent. No model is ever called.
//! - **`ikigai_nl::space`** under the config the test chooses.
//!
//! Every kernel has a JSON Meta renderer: the grounding reads each action's arguments
//! from it, and the engine's own routing cannot be observed without one.

#![allow(dead_code)] // each test binary uses a different subset

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::executor::block_on;
use ikigai_core::{
    ActionSpec, ArgRef, ArgSpec, Capability, Description, Endpoint, EndpointSpace, Error, Exact,
    Fallback, Invocation, Iri, Kernel, ReprType, Representation, Request, Space, UriTemplate, Verb,
};
use ikigai_nl::{Grounding, SpaceConfig};
use ikigai_script::authority::{same_for_all, Ceiling};
use ikigai_script::sparql::SparqlDoor;
use ikigai_script::{Backend, MemoryBackend};
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

/// A script every test host holds, published by root.
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

/// A script as the script host keeps it, read from its backend.
#[derive(Clone, Debug)]
pub struct SavedScript {
    pub version: String,
    pub state: String,
    pub language: String,
    pub requires: Vec<String>,
    pub source: String,
}

/// `ikigai-script`'s space over `backend`: the store's SPARQL doors, no ceiling (the
/// derived authority and the publisher's grant are what a test is about).
pub fn scripts(backend: Arc<MemoryBackend>) -> ikigai_script::ScriptSpace {
    ikigai_script::space(
        ikigai_script::SpaceConfig::new(backend, same_for_all(Ceiling::unbounded()))
            .sparql(SparqlDoor::store()),
    )
}

fn json(value: serde_json::Value) -> Representation {
    Representation::new(
        ReprType::new("application/json"),
        serde_json::to_vec_pretty(&value).unwrap(),
    )
}

fn arg<'a>(inv: &'a Invocation<'_>, name: &str) -> Option<&'a str> {
    match inv.request.args.get(name) {
        Some(ArgRef::Inline(bytes)) => std::str::from_utf8(bytes).ok(),
        _ => None,
    }
}

// ------------------------------------------------------------------------- the stub model

/// One prompt the stub was sent.
#[derive(Clone, Debug)]
pub struct Call {
    /// The door it arrived at (`urn:llm:ask`, `urn:llm:big:ask`).
    pub backend: String,
    pub prompt: String,
    /// The `temperature=` it was asked at, when one was sent.
    pub temperature: Option<String>,
    /// The `max_tokens=` bound it was asked under, when one was sent (ledger #1203).
    pub max_tokens: Option<String>,
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
        stub.calls.push(Call {
            backend,
            prompt,
            temperature: arg(inv, "temperature").map(str::to_string),
            max_tokens: arg(inv, "max_tokens").map(str::to_string),
        });
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
                    ArgSpec::new("max_tokens")
                        .class("http://www.w3.org/2001/XMLSchema#positiveInteger")
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
    /// The same shared root used by the kernel, for direct conformance body probes.
    pub space: Arc<dyn Space>,
    pub stub: Arc<Mutex<Stub>>,
    /// The script host's backend, read directly to see what a Sink kept.
    pub backend: Arc<MemoryBackend>,
}

impl Host {
    /// Every prompt the stub was sent, in order.
    pub fn prompts(&self) -> Vec<Call> {
        self.stub.lock().unwrap().calls.clone()
    }

    /// What the script host keeps under `name`, other than the scripts every host holds.
    pub fn saved(&self, name: &str) -> Option<SavedScript> {
        if SCRIPTS.iter().any(|s| s.name == name) {
            return None;
        }
        let head = self.backend.head(name).unwrap()?;
        let version = self.backend.version(name, &head.version).unwrap()?;
        Some(SavedScript {
            version: head.version,
            state: head.state.as_str().to_string(),
            language: version.language.as_str().to_string(),
            requires: version.requires.into_iter().collect(),
            source: version.source,
        })
    }

    /// The names of every script saved here beyond the ones every host holds.
    pub fn saved_names(&self) -> Vec<String> {
        self.backend
            .names()
            .unwrap()
            .into_iter()
            .filter(|n| !SCRIPTS.iter().any(|s| s.name == n))
            .collect()
    }
}

/// A host whose stub model answers each ask in `drafts` with its drafts, in order.
pub fn drafting_host(config: SpaceConfig, drafts: &[(&str, &[&str])]) -> Host {
    let store = DurableStore::in_memory().expect("an in-memory store");
    drafting_host_over(config, store, drafts)
}

/// [`drafting_host`] over a store the test built, so it can set the store's own limits (its
/// time budget, say) as a host does.
pub fn drafting_host_over(
    config: SpaceConfig,
    store: DurableStore,
    drafts: &[(&str, &[&str])],
) -> Host {
    let stub = Arc::new(Mutex::new(Stub {
        drafts: drafts
            .iter()
            .map(|(ask, d)| (ask.to_string(), d.iter().map(|s| s.to_string()).collect()))
            .collect(),
        calls: Vec::new(),
    }));
    let backend = Arc::new(MemoryBackend::new());
    let (kernel, space) = kernel(config, stub.clone(), backend.clone(), store);
    Host {
        kernel,
        space,
        stub,
        backend,
    }
}

/// A kernel over `nl` beside the store (seeded), the vocabulary, the script host (holding
/// [`SCRIPTS`]) and the stub model.
pub fn host(config: SpaceConfig) -> Kernel {
    drafting_host(config, &[]).kernel
}

fn kernel(
    config: SpaceConfig,
    stub: Arc<Mutex<Stub>>,
    backend: Arc<MemoryBackend>,
    store: DurableStore,
) -> (Kernel, Arc<dyn Space>) {
    let root = Fallback::new(vec![
        Arc::new(ikigai_nl::space(config)) as Arc<dyn Space>,
        Arc::new(store_space(store)) as Arc<dyn Space>,
        Arc::new(ikigai_vocab::space()) as Arc<dyn Space>,
        Arc::new(scripts(backend)) as Arc<dyn Space>,
        Arc::new(llm(stub)) as Arc<dyn Space>,
    ]);
    let space: Arc<dyn Space> = Arc::new(root);
    let kernel = Kernel::with_meta_renderer(space.clone(), Arc::new(ikigai_vocab::TurtleRenderer));
    issue(
        &kernel,
        Verb::Sink,
        "urn:iki:store:update",
        &[("content", &seed())],
        &Capability::root(),
    )
    .expect("seeding the store");
    for script in &SCRIPTS {
        let public = script.public.to_string();
        issue(
            &kernel,
            Verb::Sink,
            &format!("urn:script:{}", script.name),
            &[
                ("content", script.source),
                ("state", script.state),
                ("public", &public),
            ],
            &Capability::root(),
        )
        .unwrap_or_else(|e| panic!("publishing {}: {e}", script.name));
    }
    (kernel, space)
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
