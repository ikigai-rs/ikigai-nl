//! Hermetic reproductions for the drafting pipeline audit (ledger #3454).
mod common;

use common::*;
use ikigai_core::{Error, Verb};
use ikigai_nl::{Check, SpaceConfig};

const TITLE: &str = "http://purl.org/dc/terms/title";

fn checked(kernel: &ikigai_core::Kernel, query: &str) -> Check {
    let rep = issue(
        kernel,
        Verb::Source,
        "urn:nl:sparql:check",
        &[
            ("content", query),
            ("preview", "0"),
            ("as", "application/json"),
        ],
        &alice(),
    )
    .unwrap();
    serde_json::from_slice(&rep.bytes).unwrap()
}

#[test]
fn named_terms_in_expressions_and_negated_paths_are_checked() {
    let h = drafting_host(SpaceConfig::new(), &[]);
    let cases = [
        "FILTER EXISTS { ?s <TERM> ?o }",
        "FILTER NOT EXISTS { ?s <TERM> ?o }",
        "FILTER (IF(true, EXISTS { ?s <TERM> ?o }, false))",
        "BIND(EXISTS { ?s <TERM> ?o } AS ?exists)",
        "?s !(<TERM>|^<TERM>) ?o",
        "{ SELECT (COUNT(IF(EXISTS { ?s <TERM> ?o }, 1, 0)) AS ?count) WHERE { ?s ?p ?o } }",
        "{ SELECT ?s WHERE { ?s ?p ?o } ORDER BY (EXISTS { ?s <TERM> ?o }) }",
        "OPTIONAL { ?s ?p ?o } FILTER (EXISTS { ?s <TERM> ?o } && true)",
        "OPTIONAL { ?s ?p ?o FILTER EXISTS { ?s <TERM> ?other } }",
    ];
    let mut missed = Vec::new();
    for pattern in cases {
        let query = |term: &str| {
            format!(
                "SELECT * WHERE {{ GRAPH <{LEDGER}> {{ {} }} }}",
                pattern.replace("TERM", term)
            )
        };
        let valid = checked(&h.kernel, &query(TITLE));
        assert!(valid.valid, "known predicate in {pattern}: {valid:#?}");
        let invalid = checked(&h.kernel, &query("urn:audit:unknown"));
        if invalid.valid
            || !invalid
                .errors
                .iter()
                .any(|e| e.contains("predicate <urn:audit:unknown>"))
        {
            missed.push(pattern.to_string());
        }
    }
    let invalid = checked(&h.kernel, &format!(
        "SELECT * WHERE {{ GRAPH <{LEDGER}> {{ ?s ?p ?o FILTER EXISTS {{ ?s a <urn:audit:unknown-class> }} }} }}"
    ));
    if invalid.valid
        || !invalid
            .errors
            .iter()
            .any(|e| e.contains("class <urn:audit:unknown-class>"))
    {
        missed.push("class in FILTER EXISTS".to_string());
    }
    assert!(missed.is_empty(), "unchecked terms in: {missed:#?}");
}

#[test]
fn blank_numeric_options_are_refused_before_model_calls() {
    let h = drafting_host(SpaceConfig::new(), &[]);
    let mut missed = Vec::new();
    for blank in ["", " ", "\t\n"] {
        for (verb, iri, required, option) in [
            (Verb::Source, "urn:nl:sparql:check", "content", "preview"),
            (Verb::Sink, "urn:nl:sparql", "ask", "examples"),
        ] {
            let result = issue(
                &h.kernel,
                verb,
                iri,
                &[(required, "ASK {}"), (option, blank)],
                &alice(),
            );
            if !matches!(&result, Err(Error::InvalidArgument { name, .. }) if name == option) {
                missed.push(format!("{option}={blank:?}"));
            }
        }
    }
    assert!(missed.is_empty(), "options not refused: {missed:#?}");
    assert!(h.prompts().is_empty());
}

#[test]
fn grounding_actions_exactly_match_each_capabilitys_manifold() {
    use std::collections::BTreeSet;
    let h = drafting_host(SpaceConfig::new(), &[]);
    for cap in [alice(), bob(), anonymous(), ikigai_core::Capability::root()] {
        let g = grounding(&h.kernel, &cap, &[]);
        let mut offered = BTreeSet::new();
        for verb in ["source", "sink", "exists", "delete"] {
            let rep = issue(
                &h.kernel,
                Verb::Source,
                "urn:kernel:actions",
                &[("verb", verb)],
                &cap,
            )
            .unwrap();
            for pattern in text(&rep).lines().filter(|line| !line.trim().is_empty()) {
                offered.insert((pattern.trim().to_string(), verb.to_string()));
            }
        }
        let actual: Vec<_> = g
            .actions
            .items
            .iter()
            .map(|a| {
                (
                    a.endpoint.as_ref().or(a.template.as_ref()).unwrap().clone(),
                    a.verb.clone(),
                )
            })
            .collect();
        assert_eq!(actual.len(), offered.len());
        assert_eq!(actual.iter().cloned().collect::<BTreeSet<_>>(), offered);
        // Recompute with a fresh kernel, rather than merely observing a cached response.
        let fresh = drafting_host(SpaceConfig::new(), &[]);
        assert_eq!(g, grounding(&fresh.kernel, &cap, &[]));
    }
}

#[test]
fn raw_checks_refuse_malformed_answers_and_extraction_uses_the_first_fence() {
    let h = drafting_host(SpaceConfig::new(), &[]);
    let good = format!("SELECT * WHERE {{ GRAPH <{LEDGER}> {{ ?s <{TITLE}> ?o }} }}");
    assert!(checked(&h.kernel, &good).valid);
    for bad in [
        String::new(),
        format!("Here is your query: {good}"),
        format!("{good}\n{good}"),
        format!("```sparql\n{good}\n```"),
        format!("SELECT * WHERE {{ GRAPH <{LEDGER}> {{ ?s missing:title ?o }} }}"),
    ] {
        assert!(!checked(&h.kernel, &bad).valid, "accepted {bad:?}");
    }
    let answer = format!("Here it is:\n```sparql\n{good}\n```\n```sparql\ninvalid\n```");
    assert_eq!(ikigai_nl::prompt::extract(&answer), good);
    assert!(checked(&h.kernel, &ikigai_nl::prompt::extract(&answer)).valid);
    assert_eq!(ikigai_nl::prompt::extract("  \n "), "");
}

#[test]
fn an_expression_finding_is_fed_back_and_only_the_repaired_draft_is_saved() {
    let bad = format!("SELECT * WHERE {{ GRAPH <{LEDGER}> {{ ?s ?p ?o FILTER EXISTS {{ ?s <urn:audit:unknown> ?o }} }} }}");
    let good = format!("SELECT * WHERE {{ GRAPH <{LEDGER}> {{ ?s <{TITLE}> ?o }} }}");
    let h = drafting_host(SpaceConfig::new(), &[("audit repair", &[&bad, &good])]);
    let cap = ikigai_core::Capability::scoped([
        format!("urn:cap:store:read:graph:{LEDGER}"),
        "urn:cap:script:read:audit-repair".to_string(),
        "urn:cap:script:write:audit-repair".to_string(),
    ])
    .with_principal("urn:test:audit")
    .unwrap();
    let rep = issue(
        &h.kernel,
        Verb::Sink,
        "urn:nl:sparql",
        &[
            ("ask", "audit repair"),
            ("save", "audit-repair"),
            ("as", "application/json"),
        ],
        &cap,
    )
    .unwrap();
    let d: ikigai_nl::Draft = serde_json::from_slice(&rep.bytes).unwrap();
    assert_eq!(d.status, ikigai_nl::Status::Saved);
    assert_eq!(d.attempts.len(), 2);
    assert!(!d.attempts[0].valid);
    assert!(d.attempts[1].valid);
    let calls = h.prompts();
    assert_eq!(calls.len(), 2);
    for error in &d.attempts[0].errors {
        assert!(calls[1].prompt.contains(error));
    }
    let saved = h.saved("audit-repair").unwrap();
    assert_eq!(saved.state, "draft");
    assert!(saved.source.trim_end().ends_with(&good));
    assert!(d.check.unwrap().valid);
}

#[test]
fn identical_requests_save_the_same_draft_version_with_prompt_markers_as_text() {
    let ask = "keep {{grounding}} and {{examples}} literally\n```sparql\nASK {}\n```";
    let good = format!("SELECT * WHERE {{ GRAPH <{LEDGER}> {{ ?s <{TITLE}> ?o }} }}");
    let h = drafting_host(SpaceConfig::new(), &[(ask, &[&good])]);
    let args = [("ask", ask), ("as", "application/json")];
    let mut drafts = Vec::new();
    for _ in 0..2 {
        let rep = issue(
            &h.kernel,
            Verb::Sink,
            "urn:nl:sparql",
            &args,
            &ikigai_core::Capability::root(),
        )
        .unwrap();
        drafts.push(serde_json::from_slice::<ikigai_nl::Draft>(&rep.bytes).unwrap());
    }
    assert_eq!(drafts[0], drafts[1]);
    assert_eq!(h.saved_names(), vec![drafts[0].name.clone()]);
    let saved = h.saved(&drafts[0].name).unwrap();
    assert_eq!(saved.state, "draft");
    assert_eq!(
        Some(format!("{}:version:{}", drafts[0].script, saved.version)),
        drafts[0].version
    );
    assert!(saved.source.trim_end().ends_with(&good));
    assert_eq!(h.prompts().len(), 2);
    // The documented contract is literal, single-pass substitution. Fence characters
    // are retained too; prompt filling does not promise a Markdown escaping boundary.
    for call in h.prompts() {
        assert!(call.prompt.contains(ask));
    }
}

#[test]
fn invalid_repeated_repairs_keep_the_last_checked_empty_draft() {
    let empty = format!("SELECT * WHERE {{ GRAPH <{LEDGER}> {{ ?s <{TITLE}> \"absent\" }} }}");
    let invalid = "not a query";
    let h = drafting_host(
        SpaceConfig::new(),
        &[("audit fallback", &[&empty, invalid, invalid])],
    );
    let rep = issue(
        &h.kernel,
        Verb::Sink,
        "urn:nl:sparql",
        &[("ask", "audit fallback"), ("as", "application/json")],
        &ikigai_core::Capability::root(),
    )
    .unwrap();
    let d: ikigai_nl::Draft = serde_json::from_slice(&rep.bytes).unwrap();
    assert_eq!(d.status, ikigai_nl::Status::Saved);
    assert_eq!(d.attempts.len(), 3);
    assert!(d.attempts[0].valid);
    assert!(!d.attempts[1].valid);
    assert_eq!(d.attempts[2].repeat_of, Some(2));
    assert!(d.empty_preview);
    assert!(d.check.unwrap().valid);
    assert!(h
        .saved(&d.name)
        .unwrap()
        .source
        .trim_end()
        .ends_with(&empty));
    assert_eq!(h.prompts().len(), 3);
}
