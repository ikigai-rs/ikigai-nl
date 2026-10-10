//! `urn:nl:sparql`, end to end, with a STUB model: the acceptance of ledger #958.
//!
//! Every draft here is canned (the stub answers each ask with a fixed draft per attempt),
//! so what is under test is the funnel around the model: what it is given, what is checked,
//! what is fed back, what is saved, and what is never shown or done.

mod common;

use std::collections::BTreeSet;

use common::*;
use ikigai_core::{Capability, Error, Verb};
use ikigai_nl::{Draft, Escalation, Llm, SpaceConfig, Status};

const TITLE: &str = "http://purl.org/dc/terms/title";

/// Alice: LEDGER, public scripts, and her own draft names.
fn writer(names: &[&str]) -> Capability {
    let mut scopes = vec![
        format!("urn:cap:store:read:graph:{LEDGER}"),
        "urn:cap:script:read:public".to_string(),
    ];
    for n in names {
        scopes.push(format!("urn:cap:script:write:{n}"));
        scopes.push(format!("urn:cap:script:read:{n}"));
    }
    Capability::scoped(scopes)
}

fn draft(h: &Host, cap: &Capability, args: &[(&str, &str)]) -> Draft {
    let mut all = vec![("as", "application/json")];
    all.extend_from_slice(args);
    let rep = issue(&h.kernel, Verb::Sink, "urn:nl:sparql", &all, cap)
        .unwrap_or_else(|e| panic!("urn:nl:sparql: {e}"));
    serde_json::from_slice(&rep.bytes).expect("the JSON face parses into the model")
}

fn good(predicate: &str) -> String {
    format!(
        "SELECT ?item ?title WHERE {{ GRAPH <{LEDGER}> {{ ?item <{predicate}> ?title }} }} \
         ORDER BY ?item"
    )
}

/// N-Triples, parsed: (subject, predicate, object) as strings, objects in N-Triples form.
fn triples(nt: &str) -> Vec<(String, String, String)> {
    oxttl::NTriplesParser::new()
        .for_slice(nt.as_bytes())
        .map(|t| {
            let t = t.unwrap_or_else(|e| panic!("N-Triples: {e}\n{nt}"));
            (
                t.subject.to_string(),
                t.predicate.as_str().to_string(),
                t.object.to_string(),
            )
        })
        .collect()
}

fn objects(ts: &[(String, String, String)], s: &str, p: &str) -> Vec<String> {
    ts.iter()
        .filter(|(ts, tp, _)| ts == s && tp == p)
        .map(|(_, _, o)| o.clone())
        .collect()
}

#[test]
fn a_good_ask_produces_a_valid_saved_draft_with_a_preview() {
    let h = drafting_host(SpaceConfig::new(), &[("titles of items", &[&good(TITLE)])]);
    let d = draft(
        &h,
        &writer(&["titles"]),
        &[("ask", "titles of items"), ("save", "titles")],
    );
    assert_eq!(d.status, Status::Saved, "{d:#?}");
    assert_eq!(d.attempts.len(), 1);
    assert!(d.attempts[0].valid);
    assert_eq!(d.attempts[0].backend, "urn:llm:ask");
    assert_eq!(d.attempts[0].model.as_deref(), Some("stub-local"));
    let version = d.version.clone().unwrap();
    assert!(
        version.starts_with("urn:script:titles:version:sha256:"),
        "{version}"
    );

    let check = d.check.clone().unwrap();
    assert_eq!(check.form.as_deref(), Some("select"));
    assert_eq!(
        check.requires,
        vec![format!("urn:cap:store:read:graph:{LEDGER}")]
    );
    let preview = check.preview.unwrap();
    assert_eq!(preview.columns, vec!["item", "title"]);
    assert_eq!(
        preview.rows,
        vec![
            vec!["<urn:example:item:1>".to_string(), "\"first\"".to_string()],
            vec!["<urn:example:item:2>".to_string(), "\"second\"".to_string()],
        ]
    );

    // Saved as a DRAFT, in SPARQL, under exactly the derived authority, and it is the
    // query the model wrote, after the provenance block.
    let saved = h.saved("titles").unwrap();
    assert_eq!(saved.state, "draft");
    assert_eq!(saved.language, "sparql");
    assert_eq!(saved.requires, check.requires);
    assert_eq!(Some(saved.source.clone()), d.query);
    assert!(saved.source.ends_with(&format!("{}\n", good(TITLE))));
    assert!(saved.source.starts_with("# Drafted by urn:nl:sparql"));

    // The plain face: the version, the query, the preview.
    let plain = text(
        &issue(
            &h.kernel,
            Verb::Sink,
            "urn:nl:sparql",
            &[("ask", "titles of items"), ("save", "titles")],
            &writer(&["titles"]),
        )
        .unwrap(),
    );
    assert!(
        plain.starts_with("urn:script:titles:version:sha256:"),
        "{plain}"
    );
    assert!(plain.contains("DRAFT"), "{plain}");
    assert!(plain.contains("<urn:example:item:1>\t\"first\""), "{plain}");

    // The model was grounded in what Alice may read, and told the rules.
    let prompt = &h.prompts()[0].prompt;
    assert!(prompt.contains("titles of items"));
    assert!(prompt.contains(&format!("<{LEDGER}>")));
    assert!(prompt.contains(TITLE));
    assert!(prompt.contains("# @param"));
}

#[test]
fn a_piped_ask_arrives_as_content() {
    let h = drafting_host(SpaceConfig::new(), &[("piped ask", &[&good(TITLE)])]);
    let d = draft(
        &h,
        &writer(&["piped"]),
        &[("content", "piped ask"), ("save", "piped")],
    );
    assert_eq!(d.status, Status::Saved);
    assert_eq!(d.ask, "piped ask");
    let none = issue(
        &h.kernel,
        Verb::Sink,
        "urn:nl:sparql",
        &[("save", "piped")],
        &writer(&["piped"]),
    )
    .unwrap_err();
    assert!(
        matches!(none, Error::MissingArgument(ref n) if n == "ask"),
        "{none}"
    );
}

#[test]
fn a_draft_with_an_unknown_predicate_is_repaired() {
    let unknown = "http://purl.org/dc/terms/name";
    let h = drafting_host(
        SpaceConfig::new(),
        &[("names of items", &[&good(unknown), &good(TITLE)])],
    );
    let d = draft(
        &h,
        &writer(&["names"]),
        &[("ask", "names of items"), ("save", "names")],
    );
    assert_eq!(d.status, Status::Saved, "{d:#?}");
    assert_eq!(d.attempts.len(), 2);
    assert!(!d.attempts[0].valid);
    assert!(
        d.attempts[0].errors.iter().any(|e| e.contains(unknown)),
        "{:?}",
        d.attempts[0].errors
    );
    assert!(d.attempts[1].valid);
    // What the repair was told: its previous draft and exactly what was refused.
    let calls = h.prompts();
    assert_eq!(calls.len(), 2);
    assert!(!calls[0].prompt.contains("The checker refused it"));
    assert!(calls[1].prompt.contains("The checker refused it"));
    assert!(calls[1].prompt.contains(&good(unknown)));
    assert!(calls[1].prompt.contains(&d.attempts[0].errors[0]));
    // The saved draft is the repaired query.
    assert!(h.saved("names").unwrap().source.contains(&good(TITLE)));
}

#[test]
fn a_draft_reaching_a_graph_the_caller_cannot_read_is_refused_without_leaking_it() {
    let books =
        format!("SELECT ?b ?n WHERE {{ GRAPH <{BOOKS}> {{ ?b <http://schema.org/name> ?n }} }}");
    let h = drafting_host(SpaceConfig::new(), &[("book names", &[&books])]);
    let d = draft(
        &h,
        &writer(&["books"]),
        &[("ask", "book names"), ("save", "books")],
    );
    assert_eq!(d.status, Status::Failed, "{d:#?}");
    assert!(d.query.is_none() && d.check.is_none() && d.version.is_none());
    assert!(h.saved("books").is_none(), "nothing is saved");
    // The second answer repeated the first, so drafting stopped there.
    assert_eq!(d.attempts.len(), 2);
    assert!(d.attempts[1].errors[0].contains("same draft as attempt 1"));
    assert!(
        d.attempts[0].errors[0].contains("not a graph this caller may read"),
        "{:?}",
        d.attempts[0].errors
    );

    // Nothing of the graph reached the model: no content, no class, no shape. (The IRI
    // in the repair prompt is the model's own, quoted back from its draft.)
    for call in h.prompts() {
        assert!(!call.prompt.contains("Bounded"), "{}", call.prompt);
        assert!(!call.prompt.contains("http://schema.org/Book"));
    }
    for face in [serde_json::to_string(&d).unwrap(), d.prov.clone()] {
        assert!(!face.contains("Bounded") && !face.contains("http://schema.org/Book"));
    }

    // ★ And the refusal does not say the graph exists: a graph that does not is refused in
    // the same words.
    let check = |graph: &str| -> Vec<String> {
        let q = books.replace(BOOKS, graph);
        let rep = issue(
            &h.kernel,
            Verb::Source,
            "urn:nl:sparql:check",
            &[("content", &q), ("as", "application/json")],
            &writer(&[]),
        )
        .unwrap();
        let c: ikigai_nl::Check = serde_json::from_slice(&rep.bytes).unwrap();
        c.errors.iter().map(|e| e.replace(graph, "GRAPH")).collect()
    };
    assert_eq!(check(BOOKS), check("urn:example:nowhere"));
}

#[test]
fn a_draft_that_never_validates_comes_back_failed_with_every_attempt() {
    let broken = [
        "SELECT ?x WHERE { GRAPH <urn:example:ledger> { ?x ",
        "INSERT DATA { GRAPH <urn:example:ledger> { <urn:x> <urn:p> <urn:o> } }",
        &good("http://purl.org/dc/terms/creator"),
    ];
    let h = drafting_host(SpaceConfig::new(), &[("broken ask", &broken)]);
    let d = draft(
        &h,
        &writer(&["broken"]),
        &[("ask", "broken ask"), ("save", "broken")],
    );
    assert_eq!(d.status, Status::Failed);
    assert!(d.query.is_none());
    assert_eq!(d.attempts.len(), 3, "the default bound");
    assert!(d.attempts.iter().all(|a| !a.valid && !a.errors.is_empty()));
    assert!(
        d.attempts[1].errors[0].contains("UPDATE"),
        "{:?}",
        d.attempts[1].errors
    );
    assert!(d.note.as_deref().unwrap().contains("the bound is 3"));
    assert!(h.saved("broken").is_none());

    // The plain face lists every attempt and its errors, and no query as THE answer. (A
    // fresh host: the stub's script is per host, and this one has used it up.)
    let h = drafting_host(SpaceConfig::new(), &[("broken ask", &broken)]);
    let plain = text(
        &issue(
            &h.kernel,
            Verb::Sink,
            "urn:nl:sparql",
            &[("ask", "broken ask"), ("save", "broken")],
            &writer(&["broken"]),
        )
        .unwrap(),
    );
    assert!(plain.starts_with("no valid draft"), "{plain}");
    assert_eq!(plain.matches("\nattempt ").count(), 3, "{plain}");

    // Its provenance still says what was tried.
    let ts = triples(&d.prov);
    let attempts: Vec<_> = ts
        .iter()
        .filter(|(_, p, _)| p == "http://www.w3.org/ns/prov#wasGeneratedBy")
        .collect();
    assert_eq!(attempts.len(), 3);
    assert_eq!(
        ts.iter()
            .filter(|(_, p, o)| p == "https://ikigai-rs.dev/ns#draftValid"
                && o.starts_with("\"false\""))
            .count(),
        3
    );
}

#[test]
fn the_saved_draft_carries_its_provenance() {
    // An ask that would break out of a comment line, if it were ever written raw.
    let ask = "titles, please\n} ; DROP ALL ; #";
    let h = drafting_host(
        SpaceConfig::new(),
        &[(
            "titles, please",
            &[&good("http://purl.org/dc/terms/name"), &good(TITLE)],
        )],
    );
    let d = draft(&h, &writer(&["prov"]), &[("ask", ask), ("save", "prov")]);
    assert_eq!(d.status, Status::Saved, "{d:#?}");
    let saved = h.saved("prov").unwrap();

    // The block reads back, and it is the provenance the answer gave.
    let block = ikigai_nl::prov_of(&saved.source).expect("a provenance block");
    let ts = triples(&block);
    let a = format!("<{}>", d.activity);
    assert!(d.activity.starts_with("urn:nl:sparql:drafting:"));
    assert_eq!(
        objects(&ts, &a, "https://ikigai-rs.dev/ns#draftAsk"),
        vec![oxrdf::Literal::new_simple_literal(ask).to_string()]
    );
    let used = objects(&ts, &a, "http://www.w3.org/ns/prov#used");
    assert!(used.contains(&format!("<{}>", d.grounding)), "{used:?}");
    assert!(used.contains(&"<urn:nl:prompt:sparql>".to_string()));
    assert!(used.contains(&"<urn:nl:prompt:sparql-repair>".to_string()));
    // The grounding by identity, and each part's.
    let g = format!("<{}>", d.grounding);
    assert_eq!(
        objects(&ts, &g, "http://purl.org/dc/terms/identifier"),
        vec![format!("\"{}\"", d.grounding_identity)]
    );
    assert_eq!(
        objects(&ts, &g, "http://purl.org/dc/terms/hasPart").len(),
        4
    );
    // Each attempt: who drafted it, what it was, and what the checks said.
    let first = format!("<{}:attempt:1>", d.activity);
    let second = format!("<{}:attempt:2>", d.activity);
    assert_eq!(
        objects(&ts, &first, "http://www.w3.org/ns/prov#wasAttributedTo"),
        vec!["<urn:llm:ask>"]
    );
    assert_eq!(
        objects(&ts, &first, "https://ikigai-rs.dev/ns#model"),
        vec!["\"stub-local\""]
    );
    assert!(
        objects(&ts, &first, "https://ikigai-rs.dev/ns#draftValid")[0].starts_with("\"false\"")
    );
    assert!(!objects(&ts, &first, "https://ikigai-rs.dev/ns#draftError").is_empty());
    assert!(
        objects(&ts, &second, "https://ikigai-rs.dev/ns#draftValid")[0].starts_with("\"true\"")
    );
    assert_eq!(
        objects(&ts, &second, "http://www.w3.org/ns/prov#wasRevisionOf"),
        vec![first.clone()]
    );
    assert_eq!(
        objects(
            &ts,
            "<urn:script:prov>",
            "http://www.w3.org/ns/prov#wasDerivedFrom"
        ),
        vec![second]
    );

    // ★ The ask stayed a literal inside a comment: the saved text analyzes to exactly the
    // query the model wrote, with the authority that query derives and nothing more.
    let analysis = ikigai_nl::script_sparql::analyze(
        &saved.source,
        &ikigai_nl::script_sparql::SparqlDoor::store(),
    )
    .unwrap();
    assert_eq!(
        analysis.graphs.into_iter().collect::<Vec<_>>(),
        vec![LEDGER.to_string()]
    );
    // The Turtle face is the same provenance (from a fresh host: the stub's script is per
    // host).
    let h = drafting_host(
        SpaceConfig::new(),
        &[(
            "titles, please",
            &[&good("http://purl.org/dc/terms/name"), &good(TITLE)],
        )],
    );
    let turtle = text(
        &issue(
            &h.kernel,
            Verb::Sink,
            "urn:nl:sparql",
            &[("ask", ask), ("save", "prov"), ("as", "text/turtle")],
            &writer(&["prov"]),
        )
        .unwrap(),
    );
    assert_eq!(triples(&turtle), triples(&d.prov));
}

#[test]
fn an_anonymous_caller_is_grounded_anonymously_and_drafts_only_against_public_graphs() {
    let h = drafting_host(
        SpaceConfig::new(),
        &[(
            "public books",
            &[&format!(
                "SELECT ?b ?n WHERE {{ GRAPH <{BOOKS}> {{ ?b <http://schema.org/name> ?n }} }}"
            )],
        )],
    );
    // Holding nothing: told there is no graph, and a draft over the ledger is refused.
    let d = draft(&h, &anonymous(), &[("ask", "anything at all")]);
    assert_eq!(d.status, Status::Failed);
    let prompt = &h.prompts()[0].prompt;
    assert!(prompt.contains("may read no named graph"), "{prompt}");
    assert!(!prompt.contains(LEDGER) && !prompt.contains(BOOKS));
    assert!(d.attempts[0].errors[0].contains("not a graph this caller may read"));

    // A host's anonymous principal holding one PUBLIC graph drafts against that graph: a
    // valid draft with a preview, answered but not saved (it may write no script).
    let public = Capability::scoped([format!("urn:cap:store:read:graph:{BOOKS}")]);
    let d = draft(&h, &public, &[("ask", "public books")]);
    assert_eq!(d.status, Status::Unsaved, "{d:#?}");
    assert!(d.note.as_deref().unwrap().contains("urn:cap:script:write:"));
    let preview = d.check.unwrap().preview.unwrap();
    assert_eq!(
        preview.rows,
        vec![vec!["<urn:example:book:1>", "\"Bounded\""]]
    );
    assert!(h.saved.lock().unwrap().is_empty());
}

#[test]
fn a_parameter_is_declared_and_its_default_bound_in_the_preview() {
    let query = format!(
        "# @param title xsd:string default second -- the title to find\n\
         SELECT ?item WHERE {{ GRAPH <{LEDGER}> {{ ?item <{TITLE}> ?title }} }}"
    );
    let h = drafting_host(SpaceConfig::new(), &[("item by title", &[&query])]);
    let d = draft(
        &h,
        &writer(&["by-title"]),
        &[("ask", "item by title"), ("save", "by-title")],
    );
    assert_eq!(d.status, Status::Saved, "{d:#?}");
    let check = d.check.unwrap();
    assert_eq!(check.parameters.len(), 1);
    assert_eq!(check.parameters[0].name, "title");
    assert_eq!(check.parameters[0].default.as_deref(), Some("second"));
    // Bound as a typed value, not spliced: one row, the second item.
    assert_eq!(
        check.preview.unwrap().rows,
        vec![vec!["<urn:example:item:2>".to_string()]]
    );
    // The declaration survives the provenance block above it.
    let saved = h.saved("by-title").unwrap();
    let params = ikigai_nl::script_sparql::parameters(&saved.source).unwrap();
    assert_eq!(params.len(), 1);
}

#[test]
fn the_host_escalates_only_by_its_own_policy() {
    let config = SpaceConfig::new().llm(Llm::default().escalate(Escalation {
        after: 1,
        needs: "cost<=premium".to_string(),
    }));
    let h = drafting_host(
        config,
        &[(
            "escalate me",
            &[&good("http://purl.org/dc/terms/name"), &good(TITLE)],
        )],
    );
    let d = draft(
        &h,
        &writer(&["esc"]),
        &[("ask", "escalate me"), ("save", "esc")],
    );
    assert_eq!(d.status, Status::Saved, "{d:#?}");
    let backends: Vec<_> = h.prompts().into_iter().map(|c| c.backend).collect();
    assert_eq!(backends, vec!["urn:llm:ask", "urn:llm:big:ask"]);
    assert_eq!(d.attempts[1].model.as_deref(), Some("stub-big"));
    let ts = triples(&d.prov);
    let associated: BTreeSet<_> = objects(
        &ts,
        &format!("<{}>", d.activity),
        "http://www.w3.org/ns/prov#wasAssociatedWith",
    )
    .into_iter()
    .collect();
    assert_eq!(
        associated,
        BTreeSet::from(["<urn:llm:ask>".to_string(), "<urn:llm:big:ask>".to_string()])
    );
}

#[test]
fn the_repair_bound_is_the_hosts() {
    let drafts: Vec<String> = (0..6)
        .map(|i| good(&format!("http://purl.org/dc/terms/unknown{i}")))
        .collect();
    let drafts: Vec<&str> = drafts.iter().map(String::as_str).collect();
    let h = drafting_host(
        SpaceConfig::new().max_attempts(5),
        &[("five tries", &drafts)],
    );
    let d = draft(
        &h,
        &writer(&["five"]),
        &[("ask", "five tries"), ("save", "five")],
    );
    assert_eq!(d.status, Status::Failed);
    assert_eq!(d.attempts.len(), 5);
    assert_eq!(h.prompts().len(), 5);
    assert_eq!(SpaceConfig::new().max_attempts(0).max_attempts, 1);
    assert_eq!(
        SpaceConfig::new().max_attempts(99).max_attempts,
        ikigai_nl::MAX_ATTEMPTS
    );
}

#[test]
fn a_published_script_is_never_replaced_by_a_draft() {
    let h = drafting_host(SpaceConfig::new(), &[]);
    // `stale-urgent` is published (and public, so Alice can read its head).
    let cap = writer(&["stale-urgent"]);
    let err = issue(
        &h.kernel,
        Verb::Sink,
        "urn:nl:sparql",
        &[("ask", "anything"), ("save", "stale-urgent")],
        &cap,
    )
    .unwrap_err();
    assert!(
        matches!(&err, Error::InvalidArgument { name, .. } if name == "save"),
        "{err}"
    );
    assert!(h.prompts().is_empty(), "refused before any model was asked");
    // A bad name too.
    let err = issue(
        &h.kernel,
        Verb::Sink,
        "urn:nl:sparql",
        &[("ask", "anything"), ("save", "Not A Name")],
        &cap,
    )
    .unwrap_err();
    assert!(matches!(&err, Error::InvalidArgument { name, .. } if name == "save"));
}

#[test]
fn re_drafting_a_draft_replaces_it_and_root_gets_a_derived_name() {
    let h = drafting_host(SpaceConfig::new(), &[("again", &[&good(TITLE)])]);
    let cap = writer(&["again"]);
    let first = draft(&h, &cap, &[("ask", "again"), ("save", "again")]);
    let second = draft(&h, &cap, &[("ask", "again"), ("save", "again")]);
    assert_eq!(first.status, Status::Saved);
    assert_eq!(second.status, Status::Saved, "{second:#?}");

    let d = draft(&h, &Capability::root(), &[("ask", "again")]);
    assert_eq!(d.status, Status::Saved);
    assert!(d.name.starts_with("nl-sparql-") && d.name.len() == "nl-sparql-".len() + 12);
    assert!(h.saved(&d.name).is_some());
}

#[test]
fn the_check_is_a_resource_of_its_own() {
    let h = drafting_host(SpaceConfig::new(), &[]);
    let rep = issue(
        &h.kernel,
        Verb::Source,
        "urn:nl:sparql:check",
        &[("content", &good(TITLE))],
        &writer(&[]),
    )
    .unwrap();
    let plain = text(&rep);
    assert!(
        plain.starts_with(&format!("valid select over <{LEDGER}>")),
        "{plain}"
    );
    assert!(plain.contains("preview (LIMIT 5)"), "{plain}");

    // A bounded grounding cannot know every predicate: past the bound, a warning.
    let h = drafting_host(SpaceConfig::new().max_partitions(1), &[]);
    let rep = issue(
        &h.kernel,
        Verb::Source,
        "urn:nl:sparql:check",
        &[
            ("content", &good("http://purl.org/dc/terms/name")),
            ("as", "application/json"),
        ],
        &writer(&[]),
    )
    .unwrap();
    let c: ikigai_nl::Check = serde_json::from_slice(&rep.bytes).unwrap();
    assert!(c.valid, "{c:#?}");
    assert!(c
        .warnings
        .iter()
        .any(|w| w.contains("as far as the grounding shows")));
}
