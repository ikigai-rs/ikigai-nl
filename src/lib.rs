//! Natural language to ikigai: `urn:nl:*`.
//!
//! An LLM here is a **drafter, never an actor**: it is handed what is real (the
//! grounding), drafts in the most analyzable language that can do the job (a query, then a
//! plan, then Lisp), and its draft is validated mechanically and saved for a human to
//! publish. This version lands the first step, the grounding:
//!
//! ```text
//! urn:nl:grounding    Source    what a drafter may know, under the CALLER's capability
//! ```
//!
//! It is a composition, holding no state of its own. Each part is read through the kernel
//! under the caller's capability, so every part is exactly what that caller could read
//! directly:
//!
//! | part | read from | what it holds |
//! | --- | --- | --- |
//! | actions | `urn:kernel:actions` (once per verb), then `Meta as=application/json` of each offered endpoint | every action the caller may take, with its arguments |
//! | vocabulary | `urn:ikigai:vocab` | the `ik:` classes and properties with their comments |
//! | graphs | `urn:iki:store:graphs`, then five SPARQL queries per graph through `urn:iki:store:graph-select` | VoID-like totals, classes and predicates with counts, sample triples |
//! | examples | `urn:script:catalog`, then `urn:script:{name}` (or a configured list) | published scripts as worked examples |
//!
//! A host mounts [`space`] beside the resources it composes. The kernel needs a JSON Meta
//! renderer (`ikigai_vocab::TurtleRenderer`), because the actions' arguments come from
//! each endpoint's JSON description.
//!
//! ```
//! use std::sync::Arc;
//! use futures::executor::block_on;
//! use ikigai_core::{ArgRef, Capability, Fallback, Iri, Kernel, Request, Space, Verb};
//!
//! let root = Fallback::new(vec![
//!     Arc::new(ikigai_nl::space(ikigai_nl::SpaceConfig::new())) as Arc<dyn Space>,
//!     Arc::new(ikigai_vocab::space()) as Arc<dyn Space>,
//! ]);
//! let kernel = Kernel::with_meta_renderer(Arc::new(root), Arc::new(ikigai_vocab::TurtleRenderer));
//! let request = Request::new(Verb::Source, Iri::parse("urn:nl:grounding").unwrap())
//!     .with_arg("as", ArgRef::Inline(b"application/json".to_vec()));
//! let rep = block_on(kernel.issue(request, &Capability::scoped(Vec::<String>::new()))).unwrap();
//! let grounding: ikigai_nl::Grounding = serde_json::from_slice(&rep.bytes).unwrap();
//! assert!(grounding.identity.starts_with("sha256:"));
//! // An anonymous caller is offered no store, so no graph, not even a name.
//! assert!(grounding.graphs.items.is_empty());
//! assert!(grounding.vocabulary.shown > 0);
//! ```

#![deny(missing_docs)]

mod config;
mod endpoint;
mod focus;
mod gather;
pub mod model;
mod render;

use std::sync::Arc;

use ikigai_core::{EndpointSpace, Exact};

pub use config::{Examples, SpaceConfig};
pub use gather::{ACTIONS_IRI, CATALOG_IRI, GRAPHS_IRI, GRAPH_SELECT_IRI, SELECT_IRI, VOCAB_IRI};
pub use model::{Action, Example, GraphShape, Grounding, Input, Part, Partition, Term};
pub use render::VOID;

/// `urn:nl:grounding`.
pub const GROUNDING_IRI: &str = "urn:nl:grounding";

/// The namespace of the few terms the grounding's Turtle face needs that no shared
/// vocabulary has.
pub const NS: &str = "https://ikigai-rs.dev/ns/nl#";

/// The definitions of every [`NS`] term, as Turtle.
pub const VOCABULARY: &str = include_str!("nl.ttl");

/// The space binding `urn:nl:grounding` under `config`.
pub fn space(config: SpaceConfig) -> EndpointSpace {
    EndpointSpace::new().bind(
        Exact::new(GROUNDING_IRI),
        endpoint::GroundingEndpoint {
            config: Arc::new(config),
        },
    )
}
