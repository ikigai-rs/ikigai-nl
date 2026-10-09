//! `urn:nl:prompt:{name}`: the drafting prompts, as resources.
//!
//! A prompt is a template the drafter SOURCES through the kernel and fills, so it is
//! readable by anyone who wants to know what a model was told, citable by its digest in a
//! draft's provenance, and replaceable by a host that binds the same name in front of this
//! crate's space. The filling is one pass over the template: a value is inserted as it is
//! and never scanned for placeholders, so an ask that says `{{grounding}}` stays text.

use async_trait::async_trait;
use ikigai_core::{
    ActionSpec, ArgSpec, Description, Endpoint, Error, Invocation, ReprType, Representation,
    Result, Verb,
};

use crate::model::{Grounding, Term};

/// The prompt-template family: `urn:nl:prompt:{name}`.
pub const PROMPT_TEMPLATE: &str = "urn:nl:prompt:{name}";

/// The first draft's prompt: `urn:nl:prompt:sparql`.
pub const SPARQL_PROMPT_IRI: &str = "urn:nl:prompt:sparql";

/// What a repair adds to it: `urn:nl:prompt:sparql-repair`.
pub const REPAIR_PROMPT_IRI: &str = "urn:nl:prompt:sparql-repair";

/// The templates this crate holds, by name.
pub const PROMPTS: [(&str, &str); 2] = [
    ("sparql", include_str!("prompts/sparql.txt")),
    ("sparql-repair", include_str!("prompts/sparql-repair.txt")),
];

/// The most vocabulary terms a prompt lists.
const MAX_TERMS: usize = 60;

/// Fill `{{name}}` placeholders in one pass. An unknown placeholder is left as written.
///
/// ```
/// let filled = ikigai_nl::prompt::fill(
///     "ask: {{ask}} / {{other}}",
///     &[("ask", "say {{other}}"), ("other", "x")],
/// );
/// assert_eq!(filled, "ask: say {{other}} / x");
/// ```
pub fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find("}}") {
            Some(end) => {
                let name = &after[..end];
                match values.iter().find(|(k, _)| *k == name) {
                    Some((_, value)) => out.push_str(value),
                    None => {
                        out.push_str("{{");
                        out.push_str(name);
                        out.push_str("}}");
                    }
                }
                rest = &after[end + 2..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// What the caller may read, as a prompt states it: each graph with its classes,
/// predicates and samples, then the vocabulary terms those graphs use (and, when the
/// grounding is focused, the terms that mention the focus).
///
/// The actions are left out on purpose: a query is not an action call, and the store's
/// query doors are fixed by the host, not chosen by the model.
pub fn grounding_text(grounding: &Grounding) -> String {
    let mut out = String::new();
    if grounding.graphs.items.is_empty() {
        out.push_str("No graph: this person may read no named graph.\n");
    } else {
        out.push_str("Graphs (read each inside GRAPH <iri> { ... } or with FROM <iri>):\n");
    }
    for shape in &grounding.graphs.items {
        out.push_str(&format!(
            "- <{}>: {} triples, {} subjects\n",
            shape.graph, shape.triples, shape.subjects
        ));
        if !shape.class_partitions.is_empty() {
            let classes: Vec<String> = shape
                .class_partitions
                .iter()
                .map(|p| format!("<{}> ({})", p.iri, p.count))
                .collect();
            out.push_str(&format!(
                "  classes ({} of {}): {}\n",
                classes.len(),
                shape.classes,
                classes.join(", ")
            ));
        }
        if !shape.property_partitions.is_empty() {
            let properties: Vec<String> = shape
                .property_partitions
                .iter()
                .map(|p| format!("<{}> ({})", p.iri, p.count))
                .collect();
            out.push_str(&format!(
                "  predicates ({} of {}): {}\n",
                properties.len(),
                shape.properties,
                properties.join(", ")
            ));
        }
        for sample in &shape.samples {
            out.push_str(&format!("  sample: {sample}\n"));
        }
    }
    if let Some(note) = &grounding.graphs.note {
        out.push_str(&format!("({note})\n"));
    }
    let used = crate::focus::used(&grounding.graphs.items);
    let terms: Vec<&Term> = grounding
        .vocabulary
        .items
        .iter()
        .filter(|t| grounding.focus.is_some() || used.contains(t.iri.as_str()))
        .take(MAX_TERMS)
        .collect();
    if !terms.is_empty() {
        out.push_str("Vocabulary terms:\n");
        for t in terms {
            out.push_str(&format!("- <{}> ({})", t.iri, t.kind));
            if let Some(comment) = t.comment.as_deref().or(t.label.as_deref()) {
                out.push_str(&format!(": {comment}"));
            }
            out.push('\n');
        }
    }
    out
}

/// Up to `n` of the grounding's published SPARQL scripts, as worked examples.
pub fn examples_text(grounding: &Grounding, n: usize) -> String {
    let examples: Vec<String> = grounding
        .examples
        .items
        .iter()
        .filter(|e| e.language == "sparql")
        .take(n)
        .map(|e| {
            format!(
                "{} ({}):\n```sparql\n{}\n```",
                e.name,
                e.iri,
                e.source.trim()
            )
        })
        .collect();
    if examples.is_empty() {
        "None.".to_string()
    } else {
        examples.join("\n\n")
    }
}

/// The query in a model's answer: the first fenced block's body when there is one,
/// else the whole answer.
///
/// ```
/// let answer = "Here it is:\n```sparql\nASK { GRAPH <urn:g> { ?s ?p ?o } }\n```\nDone.";
/// assert_eq!(ikigai_nl::prompt::extract(answer), "ASK { GRAPH <urn:g> { ?s ?p ?o } }");
/// assert_eq!(ikigai_nl::prompt::extract("  ASK {}  "), "ASK {}");
/// ```
pub fn extract(answer: &str) -> String {
    if let Some(start) = answer.find("```") {
        let after = &answer[start + 3..];
        // Skip the info string (`sparql`) to the end of the fence's line.
        let body = after.find('\n').map_or("", |nl| &after[nl + 1..]);
        let end = body.find("```").unwrap_or(body.len());
        return body[..end].trim().to_string();
    }
    answer.trim().to_string()
}

pub(crate) struct PromptEndpoint;

#[async_trait]
impl Endpoint for PromptEndpoint {
    async fn invoke(&self, inv: &Invocation<'_>) -> Result<Representation> {
        if inv.request.verb != Verb::Source {
            return Err(Error::Endpoint(format!(
                "nl-prompt answers Source, not {:?}",
                inv.request.verb
            )));
        }
        let name = inv.bindings.get("name").unwrap_or_default();
        let (_, template) = PROMPTS.iter().find(|(n, _)| *n == name).ok_or_else(|| {
            Error::NotFound(format!(
                "urn:nl:prompt:{name}: the prompts are {}",
                PROMPTS.map(|(n, _)| n).join(", ")
            ))
        })?;
        Ok(Representation::new(
            ReprType::new("text/plain").with_param("charset", "utf-8"),
            template.as_bytes().to_vec(),
        )
        .cacheable())
    }

    fn name(&self) -> &str {
        "nl-prompt"
    }

    fn describe(&self) -> Description {
        Description::new("nl-prompt")
            .title("A drafting prompt")
            .summary(
                "The template a natural-language drafter fills before it asks a model, as \
                 text with {{placeholders}}: `sparql` (the first draft) and `sparql-repair` \
                 (what a repair adds: the previous draft and what the checker refused). A \
                 draft's provenance cites the prompt by IRI and sha256.",
            )
            .verb(Verb::Meta)
            .action(
                ActionSpec::new(Verb::Source)
                    .summary("The template.")
                    .input(
                        ArgSpec::new("name")
                            .summary("`sparql` or `sparql-repair`.")
                            .class("http://www.w3.org/2001/XMLSchema#string")
                            .one_of(PROMPTS.map(|(n, _)| n))
                            .binding(),
                    )
                    .output("text/plain"),
            )
    }
}
