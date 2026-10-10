//! The Turtle faces: they parse, they have no blank node, and every `ik:` term they state
//! is DEFINED in ikigai-vocab's vocabulary — a term the renderer invents fails here. The
//! other direction holds for the terms this crate brought to `ik:` (ledger #974): each is
//! still stated by some face, so a term nothing emits any more fails here too.

mod common;

use std::collections::BTreeSet;

use common::*;
use ikigai_core::{Capability, Verb};
use ikigai_nl::SpaceConfig;

/// The terms promoted into `ik:` from this crate's old `nl:` namespace (ledger #974),
/// plus `ik:model`, which an attempt reuses. Every one is stated by the grounding's
/// Turtle face or a draft's provenance.
const OWN: [&str; 14] = [
    "Grounding",
    "GroundingPart",
    "groundingFocus",
    "shownItems",
    "offeredItems",
    "sampleTriple",
    "shownSampleTriples",
    "shownClassPartitions",
    "shownPropertyPartitions",
    "draftAsk",
    "model",
    "draftValid",
    "draftError",
    "draftWarning",
];

fn ik_terms(turtle: &str) -> BTreeSet<String> {
    let triples = ikigai_conformance::rdf::parse("text/turtle", turtle.as_bytes())
        .unwrap_or_else(|e| panic!("the face parses: {e}\n{turtle}"));
    assert!(
        ikigai_conformance::rdf::blank_nodes(&triples).is_empty(),
        "no blank nodes"
    );
    ikigai_conformance::rdf::terms(&triples)
        .into_iter()
        .filter(|t| t.starts_with(ikigai_vocab::NS))
        .collect()
}

/// A draft's provenance, with every term it can state: a refused attempt (an error), then
/// a valid one whose preview leaves a required parameter unbound (a warning), each with
/// its model.
fn provenance() -> String {
    let refused = format!(
        "SELECT ?i WHERE {{ GRAPH <{LEDGER}> {{ ?i <http://purl.org/dc/terms/name> ?n }} }}"
    );
    let warned = format!(
        "# @param title xsd:string\n\
         SELECT ?i WHERE {{ GRAPH <{LEDGER}> {{ ?i <http://purl.org/dc/terms/title> ?title }} }}"
    );
    let h = drafting_host(SpaceConfig::new(), &[("every term", &[&refused, &warned])]);
    let rep = issue(
        &h.kernel,
        Verb::Sink,
        "urn:nl:sparql",
        &[("ask", "every term"), ("as", "text/turtle")],
        &Capability::root(),
    )
    .unwrap();
    text(&rep)
}

#[test]
fn every_ik_term_used_is_defined_in_the_vocabulary() {
    let kernel = host(SpaceConfig::new());
    let mut used = ik_terms(&turtle(&kernel, &Capability::root(), &[]));
    used.extend(ik_terms(&turtle(
        &kernel,
        &Capability::root(),
        &[("focus", LEDGER)],
    )));
    used.extend(ik_terms(&provenance()));

    // Declared: the subject of an `rdf:type` in ikigai-vocab's vocabulary, under `ik:`.
    let defined: BTreeSet<String> = oxttl::TurtleParser::new()
        .for_slice(ikigai_vocab::VOCABULARY)
        .map(|t| t.expect("ikigai-vocab's vocabulary parses"))
        .filter(|t| t.predicate.as_str() == "http://www.w3.org/1999/02/22-rdf-syntax-ns#type")
        .filter_map(|t| match t.subject {
            oxrdf::NamedOrBlankNode::NamedNode(n) if n.as_str().starts_with(ikigai_vocab::NS) => {
                Some(n.as_str().to_string())
            }
            _ => None,
        })
        .collect();
    let undefined: Vec<&String> = used.difference(&defined).collect();
    assert!(
        undefined.is_empty(),
        "stated but not defined: {undefined:?}"
    );

    let own: BTreeSet<String> = OWN
        .iter()
        .map(|t| format!("{}{t}", ikigai_vocab::NS))
        .collect();
    let silent: Vec<&String> = own.difference(&used).collect();
    assert!(
        silent.is_empty(),
        "defined for this crate but stated by no face: {silent:?}"
    );
}

#[test]
fn every_part_is_stated_in_turtle_with_its_origin_and_counts() {
    let kernel = host(SpaceConfig::new());
    let face = turtle(&kernel, &alice(), &[]);
    for (part, source) in [
        ("actions", "urn:kernel:actions"),
        ("vocabulary", "urn:ikigai:vocab"),
        ("graphs", "urn:iki:store:graphs"),
        ("examples", "urn:script:catalog"),
    ] {
        let block = face
            .split("\n<")
            .find(|b| b.contains(&format!(":{part}> a ik:GroundingPart")))
            .unwrap_or_else(|| panic!("no {part} part in\n{face}"));
        assert!(
            block.contains(&format!("prov:wasDerivedFrom <{source}>")),
            "{block}"
        );
        assert!(
            block.contains("ik:shownItems") && block.contains("ik:offeredItems"),
            "{block}"
        );
        assert!(block.contains("dcterms:identifier \"sha256:"), "{block}");
    }
    // An action carries the catalog's own contract and where to invoke it.
    assert!(face.contains("<urn:ikigai:endpoint:store-graph-select:action:source> a ik:Action"));
    assert!(face.contains("ik:endpoint <urn:iki:store:graph-select>"));
    assert!(face.contains("ik:template \"urn:script:{name}\""));
    // A worked example.
    assert!(face.contains("<urn:script:stale-urgent> a schema:SoftwareSourceCode"));
}
