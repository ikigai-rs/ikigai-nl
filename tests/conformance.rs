//! The module recipe as one test: `ikigai-conformance` walks `urn:nl:grounding` in a host
//! that has everything it composes, so the walk (under root) sees every part filled: the
//! actions, the vocabulary, both graphs' shapes and an example.
//!
//! # What the suite is told, and why
//!
//! - **Body probes share the kernel's root space** so BODIES compares the actual
//!   endpoints. No module argument declares `reads_body`, so BODY-INPUTS has no inputs
//!   to compare. `nothing_unprobed` keeps deterministic coverage from silently falling off.
//!   Grounding's BODIES waiver covers the suite's root-to-scoped capability change;
//!   `grounding_body_budget_preserves_the_callers_authority` pins the actual budget contract.
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
//!   (`an_anonymous_caller_…`, `h.saved_names()` empty).
//! - **`ikigai_nl::space(config)` is HOST-named** (SPACE-NAME): its doors are built from
//!   the `SpaceConfig` it is handed (the examples, the model doors, the bounds), so only the
//!   host knows which instance it is, and the space claims no name itself. The suite holds
//!   it to that: `id()` is `None` and the topology root is anonymous. Naming it would be a
//!   new public behavior and a wrong claim (one name over doors that vary by config).
//! - **The walk is scoped to this crate's own endpoints** (`Suite::own("nl-*")`, conformance
//!   0.8.0): the store, the vocabulary and the script host are here only because the
//!   grounding composes them, and each is conformed by its own crate's suite; the stub model
//!   is a test double. They are INHERITED, so no check reads them and no probe fires them, and
//!   `the_walk_reaches_…` pins that list exactly: an endpoint this crate gains outside `nl-*`
//!   turns it red instead of joining the pile unseen. (Before 0.8.0 this was a hand partition,
//!   one `opt_out` per composed id, which still ran the static checks over them.)
//! - **REQUIRED is not vacuous here**: `nl-sparql-check` declares `content` required (by value,
//!   no default), so the walk drops it and expects a typed refusal; `a_check_without_…` pins
//!   the refusal itself. `nl-prompt`'s `name` is a binding and `nl-grounding` requires
//!   nothing, so they give the check nothing to drop. `Suite::required_on_mutating()` is NOT
//!   declared: `nl-sparql`, the only mutating action, has no required input (an ask arrives
//!   by name OR piped, so neither is), and the declaration would reach nothing.

mod common;

use common::*;
use ikigai_conformance::{Check, Expected, Fixture, Suite};
use ikigai_core::Verb;
use ikigai_nl::{Examples, SpaceConfig};

/// Every endpoint the test host binds beside this crate's own: the vocabulary, the script
/// host's space, the store's space and the stub model.
const INHERITED: [&str; 26] = [
    "ikigai-vocab",
    "llm-stub-ask",
    "llm-stub-select",
    "script",
    "script-catalog",
    "script-compiled",
    "script-eval",
    "script-result",
    "script-run",
    "script-runs",
    "script-version",
    "store-ask",
    "store-construct",
    "store-describe",
    "store-dump",
    "store-graph-ask",
    "store-graph-construct",
    "store-graph-describe",
    "store-graph-info",
    "store-graph-select",
    "store-graph-update",
    "store-graphs",
    "store-info",
    "store-load",
    "store-select",
    "store-update",
];

fn config() -> SpaceConfig {
    SpaceConfig::new().examples(Examples::Named(vec!["stale-urgent".to_string()]))
}

fn suite(space: std::sync::Arc<dyn ikigai_core::Space>) -> Suite {
    Suite::new()
        .body_space(space)
        .own("nl-*")
        .host_named_space("ikigai_nl::space(config)", ikigai_nl::space(config()))
        .cacheable("nl-grounding")
        .opt_out_check(
            "nl-grounding",
            Check::Bodies,
            "conformance 0.12.0 replaces root with scoped declared requirements plus a memory \
             grant for the bounded call; grounding is authority-dependent, so that caller \
             legitimately receives a smaller complete answer, not truncation. \
             grounding_body_budget_preserves_the_callers_authority proves complete scoped \
             bytes and OverBudget under the original root caller",
        )
        .pure("nl-prompt")
        .fixture(Fixture::new("nl-sparql", Verb::Sink).arg("content", "the titles of the items"))
        .opt_out_check(
            "nl-sparql",
            Check::Authority,
            "its one write is the Sink on urn:script:{name} under the caller's own \
             capability, which enforces urn:cap:script:write:{name}; under no grants it \
             answers an unsaved draft and writes nothing (tests/sparql.rs pins it)",
        )
        .namespace(ikigai_nl::VOID)
}

fn kernel() -> ikigai_core::Kernel {
    host(config())
}

#[test]
fn conforms() {
    let h = drafting_host(config(), &[]);
    let report = suite(h.space).run_blocking(&h.kernel);
    println!("{report}");
    report.assert_clean_with(&Expected::new().nothing_unprobed());
}

/// ★ The positive half: a clean report over a walk that reached nothing would read exactly
/// like a clean report over a walk that reached everything.
#[test]
fn the_walk_reaches_the_grounding_and_probes_its_turtle() {
    let h = drafting_host(config(), &[]);
    let report = suite(h.space).run_blocking(&h.kernel);
    let mut walked = report.walked.to_vec();
    walked.sort();
    assert_eq!(
        walked,
        ["nl-grounding", "nl-prompt", "nl-sparql", "nl-sparql-check"],
        "{report}"
    );
    // The rest, EXACTLY: what `own` leaves out is named, so a new endpoint outside `nl-*`
    // (or a composed one that stops being bound) turns this red.
    let mut inherited = report.declared.inherited.clone();
    inherited.sort();
    assert_eq!(inherited, INHERITED, "{report}");
    let text = report.to_string();
    assert!(
        text.lines()
            .any(|l| l == "space: ikigai_nl::space(config) host-named"),
        "{report}"
    );
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

/// What REQUIRED asks of `nl-sparql-check`, pinned where it can be read: the query is its one
/// required input, and a check without it is refused as a missing argument, typed, never
/// answered as a check of nothing.
#[test]
fn a_check_without_its_query_is_refused_as_a_missing_argument() {
    let refused = issue(
        &kernel(),
        Verb::Source,
        "urn:nl:sparql:check",
        &[],
        &ikigai_core::Capability::root(),
    )
    .unwrap_err();
    assert!(
        matches!(refused, ikigai_core::Error::MissingArgument(ref n) if n == "content"),
        "{refused}"
    );
}

/// BODIES 0.12.0 changes root into a scoped caller for its bounded request. Grounding
/// intentionally changes with authority; prove that its shorter answer is complete,
/// while a smaller budget under the original caller really does fail.
#[test]
fn grounding_body_budget_preserves_the_callers_authority() {
    use futures::executor::block_on;
    use ikigai_core::{cap_stream_memory, Capability, Error, Iri, IssueContext, Request};

    let kernel = kernel();
    let request = Request::new(Verb::Source, Iri::parse("urn:nl:grounding").unwrap());
    let root = Capability::root();
    let full = block_on(kernel.issue(request.clone(), &root)).unwrap();
    let bound = full.bytes.len() - 1;
    kernel.cut("urn:nl:grounding");
    let limited = block_on(kernel.issue_body_with_context(
        request.clone(),
        &root,
        IssueContext::new().with_memory_limit(bound as u64),
    ));
    let collected = block_on(async { limited?.collect(bound).await });
    assert!(
        matches!(collected, Err(Error::OverBudget { .. })),
        "{collected:?}"
    );

    let scoped = Capability::scoped([cap_stream_memory(bound as u64)]);
    let complete = block_on(kernel.issue(request.clone(), &scoped)).unwrap();
    kernel.cut("urn:nl:grounding");
    let bounded = block_on(async {
        kernel
            .issue_body_with_context(
                request,
                &scoped,
                IssueContext::new().with_memory_limit(bound as u64),
            )
            .await?
            .collect(bound)
            .await
    })
    .unwrap();
    println!(
        "root: {} bytes; scoped complete: {} bytes; scoped bounded: {} bytes",
        full.bytes.len(),
        complete.bytes.len(),
        bounded.bytes.len()
    );
    assert!(complete.bytes.len() < bound);
    assert_eq!(complete.bytes, bounded.bytes);
    assert_eq!(complete.repr_type, bounded.repr_type);
}
