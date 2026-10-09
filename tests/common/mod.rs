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
//! - **`urn:nl:grounding`** under the config the test chooses.
//!
//! Every kernel has a JSON Meta renderer: the grounding reads each action's arguments
//! from it, and the engine's own routing cannot be observed without one.

#![allow(dead_code)] // each test binary uses a different subset

use std::sync::Arc;

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

struct StandInScript;

#[async_trait]
impl Endpoint for StandInScript {
    async fn invoke(&self, inv: &Invocation<'_>) -> ikigai_core::Result<Representation> {
        let name = inv.bindings.get("name").unwrap_or_default();
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
            _ => Err(Error::Denied(format!(
                "this capability holds none of urn:cap:script:read:{name} — nor, for a \
                 public script, urn:cap:script:read:public"
            ))),
        }
    }

    fn name(&self) -> &str {
        "script"
    }

    fn describe(&self) -> Description {
        Description::new("script").verb(Verb::Meta).action(
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
    }
}

/// The stand-in for `ikigai-script`'s two read doors.
pub fn scripts() -> EndpointSpace {
    EndpointSpace::new()
        .bind(Exact::new("urn:script:catalog"), StandInCatalog)
        .bind(
            UriTemplate::parse("urn:script:{name}").unwrap(),
            StandInScript,
        )
}

/// A kernel over `nl` beside the store (seeded), the vocabulary and the script stand-in.
pub fn host(config: SpaceConfig) -> Kernel {
    let store = DurableStore::in_memory().expect("an in-memory store");
    let root = Fallback::new(vec![
        Arc::new(ikigai_nl::space(config)) as Arc<dyn Space>,
        Arc::new(store_space(store)) as Arc<dyn Space>,
        Arc::new(ikigai_vocab::space()) as Arc<dyn Space>,
        Arc::new(scripts()) as Arc<dyn Space>,
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
