//! Caller SPARQL that this crate causes to run is bounded in TIME by the store's doors
//! (ledger #1344).
//!
//! The claim: ikigai-nl builds its own SPARQL evaluator, outside the store's doors, so a
//! caller-influenced query (a model's draft, a draft handed to `urn:nl:sparql:check`) is
//! bounded by stack only, never by time, and should run under
//! `ikigai_store::EvaluationBudget::run`.
//!
//! What holds, and what this file pins:
//!
//! - **The library evaluates nothing itself.** Its normal dependency graph has no evaluator
//!   in it (`cargo tree -e normal --target all -i oxigraph` and `-i spareval` print nothing),
//!   and a draft is only PARSED here (spargebra, through `ikigai_script::sparql`). So there is
//!   no evaluation of this crate's own to wrap in an `EvaluationBudget`.
//! - **Every evaluation it causes is a request to the store's doors**, under the caller's
//!   capability: the dry run of `urn:nl:sparql:check` (and so of every drafting attempt in
//!   `urn:nl:sparql`), the probes of an empty preview, and the grounding's fixed queries. Those
//!   doors run each evaluation within the store's time budget (store 0.2.8, ledger #964): the
//!   budget the caller's capability chooses, answered with a typed `Timeout` at it.
//! - **The timeout arrives as a VALUE**: the check lists it as an error of the draft ("the dry
//!   run failed: …"), so a drafting attempt that timed out is fed back to the model like any
//!   other finding, and a refusal is not what the caller is answered with.
//!
//! The draft below is the store's own pathological shape (`ikigai-store`'s
//! `tests/match_time_budget.rs`): eight unconstrained triple patterns, a cross product of
//! |graph|^8 rows, under one FILTER over every variable that never holds, so neither the
//! preview's LIMIT nor an early answer can cut it short. Over the six triples of the test
//! ledger that is about 1.7 million rows. Measured through this same check in a debug build
//! with the store's budget raised to 120 s, k = 5, 6 and 7 patterns held the caller 87 ms,
//! 518 ms and 6.5 s, so k = 8 is a minute or more; with the budget at 200 ms, k = 7 and k = 8
//! were each answered in about 250 ms, with the store's timeout.

mod common;

use std::time::{Duration, Instant};

use common::*;
use ikigai_core::{Capability, Verb};
use ikigai_nl::{Check, Draft, SpaceConfig, Status};
use ikigai_store::budget::TimeBudget;
use ikigai_store::DurableStore;

/// The store's time budget in these tests: small, so a test answered at it is fast.
const BUDGET: Duration = Duration::from_millis(200);

/// The store's own timeout wording, with this budget in it: proof the answer is the door's
/// budget and not some other failure.
fn timed_out(message: &str) -> bool {
    message.contains(&format!(
        "ran past its time budget of {} ms",
        BUDGET.as_millis()
    ))
}

/// A store as a host builds one, with its time budget set.
fn budgeted_store() -> DurableStore {
    DurableStore::in_memory()
        .expect("an in-memory store")
        .with_time_budget(TimeBudget::new(BUDGET))
}

/// `k` unconstrained triple patterns over LEDGER and one FILTER over all their variables.
fn cross_join(k: usize) -> String {
    let patterns: String = (0..k).map(|i| format!("?s{i} ?p{i} ?o{i} . ")).collect();
    let all: Vec<String> = (0..k)
        .map(|i| format!("STR(?s{i}), STR(?p{i}), STR(?o{i})"))
        .collect();
    format!(
        "SELECT ?s0 WHERE {{ GRAPH <{LEDGER}> {{ {patterns}}} FILTER(CONCAT({}) = \"never\") }}",
        all.join(", ")
    )
}

/// Alice, who may read LEDGER and write her own draft names. ⚠ SCOPED on purpose: the store
/// gives ROOT its budget ceiling (120 s), not the base, so under root the 200 ms budget does
/// not apply and the draft simply runs.
fn writer(name: &str) -> Capability {
    Capability::scoped([
        format!("urn:cap:store:read:graph:{LEDGER}"),
        "urn:cap:script:read:public".to_string(),
        format!("urn:cap:script:write:{name}"),
        format!("urn:cap:script:read:{name}"),
    ])
    .with_principal("urn:test:person:alice")
    .expect("a principal a door can mint")
}

/// How long the caller may wait in all: the grounding, the check, and one answer at the
/// budget, with room for a loaded CI machine. Far below what the evaluation itself costs.
const ANSWERED_WITHIN: Duration = Duration::from_secs(5);

#[test]
fn a_pathological_draft_is_answered_at_the_stores_budget_as_a_check_error() {
    let h = drafting_host_over(SpaceConfig::new(), budgeted_store(), &[]);
    let start = Instant::now();
    let rep = issue(
        &h.kernel,
        Verb::Source,
        "urn:nl:sparql:check",
        &[("content", &cross_join(8)), ("as", "application/json")],
        &writer("slow"),
    )
    .expect("the check answers; a timed-out dry run is a finding, not a refusal");
    let elapsed = start.elapsed();
    let c: Check = serde_json::from_slice(&rep.bytes).expect("the check's JSON face");

    assert!(!c.valid, "{c:#?}");
    assert!(
        c.errors
            .iter()
            .any(|e| e.starts_with("the dry run failed: timeout: ") && timed_out(e)),
        "the dry run must be stopped by the store's budget, typed as a timeout: {:#?}",
        c.errors
    );
    assert!(c.preview.is_none(), "{c:#?}");
    assert!(
        elapsed < ANSWERED_WITHIN,
        "the caller waited {elapsed:?}; the store's budget is {BUDGET:?}"
    );
}

#[test]
fn a_drafting_attempt_that_times_out_is_fed_back_and_bounded() {
    let slow = cross_join(8);
    let h = drafting_host_over(
        SpaceConfig::new(),
        budgeted_store(),
        &[("everything at once", &[slow.as_str()])],
    );
    let start = Instant::now();
    let rep = issue(
        &h.kernel,
        Verb::Sink,
        "urn:nl:sparql",
        &[
            ("ask", "everything at once"),
            ("save", "slow"),
            ("as", "application/json"),
        ],
        &writer("slow"),
    )
    .expect("drafting answers");
    let elapsed = start.elapsed();
    let d: Draft = serde_json::from_slice(&rep.bytes).expect("the draft's JSON face");

    assert_ne!(d.status, Status::Saved, "{d:#?}");
    let first = d.attempts.first().expect("at least one attempt");
    assert!(!first.valid, "{d:#?}");
    assert!(
        first.errors.iter().any(|e| timed_out(e)),
        "the first attempt carries the store's timeout as a finding: {:#?}",
        first.errors
    );
    // Each attempt's dry run is answered at the budget, and the attempts are the host's
    // bound, so the whole draft is bounded by them.
    let attempts = u32::try_from(d.attempts.len()).expect("a handful of attempts");
    assert!(
        elapsed < ANSWERED_WITHIN * attempts.max(1),
        "drafting took {elapsed:?} over {attempts} attempt(s); the store's budget is {BUDGET:?}"
    );
}
