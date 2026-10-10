//! The module recipe as one test: `ikigai-conformance` walks `urn:nl:grounding` in a host
//! that has everything it composes, so the walk (under root) sees every part filled: the
//! actions, the vocabulary, both graphs' shapes and an example.
//!
//! # What the suite is told, and why
//!
//! - **`nl-grounding` is `cacheable`.** With configured examples every part is cacheable,
//!   so the composite must come back cached and carry its parts' threads. (With the
//!   default, the live script catalog, it is live by construction; `tests/grounding.rs`
//!   holds both polarities.)
//! - **One namespace is registered**: VoID, a W3C interest-group vocabulary the suite's
//!   well-known list does not carry (the registration waives every term under it). Every
//!   other term the faces state is `ik:` or well-known, so the suite's VOCABULARY check
//!   holds each `ik:` term to `ikigai_vocab::VOCABULARY` itself; `tests/turtle.rs` adds
//!   that the grounding's and the drafting's own `ik:` terms are all still stated.
//! - **`nl-prompt` is `pure`**: a constant template per name, cacheable with no thread but
//!   its own, because nothing can change it short of a new build.
//! - **`nl-sparql` gets a fixture** (an ask, piped as `content`): its contract cannot say
//!   "an ask, by name OR piped", so neither input is required and the walk's minimal inputs
//!   carry no ask. With it the walk drafts through the stub model.
//! - **`nl-sparql`'s AUTHORITY is waived, with the reason**: it declares no `requires`
//!   because its only write is a sub-request, the Sink on `urn:script:{name}` under the
//!   caller's own capability, which enforces `urn:cap:script:write:{name}` itself. Under no
//!   grants the walk's call RESOLVES (an unsaved draft is a legitimate answer) and the
//!   suite reads that as a mutation; `tests/sparql.rs` pins that nothing was written
//!   (`an_anonymous_caller_…`, `h.saved` empty).
//! - **Everything else in the kernel is opted out**: the store, the vocabulary and the
//!   script stand-in are here only because the grounding composes them, and the first two
//!   are conformed by their own crates' suites; the stub model is a test double.

mod common;

use common::*;
use ikigai_conformance::{Check, Fixture, Suite};
use ikigai_core::Verb;
use ikigai_nl::{Examples, SpaceConfig};

const COMPOSED: [&str; 3] = ["ikigai-vocab", "script", "script-catalog"];

fn suite() -> Suite {
    let mut suite = Suite::new()
        .cacheable("nl-grounding")
        .pure("nl-prompt")
        .fixture(Fixture::new("nl-sparql", Verb::Sink).arg("content", "the titles of the items"))
        .opt_out_check(
            "nl-sparql",
            Check::Authority,
            "its one write is the Sink on urn:script:{name} under the caller's own \
             capability, which enforces urn:cap:script:write:{name}; under no grants it \
             answers an unsaved draft and writes nothing (tests/sparql.rs pins it)",
        )
        .namespace(ikigai_nl::VOID);
    for id in ["llm-stub-ask", "llm-stub-select"] {
        suite = suite.opt_out(id, None, "a test double for the host's LLM doors");
    }
    for id in COMPOSED {
        suite = suite.opt_out(
            id,
            None,
            "composed by the grounding; conformed by its own crate (the script stand-in by \
             ikigai-script's)",
        );
    }
    for id in STORE {
        suite = suite.opt_out(
            id,
            None,
            "composed by the grounding; ikigai-store's own suite conforms it",
        );
    }
    suite
}

/// Every description id `ikigai_store::space` binds.
const STORE: [&str; 13] = [
    "store-ask",
    "store-construct",
    "store-describe",
    "store-graph-ask",
    "store-graph-construct",
    "store-graph-describe",
    "store-graph-select",
    "store-graph-update",
    "store-graphs",
    "store-info",
    "store-load",
    "store-select",
    "store-update",
];

fn kernel() -> ikigai_core::Kernel {
    host(SpaceConfig::new().examples(Examples::Named(vec!["stale-urgent".to_string()])))
}

#[test]
fn conforms() {
    let report = suite().run_blocking(&kernel());
    println!("{report}");
    assert!(report.is_clean(), "{report}");
}

/// ★ The positive half: a clean report over a walk that reached nothing would read exactly
/// like a clean report over a walk that reached everything.
#[test]
fn the_walk_reaches_the_grounding_and_probes_its_turtle() {
    let report = suite().run_blocking(&kernel());
    for id in ["nl-grounding", "nl-sparql", "nl-sparql-check", "nl-prompt"] {
        assert!(
            report.walked.iter().any(|w| w == id),
            "{id} not walked:\n{report}"
        );
    }
    let text = report.to_string();
    let probed = text
        .lines()
        .find(|l| l.starts_with("probed: nl-grounding source `text/turtle`"))
        .unwrap_or_else(|| panic!("{report}"));
    let triples: usize = probed
        .split(": ")
        .nth(2)
        .and_then(|rest| rest.split(' ').next())
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("{probed}"));
    assert!(triples > 1000, "{probed}");
}
