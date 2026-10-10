//! `focus=`: narrowing a grounding to a graph or a topic.
//!
//! One rule, applied to every part: an item stays when any of its text mentions the focus,
//! case-insensitively. A graph also stays when a class or predicate it uses mentions it,
//! and a vocabulary term also stays when a kept graph uses it, so a focus on one graph
//! keeps that graph's shape and the `ik:` terms it is written in.

use std::collections::BTreeSet;

use crate::model::{Action, Example, GraphShape, Term};

/// Whether any of `texts` mentions `focus`, ignoring case.
pub(crate) fn mentions<'a>(focus: &str, texts: impl IntoIterator<Item = &'a str>) -> bool {
    let focus = focus.to_lowercase();
    texts
        .into_iter()
        .any(|text| text.to_lowercase().contains(&focus))
}

pub(crate) fn action(focus: &str, action: &Action) -> bool {
    let mut texts = vec![
        action.match_iri.as_str(),
        action.contract.as_str(),
        action.id.as_str(),
        action.title.as_str(),
        action.summary.as_str(),
        action.action_summary.as_str(),
    ];
    texts.extend(action.endpoint.as_deref());
    texts.extend(action.template.as_deref());
    texts.extend(action.inputs.iter().map(|i| i.name.as_str()));
    mentions(focus, texts)
}

pub(crate) fn graph(focus: &str, shape: &GraphShape) -> bool {
    let partitions = shape
        .class_partitions
        .iter()
        .chain(&shape.property_partitions)
        .map(|p| p.iri.as_str());
    mentions(
        focus,
        std::iter::once(shape.graph.as_str()).chain(partitions),
    )
}

/// The IRIs of every class and predicate the kept graphs use.
pub(crate) fn used(shapes: &[GraphShape]) -> BTreeSet<&str> {
    shapes
        .iter()
        .flat_map(|s| s.class_partitions.iter().chain(&s.property_partitions))
        .map(|p| p.iri.as_str())
        .collect()
}

pub(crate) fn term(focus: &str, term: &Term, used: &BTreeSet<&str>) -> bool {
    used.contains(term.iri.as_str())
        || mentions(
            focus,
            [term.iri.as_str()]
                .into_iter()
                .chain(term.label.as_deref())
                .chain(term.comment.as_deref()),
        )
}

pub(crate) fn example(focus: &str, example: &Example) -> bool {
    mentions(
        focus,
        [
            example.iri.as_str(),
            example.name.as_str(),
            example.source.as_str(),
        ],
    )
}
