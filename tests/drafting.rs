//! Drafting on a real gonk: the live demo of 2026-10-11 (devtools
//! `claude/research/nl-script-demo-2026-10-11.md`), reproduced with a STUB model that
//! answers exactly what Llama 3.3 70B answered there.
//!
//! - **Ledger #1242**: the ask "the number and title of every open item with priority 2 or
//!   higher" drafted `ledger:status "open"` where the status is the IRI `ledger:open`. The
//!   check passed it (it parses, every predicate exists), the preview was EMPTY, and the
//!   draft was saved with no repair. Two halves: an empty preview is a repair trigger, with
//!   the evidence (which triple patterns match nothing on their own), and the grounding
//!   states each predicate's value kind and, for a small closed set, its values. The
//!   draft's `FROM <g>` beside `GRAPH <g> { }` is HARMLESS here and stays in the corrected
//!   draft on purpose: the script host strips FROM and passes the graph set as both the
//!   default and the named graphs (the tutorial's `claims.rs` pinned it), so no check is
//!   built for it.
//! - **Ledger #1243**: drafting with no focus was refused, because the unfocused grounding
//!   carried every endpoint action (1,529 on gonk, about 100 KB per browse root) and went
//!   over the 1 MiB bound before any model was asked. And `urn:nl:sparql` said `graph=`
//!   where the grounding and the check say `focus=`, while its size refusal named `focus=`.

mod common;

use common::*;
use ikigai_core::{Capability, Error, Verb};
use ikigai_nl::{Draft, SpaceConfig, Status};

/// The gonk ledger's graph and vocabulary, as the demo's draft names them.
const GRAPH: &str = "urn:iki:ledger:graph:default";
const NS: &str = "https://ikigai-rs.dev/ns/ledger#";
const TITLE: &str = "http://purl.org/dc/terms/title";

/// The ask, exactly as the demo put it.
const ASK: &str = "the number and title of every open item with priority 2 or higher (0 is \
                   highest), ordered by number";

/// Four items: three open, one closed, so `ledger:status` is a small closed set of IRIs
/// that repeat, and `ledger:priority` a small closed set of integers.
fn seed_ledger(h: &Host) {
    let item = |n: u32, status: &str, priority: u32, title: &str| {
        format!(
            "<urn:iki:ledger:default:item:{n}> a <{NS}Item> ; <{NS}status> <{NS}{status}> ; \
             <{NS}priority> {priority} ; <{NS}number> {n} ; <{TITLE}> \"{title}\" ."
        )
    };
    let update = format!(
        "INSERT DATA {{ GRAPH <{GRAPH}> {{ {} {} {} {} }} }}",
        item(4, "open", 1, "No passkey list or revoke command"),
        item(
            5,
            "open",
            2,
            "Dependency floors never re-resolved at =floor"
        ),
        item(6, "closed", 0, "A closed item"),
        item(7, "open", 3, "A low-priority item"),
    );
    issue(
        &h.kernel,
        Verb::Sink,
        "urn:iki:store:update",
        &[("content", &update)],
        &Capability::root(),
    )
    .expect("seeding the ledger graph");
}

/// The demo's draft, verbatim but for whitespace: ONE error, the status as a string.
fn demo_draft(status: &str) -> String {
    format!(
        "# @param limit xsd:integer optional default 2\n\
         SELECT ?number ?title\n\
         FROM <{GRAPH}>\n\
         WHERE {{\n  \
           GRAPH <{GRAPH}> {{\n    \
             ?item a <{NS}Item> ;\n      \
               <{NS}status> {status} ;\n      \
               <{NS}priority> ?priority ;\n      \
               <{NS}number> ?number ;\n      \
               <{TITLE}> ?title .\n    \
             FILTER (?priority <= ?limit)\n  \
           }}\n\
         }}\n\
         ORDER BY ?number"
    )
}

/// What the demo's draft said.
fn broken() -> String {
    demo_draft("\"open\"")
}

/// The same draft with the status an IRI. FROM + GRAPH kept: harmless on ikigai.
fn fixed() -> String {
    demo_draft(&format!("<{NS}open>"))
}

/// A person who may read the ledger graph and save drafts named `demo-*`, with the
/// principal a host's door would mint.
fn person() -> Capability {
    Capability::scoped([
        format!("urn:cap:store:read:graph:{GRAPH}"),
        "urn:cap:script:write:demo-*".to_string(),
        "urn:cap:script:read:demo-*".to_string(),
    ])
    .with_principal("urn:test:person:brian")
    .expect("a principal a door can mint")
}

fn draft(h: &Host, cap: &Capability, args: &[(&str, &str)]) -> Result<Draft, Error> {
    let mut all = vec![("as", "application/json")];
    all.extend_from_slice(args);
    let rep = issue(&h.kernel, Verb::Sink, "urn:nl:sparql", &all, cap)?;
    Ok(serde_json::from_slice(&rep.bytes).expect("the JSON face parses into the model"))
}

// ------------------------------------------------------------------- ledger #1242 (a)

#[test]
fn an_empty_preview_is_repaired_with_the_patterns_that_match_nothing() {
    let h = drafting_host(SpaceConfig::new(), &[(ASK, &[&broken(), &fixed()])]);
    seed_ledger(&h);
    let d = draft(
        &h,
        &person(),
        &[("ask", ASK), ("focus", "ledger"), ("save", "demo-open-p2")],
    )
    .unwrap();

    // The demo's draft passes every mechanical check, and its preview is empty.
    let first = &d.attempts[0];
    assert!(first.empty_preview, "{d:#?}");
    // So it was sent back, and the repair was told WHICH pattern matches nothing alone:
    // the status as a string. The others each match something.
    assert_eq!(d.attempts.len(), 2, "{d:#?}");
    let calls = h.prompts();
    assert_eq!(calls.len(), 2);
    let repair = &calls[1].prompt;
    assert!(repair.contains(&broken()), "{repair}");
    assert!(repair.contains("preview is empty"), "{repair}");
    assert!(
        repair.contains(&format!("?item <{NS}status> \"open\"")),
        "{repair}"
    );
    assert!(
        !repair.contains(&format!("?item <{NS}number> ?number")),
        "{repair}"
    );

    // The corrected draft, FROM + GRAPH and all, has rows and is saved, not marked.
    assert_eq!(d.status, Status::Saved, "{d:#?}");
    assert!(!d.empty_preview);
    assert!(d.attempts[1].valid && !d.attempts[1].empty_preview);
    let preview = d.check.as_ref().unwrap().preview.as_ref().unwrap();
    assert_eq!(
        preview.rows,
        vec![
            vec![
                "\"4\"^^<http://www.w3.org/2001/XMLSchema#integer>".to_string(),
                "\"No passkey list or revoke command\"".to_string()
            ],
            vec![
                "\"5\"^^<http://www.w3.org/2001/XMLSchema#integer>".to_string(),
                "\"Dependency floors never re-resolved at =floor\"".to_string()
            ],
        ]
    );
    assert!(h.saved("demo-open-p2").unwrap().source.contains(&fixed()));
}

#[test]
fn a_draft_still_empty_after_the_last_attempt_is_saved_marked_never_silently() {
    // The model never learns: three times the demo's draft (the third is a repeat, which
    // stops drafting at the second).
    let h = drafting_host(SpaceConfig::new(), &[(ASK, &[&broken()])]);
    seed_ledger(&h);
    let d = draft(
        &h,
        &person(),
        &[("ask", ASK), ("focus", "ledger"), ("save", "demo-empty")],
    )
    .unwrap();
    assert_eq!(d.status, Status::Saved, "{d:#?}");
    assert!(d.empty_preview, "{d:#?}");
    assert!(
        d.note.as_deref().unwrap().contains("preview is EMPTY"),
        "{d:#?}"
    );
    let saved = h.saved("demo-empty").unwrap();
    assert!(saved.source.contains(&broken()));

    // Marked in the provenance: the attempt the script derives from carries the warning.
    let block = ikigai_nl::prov_of(&saved.source).unwrap();
    assert!(block.contains(ikigai_nl::EMPTY_PREVIEW), "{block}");
    // And in the plain answer, on its first line.
    let h = drafting_host(SpaceConfig::new(), &[(ASK, &[&broken()])]);
    seed_ledger(&h);
    let plain = text(
        &issue(
            &h.kernel,
            Verb::Sink,
            "urn:nl:sparql",
            &[("ask", ASK), ("focus", "ledger"), ("save", "demo-empty")],
            &person(),
        )
        .unwrap(),
    );
    let first = plain.lines().next().unwrap();
    assert!(first.contains("preview is EMPTY"), "{plain}");
}

#[test]
fn the_check_says_which_patterns_match_nothing_alone() {
    let h = drafting_host(SpaceConfig::new(), &[]);
    seed_ledger(&h);
    let rep = issue(
        &h.kernel,
        Verb::Source,
        "urn:nl:sparql:check",
        &[
            ("content", &broken()),
            ("focus", "ledger"),
            ("as", "application/json"),
        ],
        &person(),
    )
    .unwrap();
    let c: ikigai_nl::Check = serde_json::from_slice(&rep.bytes).unwrap();
    assert!(c.valid, "an empty answer is not a malformed query: {c:#?}");
    assert!(c.preview.as_ref().unwrap().is_empty());
    assert_eq!(
        c.unmatched,
        vec![format!("GRAPH <{GRAPH}> {{ ?item <{NS}status> \"open\" }}")],
        "{c:#?}"
    );
    // A query whose patterns each match, combined so nothing does, says so instead.
    let none = format!(
        "SELECT ?item WHERE {{ GRAPH <{GRAPH}> {{ ?item <{NS}status> <{NS}closed> ; \
         <{NS}priority> 3 }} }}"
    );
    let rep = issue(
        &h.kernel,
        Verb::Source,
        "urn:nl:sparql:check",
        &[("content", &none), ("as", "application/json")],
        &person(),
    )
    .unwrap();
    let c: ikigai_nl::Check = serde_json::from_slice(&rep.bytes).unwrap();
    assert!(c.unmatched.is_empty(), "{c:#?}");
    assert!(
        c.warnings
            .iter()
            .any(|w| w.contains("matches something on its own")),
        "{c:#?}"
    );
}

// ------------------------------------------------------------------- ledger #1242 (b)

#[test]
fn the_grounding_states_each_predicates_value_kind_and_a_small_closed_set() {
    let h = drafting_host(SpaceConfig::new(), &[(ASK, &[&fixed()])]);
    seed_ledger(&h);
    let g = grounding(&h.kernel, &person(), &[("focus", "ledger")]);
    let shape = g.graphs.items.iter().find(|s| s.graph == GRAPH).unwrap();
    let partition = |p: &str| {
        shape
            .property_partitions
            .iter()
            .find(|q| q.iri == p)
            .unwrap_or_else(|| panic!("{p}: {shape:#?}"))
    };
    let status = partition(&format!("{NS}status"));
    assert_eq!(status.kinds.len(), 1, "{status:#?}");
    assert_eq!(status.kinds[0].kind, "iri");
    assert_eq!(
        status.values,
        vec![format!("<{NS}open>"), format!("<{NS}closed>")],
        "most used first"
    );
    let number = partition(&format!("{NS}number"));
    assert_eq!(number.kinds[0].kind, "literal");
    assert_eq!(
        number.kinds[0].datatype.as_deref(),
        Some("http://www.w3.org/2001/XMLSchema#integer")
    );
    assert!(
        number.values.is_empty(),
        "every number is distinct: {number:#?}"
    );
    let title = partition(TITLE);
    assert_eq!(title.kinds[0].kind, "literal");
    assert!(title.values.is_empty());

    // The drafter is told, in the prompt it drafts from.
    draft(
        &h,
        &person(),
        &[("ask", ASK), ("focus", "ledger"), ("save", "demo-kinds")],
    )
    .unwrap();
    let prompt = &h.prompts()[0].prompt;
    assert!(
        prompt.contains(&format!(
            "<{NS}status> (4): IRI, one of <{NS}open> <{NS}closed>"
        )),
        "{prompt}"
    );
    assert!(
        prompt.contains(&format!("<{NS}number> (4): literal xsd:integer")),
        "{prompt}"
    );
}

// ---------------------------------------------------------------------- ledger #1243

#[test]
fn an_unfocused_ask_fits_because_the_drafter_leaves_the_actions_out() {
    // A host bound just under the full unfocused grounding, as gonk's 1 MiB was under its
    // 5 MB: the full grounding is refused, as it should be.
    let probe = host(SpaceConfig::new());
    let full = issue(
        &probe,
        Verb::Source,
        "urn:nl:grounding",
        &[("as", "application/json")],
        &person(),
    )
    .unwrap()
    .bytes
    .len();
    let config = SpaceConfig::new().max_bytes(full - 1);
    let h = drafting_host(config, &[(ASK, &[&fixed()])]);
    seed_ledger(&h);
    let refused = issue(
        &h.kernel,
        Verb::Source,
        "urn:nl:grounding",
        &[("as", "application/json")],
        &person(),
    )
    .unwrap_err();
    assert!(
        matches!(&refused, Error::InvalidArgument { name, .. } if name == "focus"),
        "{refused}"
    );

    // The drafter needs graphs, vocabulary and examples, not the actions, so with no
    // focus at all it drafts.
    let d = draft(&h, &person(), &[("ask", ASK), ("save", "demo-unfocused")]).unwrap();
    assert_eq!(d.status, Status::Saved, "{d:#?}");
    // And the grounding it cites holds no action: the part says why.
    let g = grounding(
        &h.kernel,
        &person(),
        &[("parts", "vocabulary graphs examples")],
    );
    assert_eq!(g.iri, d.grounding);
    assert!(g.actions.items.is_empty());
    assert!(g.actions.of > 0, "the actions offered are still counted");
    assert!(
        g.actions.note.as_deref().unwrap().contains("parts="),
        "{:?}",
        g.actions.note
    );
}

#[test]
fn focus_is_the_drafters_argument_and_graph_its_alias_for_one_release() {
    let h = drafting_host(SpaceConfig::new(), &[(ASK, &[&fixed()])]);
    seed_ledger(&h);
    let by_focus = draft(
        &h,
        &person(),
        &[("ask", ASK), ("focus", "ledger"), ("save", "demo-focus")],
    )
    .unwrap();
    let by_graph = draft(
        &h,
        &person(),
        &[("ask", ASK), ("graph", "ledger"), ("save", "demo-graph")],
    )
    .unwrap();
    assert_eq!(by_focus.grounding, by_graph.grounding);
    // Both, disagreeing: refused, never one silently chosen.
    let both = draft(
        &h,
        &person(),
        &[("ask", ASK), ("focus", "ledger"), ("graph", "books")],
    )
    .unwrap_err();
    assert!(
        matches!(&both, Error::InvalidArgument { name, .. } if name == "graph"),
        "{both}"
    );

    // The size refusal names the argument the caller used.
    let h = drafting_host(SpaceConfig::new().max_bytes(64), &[(ASK, &[&fixed()])]);
    for arg in ["focus", "graph"] {
        let err = draft(&h, &person(), &[("ask", ASK), (arg, "ledger")]).unwrap_err();
        match &err {
            Error::InvalidArgument { name, detail } => {
                assert_eq!(name, arg, "{err}");
                assert!(detail.contains(&format!("{arg}=")), "{detail}");
                let other = if arg == "focus" { "graph=" } else { "focus=" };
                assert!(!detail.contains(other), "{detail}");
            }
            other => panic!("{other}"),
        }
    }
    // With neither, the refusal teaches the one name.
    let err = draft(&h, &person(), &[("ask", ASK)]).unwrap_err();
    assert!(
        matches!(&err, Error::InvalidArgument { name, detail } if name == "focus" && detail.contains("focus=")),
        "{err}"
    );
}

// ------------------------------------------------------------------- ledger #1298

/// One drafting's three faces, each from a fresh host (the stub's script is per host).
fn faces(answers: &[&str], save: &str) -> (serde_json::Value, String, String) {
    let face = |want: &str| {
        let h = drafting_host(SpaceConfig::new(), &[(ASK, answers)]);
        seed_ledger(&h);
        text(
            &issue(
                &h.kernel,
                Verb::Sink,
                "urn:nl:sparql",
                &[
                    ("ask", ASK),
                    ("focus", "ledger"),
                    ("save", save),
                    ("as", want),
                ],
                &person(),
            )
            .unwrap(),
        )
    };
    let json = serde_json::from_str(&face("application/json")).unwrap();
    (json, face("text/plain"), face("text/turtle"))
}

#[test]
fn a_repeat_says_drafting_stopped_and_is_recorded_as_a_repeat_not_a_failure() {
    // The model gives the demo's draft, then the same draft again: two asks of a bound of 3.
    let (json, plain, turtle) = faces(&[&broken()], "demo-repeat");
    let attempts = json["attempts"].as_array().unwrap();
    assert_eq!(attempts.len(), 2, "{json:#}");
    assert_eq!(json["status"], "saved", "{json:#}");

    // What happened, said in so many words: no "within the bound of 3" after 2 asks.
    let note = json["note"].as_str().unwrap();
    assert!(
        note.contains("drafting stopped: the model repeated attempt 1"),
        "{note}"
    );
    assert!(!note.contains("within the bound"), "{note}");
    let first = plain.lines().next().unwrap();
    assert!(
        first.contains("drafting stopped: the model repeated attempt 1"),
        "{plain}"
    );
    assert!(!first.contains("within the bound"), "{plain}");

    // The repeat is a REPEAT, a field of its own; the attempt it repeats is not one.
    assert_eq!(attempts[1]["repeatOf"], 1, "{json:#}");
    assert!(attempts[0].get("repeatOf").is_none(), "{json:#}");
    assert_eq!(attempts[1]["valid"], false, "it was not a new valid draft");

    // And the provenance says so: the repeat is an alternate of the attempt it repeats.
    let a = json["activity"].as_str().unwrap();
    let alternate =
        format!("<{a}:attempt:2> <http://www.w3.org/ns/prov#alternateOf> <{a}:attempt:1> .");
    assert!(turtle.contains(&alternate), "{turtle}");
    assert!(json["prov"].as_str().unwrap().contains(&alternate));
}

#[test]
fn drafting_that_runs_to_the_bound_still_says_the_bound() {
    // Three different empty drafts: no repeat, so the bound is what stopped drafting.
    let other =
        |n: u32| broken().replace("ORDER BY ?number", &format!("ORDER BY ?number LIMIT {n}"));
    let (json, plain, _) = faces(&[&other(10), &other(11), &other(12)], "demo-bound");
    assert_eq!(json["attempts"].as_array().unwrap().len(), 3, "{json:#}");
    let note = json["note"].as_str().unwrap();
    assert!(
        note.contains("no attempt within the bound of 3 did better"),
        "{note}"
    );
    assert!(!note.contains("repeated"), "{note}");
    assert!(json["attempts"]
        .as_array()
        .unwrap()
        .iter()
        .all(|a| a.get("repeatOf").is_none()));
    assert!(
        plain
            .lines()
            .next()
            .unwrap()
            .contains("within the bound of 3"),
        "{plain}"
    );
}
