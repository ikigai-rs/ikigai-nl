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
    // An action is the manifold's match, saying where to invoke it, joined to the
    // catalog's own contract (ledger #1034).
    assert!(face.contains("<urn:ikigai:match:source:urn:iki:store:graph-select> a ik:ActionMatch"));
    assert!(face.contains("ik:endpoint <urn:iki:store:graph-select>"));
    assert!(face.contains("<urn:ikigai:match:source:urn:script:%7Bname%7D> a ik:ActionMatch"));
    assert!(face.contains("ik:template \"urn:script:{name}\""));
    assert!(face.contains("ik:contract <urn:ikigai:contract:store-graph-select:source:b3:"));
    assert!(!face.contains(":action:source>"), "no old-style action IRI");
    // A worked example.
    assert!(face.contains("<urn:script:stale-urgent> a schema:SoftwareSourceCode"));
}

/// A Turtle document's triples as (subject, predicate, object) text: an IRI as itself, a
/// literal as its value.
fn triples(turtle: &str) -> Vec<(String, String, String)> {
    oxttl::TurtleParser::new()
        .for_slice(turtle)
        .map(|t| t.unwrap_or_else(|e| panic!("parses: {e}\n{turtle}")))
        .map(|t| {
            let subject = match &t.subject {
                oxrdf::NamedOrBlankNode::NamedNode(n) => n.as_str().to_string(),
                other => other.to_string(),
            };
            let object = match &t.object {
                oxrdf::Term::NamedNode(n) => n.as_str().to_string(),
                oxrdf::Term::Literal(l) => l.value().to_string(),
                other => other.to_string(),
            };
            (subject, t.predicate.as_str().to_string(), object)
        })
        .collect()
}

fn objects<'a>(triples: &'a [(String, String, String)], s: &str, p: &str) -> Vec<&'a str> {
    let mut found: Vec<&str> = triples
        .iter()
        .filter(|(ts, tp, _)| ts == s && tp == p)
        .map(|(_, _, o)| o.as_str())
        .collect();
    found.sort();
    found
}

/// Ledger #1034 (core 0.1.91, ledger #948): an action is ONE match per door and verb,
/// named as the manifold names it (`urn:ikigai:match:{verb}:{pattern}`) and joined by
/// `ik:contract` to the catalog's content-addressed contract node. Where to invoke it hangs
/// on the match, never on a node of its own: a grounding that minted the old
/// `urn:ikigai:endpoint:{id}:action:{verb}` put `ik:endpoint` on one node while
/// `to_turtle` wrote the contract on another. And the contract the grounding renders is the
/// one the manifold cites, digest for digest.
#[test]
fn each_action_is_the_manifolds_match_joined_to_its_contract() {
    let kernel = host(SpaceConfig::new());
    let rdf_type = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
    let has_part = "http://purl.org/dc/terms/hasPart";
    let ik = |t: &str| format!("{}{t}", ikigai_vocab::NS);
    let typed = |g: &[(String, String, String)], class: &str| -> BTreeSet<String> {
        g.iter()
            .filter(|(_, p, o)| p == rdf_type && *o == ik(class))
            .map(|(s, _, _)| s.clone())
            .collect()
    };
    for cap in [Capability::root(), alice()] {
        let face = triples(&turtle(&kernel, &cap, &[]));
        let manifold = triples(&text(
            &issue(
                &kernel,
                Verb::Source,
                "urn:kernel:actions",
                &[("as", "text/turtle")],
                &cap,
            )
            .unwrap(),
        ));

        // Every match the manifold offers is in the grounding, as the same node, with the
        // same contract and the same door.
        let matches = typed(&manifold, "ActionMatch");
        assert!(!matches.is_empty(), "the manifold offers something");
        let actions_part = face
            .iter()
            .find(|(s, p, o)| p == rdf_type && *o == ik("GroundingPart") && s.ends_with(":actions"))
            .map(|(s, _, _)| s.clone())
            .expect("an actions part");
        let parts: BTreeSet<&str> = objects(&face, &actions_part, has_part)
            .into_iter()
            .collect();
        assert_eq!(
            typed(&face, "ActionMatch"),
            matches,
            "the grounding's matches are the manifold's"
        );
        for m in &matches {
            assert!(m.starts_with("urn:ikigai:match:"), "{m}");
            assert!(parts.contains(m.as_str()), "the actions part holds {m}");
            // The match states only these, each as the manifold does, so the grounding
            // unions with the manifold without a second value on any of its rows (the verb
            // and the scopes are the contract's).
            for (_, p, _) in face.iter().filter(|(s, _, _)| s == m) {
                assert!(
                    [
                        rdf_type.to_string(),
                        ik("contract"),
                        ik("endpoint"),
                        ik("template")
                    ]
                    .contains(p),
                    "{m} states {p}, which the manifold's row may disagree with"
                );
            }
            for term in ["contract", "endpoint", "template"] {
                assert_eq!(
                    objects(&face, m, &ik(term)),
                    objects(&manifold, m, &ik(term)),
                    "{m} ik:{term}"
                );
            }
            let [contract] = objects(&face, m, &ik("contract"))[..] else {
                panic!("{m} has one contract");
            };
            assert!(
                typed(&face, "Action").contains(contract),
                "the contract {contract} is rendered, as the catalog renders it"
            );
        }

        // One node per action: whatever says where to invoke is a match, and every
        // contract rendered is one some match cites.
        for (s, p, _) in &face {
            if *p == ik("endpoint") || *p == ik("template") {
                assert!(
                    matches.contains(s),
                    "{s} says where to invoke but is no match"
                );
            }
        }
        let cited: BTreeSet<&str> = matches
            .iter()
            .flat_map(|m| objects(&face, m, &ik("contract")))
            .collect();
        for contract in typed(&face, "Action") {
            assert!(
                cited.contains(contract.as_str()),
                "{contract} is cited by no match"
            );
        }
    }
}
