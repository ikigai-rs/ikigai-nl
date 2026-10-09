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
//! - **Two namespaces are registered**: VoID (a W3C interest-group vocabulary the suite's
//!   well-known list does not carry) and this crate's own `nl:`. The registration waives
//!   every term under each, so `tests/turtle.rs` pins the `nl:` terms EXACTLY against
//!   `ikigai_nl::VOCABULARY`, red in both directions.
//! - **Everything else in the kernel is opted out**: the store, the vocabulary and the
//!   script stand-in are here only because the grounding composes them, and the first two
//!   are conformed by their own crates' suites.

mod common;

use common::*;
use ikigai_conformance::Suite;
use ikigai_nl::{Examples, SpaceConfig};

const COMPOSED: [&str; 3] = ["ikigai-vocab", "script", "script-catalog"];

fn suite() -> Suite {
    let mut suite = Suite::new()
        .cacheable("nl-grounding")
        .namespace(ikigai_nl::VOID)
        .namespace(ikigai_nl::NS);
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
    assert!(
        report.walked.iter().any(|id| id == "nl-grounding"),
        "{report}"
    );
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
