//! `urn:nl:grounding`: the composition.

use std::sync::Arc;

use async_trait::async_trait;
use ikigai_core::{
    ActionSpec, ArgRef, ArgSpec, Description, Endpoint, Error, Invocation, ReprType,
    Representation, Result, Verb,
};
use serde::Serialize;

use crate::config::SpaceConfig;
use crate::gather::{self, Manifold, ACTIONS_IRI, GRAPHS_IRI, VOCAB_IRI};
use crate::model::{digest, Grounding, Part, SCHEMA};
use crate::{focus, render};

const TURTLE: &str = "text/turtle";
const JSON: &str = "application/json";
const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";

pub(crate) struct GroundingEndpoint {
    pub(crate) config: Arc<SpaceConfig>,
}

/// An optional inline string argument. Present but unreadable is refused, never read as
/// absent: a `focus` silently dropped would widen the answer.
fn optional<'a>(inv: &'a Invocation<'_>, name: &str) -> Result<Option<&'a str>> {
    match inv.request.args.get(name) {
        None => Ok(None),
        Some(ArgRef::Inline(bytes)) => std::str::from_utf8(bytes)
            .map(|s| Some(s.trim()).filter(|s| !s.is_empty()))
            .map_err(|_| Error::InvalidArgument {
                name: name.to_string(),
                detail: "not UTF-8".to_string(),
            }),
        Some(_) => Err(Error::InvalidArgument {
            name: name.to_string(),
            detail: "pass it inline (`name=value`), not by reference".to_string(),
        }),
    }
}

/// The digest a grounding is named by: its focus and its parts' identities.
#[derive(Serialize)]
struct Identity<'a> {
    schema: u32,
    focus: Option<&'a str>,
    parts: [&'a str; 4],
}

#[async_trait]
impl Endpoint for GroundingEndpoint {
    async fn invoke(&self, inv: &Invocation<'_>) -> Result<Representation> {
        if inv.request.verb != Verb::Source {
            return Err(Error::Endpoint(format!(
                "nl-grounding answers Source, not {:?}",
                inv.request.verb
            )));
        }
        let want = match optional(inv, "as")? {
            None | Some(TURTLE) => TURTLE,
            Some(JSON) => JSON,
            Some(other) => {
                return Err(Error::InvalidArgument {
                    name: "as".to_string(),
                    detail: format!("`{other}`: this resource answers {TURTLE} or {JSON}"),
                })
            }
        };
        let focus = optional(inv, "focus")?;
        let config = &self.config;
        let manifold = Manifold::read(inv).await?;

        // The actions.
        let offered = gather::actions(inv, &manifold).await?;
        let of = offered.len();
        let kept: Vec<_> = match focus {
            Some(f) => offered
                .into_iter()
                .filter(|a| focus::action(f, a))
                .collect(),
            None => offered,
        };
        let note = (kept.len() < of).then(|| narrowed(kept.len(), of, "actions", focus));
        let actions = Part::new(Some(ACTIONS_IRI.to_string()), None, of, note, kept);

        // The graphs (before the vocabulary: a focused vocabulary keeps the terms the
        // kept graphs use).
        let graphs = match gather::graphs(inv, &manifold, config, focus).await? {
            None => Part::new(
                Some(GRAPHS_IRI.to_string()),
                None,
                0,
                Some(format!(
                    "{GRAPHS_IRI} is not offered to this capability, or not bound: no graph \
                     is readable"
                )),
                Vec::new(),
            ),
            Some((of, shapes)) => {
                let summarized = shapes.len();
                let kept: Vec<_> = match focus {
                    Some(f) => shapes.into_iter().filter(|s| focus::graph(f, s)).collect(),
                    None => shapes,
                };
                let note = if summarized < of {
                    Some(format!(
                        "the first {summarized} of {of} readable graphs were summarized (the \
                         bound is {}){}",
                        config.max_graphs,
                        if focus.is_some() {
                            format!(", {} of them mention the focus", kept.len())
                        } else {
                            "; name one with focus=".to_string()
                        }
                    ))
                } else {
                    (kept.len() < of).then(|| narrowed(kept.len(), of, "graphs", focus))
                };
                Part::new(Some(GRAPHS_IRI.to_string()), None, of, note, kept)
            }
        };

        // The vocabulary.
        let vocabulary = match gather::vocabulary(inv).await? {
            None => Part::new(
                Some(VOCAB_IRI.to_string()),
                None,
                0,
                Some(format!("{VOCAB_IRI} is not bound")),
                Vec::new(),
            ),
            Some((version, terms)) => {
                let of = terms.len();
                let used = focus::used(&graphs.items);
                let kept: Vec<_> = match focus {
                    Some(f) => terms
                        .into_iter()
                        .filter(|t| focus::term(f, t, &used))
                        .collect(),
                    None => terms,
                };
                let note = (kept.len() < of).then(|| narrowed(kept.len(), of, "terms", focus));
                Part::new(Some(VOCAB_IRI.to_string()), version, of, note, kept)
            }
        };

        // The worked examples.
        let gathered = gather::examples(inv, &manifold, config).await?;
        let fetched = gathered.items.len();
        let kept: Vec<_> = match focus {
            Some(f) => gathered
                .items
                .into_iter()
                .filter(|e| focus::example(f, e))
                .collect(),
            None => gathered.items,
        };
        let note = match gathered.note {
            Some(note) => Some(note),
            None if fetched < gathered.of => Some(format!(
                "the first {fetched} of {} published scripts were read (the bound is {})",
                gathered.of, config.max_examples
            )),
            None => (kept.len() < gathered.of)
                .then(|| narrowed(kept.len(), gathered.of, "scripts", focus)),
        };
        let examples = Part::new(gathered.source, None, gathered.of, note, kept);

        let identity = digest(&Identity {
            schema: SCHEMA,
            focus,
            parts: [
                &actions.identity,
                &vocabulary.identity,
                &graphs.identity,
                &examples.identity,
            ],
        });
        let grounding = Grounding {
            schema: SCHEMA,
            iri: format!("urn:nl:grounding:{identity}"),
            identity,
            focus: focus.map(str::to_string),
            actions,
            vocabulary,
            graphs,
            examples,
        };

        let (repr_type, bytes) = if want == JSON {
            let mut bytes = serde_json::to_vec_pretty(&grounding)
                .map_err(|e| Error::Endpoint(format!("serializing the grounding: {e}")))?;
            bytes.push(b'\n');
            (ReprType::new(JSON), bytes)
        } else {
            (
                ReprType::new(TURTLE).with_param("charset", "utf-8"),
                render::turtle(&grounding),
            )
        };
        if bytes.len() > config.max_bytes {
            return Err(Error::InvalidArgument {
                name: "focus".to_string(),
                detail: format!(
                    "the grounding {} is {} bytes as {want}, over this host's bound of {} \
                     (it holds {} actions, {} vocabulary terms, {} graphs and {} examples). \
                     Narrow it with focus= a graph IRI or a topic word",
                    match focus {
                        Some(f) => format!("for focus `{f}`"),
                        None => "with no focus".to_string(),
                    },
                    bytes.len(),
                    config.max_bytes,
                    grounding.actions.shown,
                    grounding.vocabulary.shown,
                    grounding.graphs.shown,
                    grounding.examples.shown,
                ),
            });
        }
        // Cacheable as far as its parts are: the kernel meets this with every part's
        // expiry and hangs it from every part's threads, and keys it on the caller's
        // capability, so it is never served to a caller it was not computed for.
        Ok(Representation::new(repr_type, bytes).cacheable())
    }

    fn name(&self) -> &str {
        "nl-grounding"
    }

    fn describe(&self) -> Description {
        Description::new("nl-grounding")
            .title("Grounding for a natural-language drafter")
            .summary(
                "What a drafter may know, as data, under the CALLER's capability: the actions \
                 it may take (the manifold, with each action's arguments), the ik: \
                 vocabulary, the shape of each named graph it may read (VoID-like totals, \
                 the classes and predicates used, a few sample triples), and published \
                 scripts it may read as worked examples. A graph or script the caller cannot \
                 read contributes nothing, not even its name. Each part names the resource it \
                 came from and a sha256 identity, and the grounding is named by the sha256 of \
                 its parts, so a draft can cite exactly what it saw. Bounded: a part that \
                 shows fewer items than were offered says so (`nl:shown` of `nl:of`), and a \
                 grounding over the host's size bound is refused with its size, never \
                 truncated. Cached per capability, recomputed when a graph, a description or \
                 a script it read changes.",
            )
            .verb(Verb::Meta)
            .action(
                ActionSpec::new(Verb::Source)
                    .summary("The grounding for this capability, optionally narrowed by focus.")
                    .input(
                        ArgSpec::new("focus")
                            .summary(
                                "Narrow to a graph IRI or a topic word: every part keeps the \
                                 items that mention it (case-insensitive), a graph also when \
                                 a class or predicate it uses does, and a vocabulary term \
                                 also when a kept graph uses it.",
                            )
                            .class(XSD_STRING)
                            .optional(),
                    )
                    .input(
                        ArgSpec::new("as")
                            .summary("The face: Turtle (default) or the JSON form.")
                            .class(XSD_STRING)
                            .one_of([TURTLE, JSON])
                            .default_value(TURTLE)
                            .optional(),
                    )
                    .output(TURTLE)
                    .output(JSON),
            )
    }
}

fn narrowed(shown: usize, of: usize, what: &str, focus: Option<&str>) -> String {
    match focus {
        Some(f) => format!("{shown} of {of} {what} mention the focus `{f}`"),
        None => format!("{shown} of {of} {what}"),
    }
}
