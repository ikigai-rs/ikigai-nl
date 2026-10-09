//! The acceptance: two graphs, the vocabulary, a few published scripts and different
//! callers. Every refusal or absence here is paired with the positive case that proves
//! the path works, because an empty part is a legitimate answer and a grounding that has
//! quietly stopped reading something is otherwise indistinguishable from one with nothing
//! to read.

mod common;

use common::*;
use ikigai_core::{Capability, Error, Expiry, Verb};
use ikigai_nl::{Examples, SpaceConfig};

fn action<'a>(g: &'a ikigai_nl::Grounding, iri: &str, verb: &str) -> Option<&'a ikigai_nl::Action> {
    g.actions
        .items
        .iter()
        .find(|a| a.endpoint.as_deref() == Some(iri) && a.verb == verb)
}

#[test]
fn two_callers_get_different_groundings_each_with_only_what_it_may_read() {
    let kernel = host(SpaceConfig::new());
    let a = grounding(&kernel, &alice(), &[]);
    let b = grounding(&kernel, &bob(), &[]);

    let graphs = |g: &ikigai_nl::Grounding| -> Vec<String> {
        g.graphs.items.iter().map(|s| s.graph.clone()).collect()
    };
    assert_eq!(graphs(&a), vec![LEDGER]);
    assert_eq!(graphs(&b), vec![BOOKS]);
    assert_ne!(a.identity, b.identity);

    let names = |g: &ikigai_nl::Grounding| -> Vec<String> {
        g.examples.items.iter().map(|e| e.name.clone()).collect()
    };
    // Alice reads public scripts; Bob holds one private script's grant. The draft is in
    // neither: only published scripts are examples.
    assert_eq!(names(&a), vec!["stale-urgent"]);
    assert_eq!(names(&b), vec!["book-titles"]);

    // Not even the name: neither face of Alice's grounding mentions Bob's graph or script,
    // and the reverse.
    for face in [
        serde_json::to_string(&a).unwrap(),
        turtle(&kernel, &alice(), &[]),
    ] {
        assert!(!face.contains(BOOKS), "{face}");
        assert!(!face.contains("book-titles"));
        assert!(!face.contains("half-done"));
    }
    for face in [
        serde_json::to_string(&b).unwrap(),
        turtle(&kernel, &bob(), &[]),
    ] {
        assert!(!face.contains(LEDGER), "{face}");
        assert!(!face.contains("stale-urgent"));
    }
}

#[test]
fn an_anonymous_caller_gets_an_anonymous_grounding() {
    let kernel = host(SpaceConfig::new());
    let g = grounding(&kernel, &anonymous(), &[]);
    assert!(g.graphs.items.is_empty());
    assert_eq!(g.graphs.of, 0);
    assert!(g.graphs.note.as_deref().unwrap().contains("not offered"));
    assert!(g.examples.items.is_empty());
    // …but not an empty one: what anyone may know is there.
    assert!(g.vocabulary.shown > 100, "{}", g.vocabulary.shown);
    assert!(action(&g, "urn:nl:grounding", "source").is_some());
    assert!(action(&g, "urn:iki:store:graphs", "source").is_none());
    let face = turtle(&kernel, &anonymous(), &[]);
    assert!(!face.contains(LEDGER) && !face.contains(BOOKS));
}

#[test]
fn the_actions_are_the_manifold_under_the_callers_capability_with_their_arguments() {
    let kernel = host(SpaceConfig::new());
    let a = grounding(&kernel, &alice(), &[]);
    let select =
        action(&a, "urn:iki:store:graph-select", "source").expect("alice may run a scoped query");
    assert_eq!(select.requires, vec!["urn:cap:store:read:graph:*"]);
    let inputs: Vec<_> = select.inputs.iter().map(|i| i.name.as_str()).collect();
    assert!(
        inputs.contains(&"graph") && inputs.contains(&"query"),
        "{inputs:?}"
    );
    assert!(
        select
            .inputs
            .iter()
            .find(|i| i.name == "query")
            .unwrap()
            .required
    );
    // Writing is not hers to do, so it is not in her grounding…
    assert!(action(&a, "urn:iki:store:update", "sink").is_none());
    assert!(action(&a, "urn:iki:store:graph-update", "sink").is_none());
    // …and it is in root's.
    let root = grounding(&kernel, &Capability::root(), &[]);
    assert!(action(&root, "urn:iki:store:update", "sink").is_some());
    // A template binding names its pattern, and its variable is a binding input.
    let script = a
        .actions
        .items
        .iter()
        .find(|x| x.template.as_deref() == Some("urn:script:{name}"))
        .expect("the script template is offered to a holder of a script read grant");
    assert!(script.inputs.iter().any(|i| i.name == "name" && i.binding));
}

#[test]
fn a_graphs_shape_is_its_totals_classes_predicates_and_samples() {
    let kernel = host(SpaceConfig::new());
    let a = grounding(&kernel, &alice(), &[]);
    let ledger = &a.graphs.items[0];
    assert_eq!(ledger.error, None);
    assert_eq!(ledger.triples, 6);
    assert_eq!(ledger.subjects, 3);
    assert_eq!(ledger.properties, 3);
    assert_eq!(ledger.classes, 1);
    assert_eq!(ledger.class_partitions.len(), 1);
    assert_eq!(ledger.class_partitions[0].iri, "urn:example:Item");
    assert_eq!(ledger.class_partitions[0].count, 3);
    let predicates: Vec<_> = ledger
        .property_partitions
        .iter()
        .map(|p| (p.iri.as_str(), p.count))
        .collect();
    assert_eq!(
        predicates,
        vec![
            ("http://www.w3.org/1999/02/22-rdf-syntax-ns#type", 3),
            ("http://purl.org/dc/terms/title", 2),
            ("https://ikigai-rs.dev/ns#summary", 1),
        ]
    );
    assert_eq!(ledger.samples.len(), 5);
    assert!(
        ledger.samples[0].starts_with("<urn:example:item:1> "),
        "{:?}",
        ledger.samples
    );
    // The store's default graph is no named graph's shape.
    assert!(!serde_json::to_string(&a)
        .unwrap()
        .contains("in the default graph"));
}

#[test]
fn bounds_say_first_n_of_m_and_never_truncate_silently() {
    let kernel = host(
        SpaceConfig::new()
            .samples(2)
            .max_partitions(1)
            .max_graphs(1),
    );
    let root = grounding(&kernel, &Capability::root(), &[]);
    // One graph of two, and the part says so.
    assert_eq!((root.graphs.shown, root.graphs.of), (1, 2));
    assert!(
        root.graphs
            .note
            .as_deref()
            .unwrap()
            .contains("first 1 of 2"),
        "{:?}",
        root.graphs.note
    );
    // Two samples of the graph's triples; one class and one predicate of each total.
    let shape = &root.graphs.items[0];
    assert_eq!(shape.graph, BOOKS, "graphs are summarized in IRI order");
    assert_eq!((shape.samples.len(), shape.triples), (2, 2));
    assert_eq!((shape.class_partitions.len(), shape.classes), (1, 1));
    assert_eq!((shape.property_partitions.len(), shape.properties), (1, 2));
    // The Turtle face states the same counts.
    let face = turtle(&kernel, &Capability::root(), &[]);
    assert!(face.contains("nl:samplesShown 2"), "{face}");
    assert!(face.contains("nl:propertiesShown 1"));
    assert!(face.contains("void:properties 2"));
    assert!(face.contains("nl:shown 1") && face.contains("nl:of 2"));
    // A focus reaches the graph past the bound.
    let focused = grounding(&kernel, &Capability::root(), &[("focus", LEDGER)]);
    assert_eq!(focused.graphs.items[0].graph, LEDGER);
}

#[test]
fn a_grounding_over_the_size_bound_is_refused_with_its_size() {
    let kernel = host(SpaceConfig::new().max_bytes(20_000));
    let refused = issue(&kernel, Verb::Source, "urn:nl:grounding", &[], &alice())
        .expect_err("the whole vocabulary alone is larger than 20 kB");
    let Error::InvalidArgument { name, detail } = refused else {
        panic!("refused with {refused:?}");
    };
    assert_eq!(name, "focus");
    assert!(
        detail.contains("over this host's bound of 20000"),
        "{detail}"
    );
    // Narrowed, it fits: the same caller, the same bound.
    let g = grounding(&kernel, &alice(), &[("focus", LEDGER)]);
    assert_eq!(g.graphs.items.len(), 1);
}

#[test]
fn a_focus_on_a_graph_keeps_its_shape_and_the_terms_it_is_written_in() {
    let kernel = host(SpaceConfig::new());
    let g = grounding(&kernel, &Capability::root(), &[("focus", LEDGER)]);
    assert_eq!(g.focus.as_deref(), Some(LEDGER));
    assert_eq!(g.graphs.items.len(), 1);
    assert_eq!(g.graphs.of, 2);
    let terms: Vec<_> = g.vocabulary.items.iter().map(|t| t.iri.as_str()).collect();
    assert!(
        terms.contains(&"https://ikigai-rs.dev/ns#summary"),
        "{terms:?}"
    );
    assert!(g.vocabulary.shown < g.vocabulary.of);
    assert!(g
        .vocabulary
        .note
        .as_deref()
        .unwrap()
        .contains("mention the focus"));
    // A focus on a graph the caller cannot read finds nothing, exactly as a focus on a
    // graph that does not exist: no oracle.
    let unreadable = grounding(&kernel, &alice(), &[("focus", BOOKS)]);
    let absent = grounding(&kernel, &alice(), &[("focus", "urn:example:nonesuch")]);
    assert!(unreadable.graphs.items.is_empty() && absent.graphs.items.is_empty());
    assert_eq!(unreadable.graphs.of, absent.graphs.of);
    // A topic word.
    let word = grounding(&kernel, &Capability::root(), &[("focus", "BOOK")]);
    assert_eq!(word.graphs.items.len(), 1);
    assert_eq!(word.graphs.items[0].graph, BOOKS);
    assert!(word.examples.items.iter().all(|e| e.name == "book-titles"));
}

#[test]
fn cached_per_capability_and_recomputed_after_a_write_to_a_graph_it_summarized() {
    // Named examples: every part of this grounding is cacheable.
    let kernel =
        host(SpaceConfig::new().examples(Examples::Named(vec!["stale-urgent".to_string()])));
    let cap = inspecting(&[
        format!("urn:cap:store:read:graph:{LEDGER}"),
        "urn:cap:script:read:public".to_string(),
    ]);
    let cached = |cap: &Capability| {
        text(
            &issue(
                &kernel,
                Verb::Source,
                "urn:kernel:cached",
                &[("target", "urn:nl:grounding")],
                cap,
            )
            .unwrap(),
        )
        .trim()
        .to_string()
    };
    let first = issue(&kernel, Verb::Source, "urn:nl:grounding", &[], &cap).unwrap();
    assert_eq!(first.expiry, Expiry::Never);
    assert_eq!(cached(&cap), "true");
    assert_eq!(
        text(&issue(&kernel, Verb::Source, "urn:nl:grounding", &[], &cap).unwrap()),
        text(&first)
    );
    // Per capability: another caller's read is not this one's cache entry.
    assert_eq!(cached(&inspecting(&[])), "false");

    let before = grounding(&kernel, &cap, &[]);
    issue(
        &kernel,
        Verb::Sink,
        "urn:iki:store:graph-update",
        &[
            ("graph", LEDGER),
            (
                "content",
                &format!("INSERT DATA {{ GRAPH <{LEDGER}> {{ <urn:example:item:4> a <urn:example:Item> }} }}"),
            ),
        ],
        &Capability::root(),
    )
    .expect("a write to the ledger graph");
    assert_eq!(cached(&cap), "false", "the write cut the grounding");
    let after = grounding(&kernel, &cap, &[]);
    assert_eq!(
        after.graphs.items[0].triples,
        before.graphs.items[0].triples + 1
    );
    assert_ne!(after.identity, before.identity);
    assert_ne!(after.graphs.identity, before.graphs.identity);
    // What did not change cites the same: the actions and vocabulary parts.
    assert_eq!(after.actions.identity, before.actions.identity);
    assert_eq!(after.vocabulary.identity, before.vocabulary.identity);
    // And reading again is cached again.
    issue(&kernel, Verb::Source, "urn:nl:grounding", &[], &cap).unwrap();
    assert_eq!(cached(&cap), "true");
}

#[test]
fn the_live_catalog_makes_the_grounding_live_and_the_kernel_says_why() {
    let kernel = host(SpaceConfig::new());
    let cap = inspecting(&["urn:cap:script:read:public".to_string()]);
    let rep = issue(&kernel, Verb::Source, "urn:nl:grounding", &[], &cap).unwrap();
    assert_eq!(rep.expiry, Expiry::Always);
    let why = text(&issue(&kernel, Verb::Source, "urn:kernel:uncached", &[], &cap).unwrap());
    let line = why
        .lines()
        .find(|l| l.contains("urn:nl:grounding"))
        .unwrap_or_else(|| panic!("{why}"));
    assert!(line.contains("urn:script:catalog"), "{why}");
    // Without a grant that reaches the catalog it is never read, and the grounding caches.
    let cap = inspecting(&[]);
    let rep = issue(&kernel, Verb::Source, "urn:nl:grounding", &[], &cap).unwrap();
    assert_eq!(rep.expiry, Expiry::Never);
}

#[test]
fn the_identity_is_the_content_so_the_same_view_cites_the_same() {
    let one = host(SpaceConfig::new());
    let two = host(SpaceConfig::new());
    let a = grounding(&one, &alice(), &[]);
    let b = grounding(&two, &alice(), &[]);
    assert_eq!(a.identity, b.identity);
    assert_eq!(a.iri, format!("urn:nl:grounding:{}", a.identity));
    assert!(a.identity.starts_with("sha256:") && a.identity.len() == 7 + 64);
    for part in [
        &a.actions.identity,
        &a.vocabulary.identity,
        &a.graphs.identity,
        &a.examples.identity,
    ] {
        assert!(part.starts_with("sha256:"));
    }
    assert_eq!(
        a.vocabulary.version.as_deref(),
        Some(ikigai_vocab_version().as_str())
    );
    // Each part names where it came from.
    assert_eq!(a.actions.source.as_deref(), Some("urn:kernel:actions"));
    assert_eq!(a.vocabulary.source.as_deref(), Some("urn:ikigai:vocab"));
    assert_eq!(a.graphs.source.as_deref(), Some("urn:iki:store:graphs"));
    assert_eq!(a.examples.source.as_deref(), Some("urn:script:catalog"));
    let example = &a.examples.items[0];
    assert!(example.version.starts_with("sha256:"));
    assert_eq!(
        example.version_iri.as_deref(),
        Some(format!("urn:script:stale-urgent:version:{}", example.version).as_str())
    );
}

/// The vocabulary's `owl:versionInfo`, read the same way a person would.
fn ikigai_vocab_version() -> String {
    let line = ikigai_vocab::VOCABULARY
        .lines()
        .find(|l| l.contains("owl:versionInfo \""))
        .unwrap();
    line.split('"').nth(1).unwrap().to_string()
}

#[test]
fn as_is_refused_outside_its_two_faces() {
    let kernel = host(SpaceConfig::new());
    let refused = issue(
        &kernel,
        Verb::Source,
        "urn:nl:grounding",
        &[("as", "text/html")],
        &alice(),
    )
    .expect_err("only Turtle and JSON");
    assert!(matches!(refused, Error::InvalidArgument { ref name, .. } if name == "as"));
}

#[test]
fn a_broad_reader_is_summarized_through_the_whole_dataset_door() {
    // `urn:cap:store:read` lists every graph, and the scoped door refuses it (that door
    // wants the per-graph grant), so each shape is read through `urn:iki:store:select`
    // with `GRAPH ?g` bound to one graph.
    let kernel = host(SpaceConfig::new());
    let g = grounding(&kernel, &Capability::scoped(["urn:cap:store:read"]), &[]);
    let shapes: Vec<_> = g
        .graphs
        .items
        .iter()
        .map(|s| (s.graph.as_str(), s.triples, s.error.is_none()))
        .collect();
    assert_eq!(shapes, vec![(BOOKS, 2, true), (LEDGER, 6, true)]);
    // The same numbers a per-graph reader gets.
    let a = grounding(&kernel, &alice(), &[]);
    assert_eq!(a.graphs.items[0].identity, g.graphs.items[1].identity);
}

#[test]
fn a_configured_example_the_caller_may_not_read_contributes_nothing_not_even_a_count() {
    let kernel = host(SpaceConfig::new().examples(Examples::Named(vec![
        "stale-urgent".to_string(),
        "book-titles".to_string(),
        "half-done".to_string(),
    ])));
    let a = grounding(&kernel, &alice(), &[]);
    assert_eq!((a.examples.shown, a.examples.of), (1, 1));
    assert_eq!(a.examples.items[0].name, "stale-urgent");
    let face = serde_json::to_string(&a).unwrap() + &turtle(&kernel, &alice(), &[]);
    assert!(!face.contains("book-titles") && !face.contains("half-done"));
    // Bob reads his private one; the draft is nobody's example.
    let b = grounding(&kernel, &bob(), &[]);
    let names: Vec<_> = b.examples.items.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["book-titles"]);
}
