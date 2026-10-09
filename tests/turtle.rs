//! The Turtle face: it parses, it has no blank node, and the `nl:` terms it uses are
//! EXACTLY the ones `ikigai_nl::VOCABULARY` defines — a new term the renderer invents fails
//! here, and so does a defined term nothing emits any more.

mod common;

use std::collections::BTreeSet;

use common::*;
use ikigai_core::Capability;
use ikigai_nl::SpaceConfig;

fn nl_terms(turtle: &str) -> BTreeSet<String> {
    let triples = ikigai_conformance::rdf::parse("text/turtle", turtle.as_bytes())
        .unwrap_or_else(|e| panic!("the face parses: {e}\n{turtle}"));
    assert!(
        ikigai_conformance::rdf::blank_nodes(&triples).is_empty(),
        "no blank nodes"
    );
    ikigai_conformance::rdf::terms(&triples)
        .into_iter()
        .filter(|t| t.starts_with(ikigai_nl::NS))
        .collect()
}

#[test]
fn the_nl_terms_used_are_exactly_the_terms_defined() {
    let kernel = host(SpaceConfig::new());
    let mut used = nl_terms(&turtle(&kernel, &Capability::root(), &[]));
    used.extend(nl_terms(&turtle(
        &kernel,
        &Capability::root(),
        &[("focus", LEDGER)],
    )));

    let defined: BTreeSet<String> = oxttl::TurtleParser::new()
        .for_slice(ikigai_nl::VOCABULARY)
        .map(|t| t.expect("nl.ttl parses"))
        .filter(|t| t.predicate.as_str() == "http://www.w3.org/1999/02/22-rdf-syntax-ns#type")
        .filter_map(|t| match t.subject {
            oxrdf::NamedOrBlankNode::NamedNode(n) if n.as_str().starts_with(ikigai_nl::NS) => {
                Some(n.as_str().to_string())
            }
            _ => None,
        })
        .collect();
    assert_eq!(used, defined);
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
            .find(|b| b.contains(&format!(":{part}> a nl:Part")))
            .unwrap_or_else(|| panic!("no {part} part in\n{face}"));
        assert!(
            block.contains(&format!("prov:wasDerivedFrom <{source}>")),
            "{block}"
        );
        assert!(
            block.contains("nl:shown") && block.contains("nl:of"),
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
