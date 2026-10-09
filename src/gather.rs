//! Each part of the grounding, read through the kernel under the CALLER's capability.
//!
//! Nothing here holds state or decides authority. Every read is a sub-request issued with
//! the invocation's own capability, so the kernel records it as a dependency (its expiry
//! and golden threads become the grounding's) and the resource being read enforces its
//! own grant. Two rules keep the grounding cacheable for callers who lack a grant:
//!
//! - **A resource the manifold does not offer is not read.** The manifold
//!   (`urn:kernel:actions`) is filtered by the same check the kernel's floor runs, so a
//!   resource absent from it would answer `Denied`, and a refusal is never cached: reading
//!   it anyway would make the whole grounding uncacheable for that caller.
//! - **A resource that is not bound is an empty part with a note**, never an error: the
//!   kernel records `Unresolved` as a thread named after the missing name, so binding it
//!   later recomputes the grounding.

use std::collections::{BTreeMap, BTreeSet};

use ikigai_core::{
    ArgRef, Bindings, Description, Error, Invocation, Iri, Representation, Request, Result,
    UriTemplate, Verb,
};
use oxrdf::{NamedOrBlankNode, Term as RdfTerm};
use serde::Deserialize;

use crate::config::{Examples, SpaceConfig};
use crate::model::{Action, Example, GraphShape, Input, Partition, Term};

/// The capability-scoped action manifold.
pub const ACTIONS_IRI: &str = "urn:kernel:actions";
/// Where `ikigai-vocab` binds the `ik:` vocabulary.
pub const VOCAB_IRI: &str = ikigai_vocab::VOCAB_IRI;
/// `ikigai-store`'s listing of the named graphs the caller may read.
pub const GRAPHS_IRI: &str = "urn:iki:store:graphs";
/// `ikigai-store`'s per-graph query door.
pub const GRAPH_SELECT_IRI: &str = "urn:iki:store:graph-select";
/// `ikigai-store`'s whole-dataset query door, for a broad reader.
pub const SELECT_IRI: &str = "urn:iki:store:select";
/// `ikigai-script`'s catalog.
pub const CATALOG_IRI: &str = "urn:script:catalog";

const JSON: &str = "application/json";
const SPARQL_JSON: &str = "application/sparql-results+json";

/// The value a template variable is filled with to describe a template binding: the
/// kernel's own catalog walk does the same, and `describe()` never reads its bindings.
const PROBE: &str = "probe";

const VERBS: [(Verb, &str); 4] = [
    (Verb::Source, "source"),
    (Verb::Sink, "sink"),
    (Verb::Exists, "exists"),
    (Verb::Delete, "delete"),
];

const TOTALS: &str = include_str!("queries/totals.rq");
const CLASS_TOTAL: &str = include_str!("queries/class-total.rq");
const CLASSES: &str = include_str!("queries/classes.rq");
const PROPERTIES: &str = include_str!("queries/properties.rq");
const SAMPLES: &str = include_str!("queries/samples.rq");

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDF_PROPERTY: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#Property";
const RDFS: &str = "http://www.w3.org/2000/01/rdf-schema#";
const OWL: &str = "http://www.w3.org/2002/07/owl#";

pub(crate) fn request(verb: Verb, target: &str, args: &[(&str, &str)]) -> Result<Request> {
    let iri = Iri::parse(target).map_err(|e| Error::Endpoint(format!("`{target}`: {e}")))?;
    let mut request = Request::new(verb, iri);
    for (name, value) in args {
        request = request.with_arg(*name, ArgRef::Inline(value.as_bytes().to_vec()));
    }
    Ok(request)
}

fn text(rep: &Representation) -> Result<&str> {
    std::str::from_utf8(&rep.bytes)
        .map_err(|e| Error::Endpoint(format!("a part of the grounding was not UTF-8: {e}")))
}

fn verb_name(verb: Verb) -> &'static str {
    VERBS
        .iter()
        .find(|(v, _)| *v == verb)
        .map(|(_, name)| *name)
        .unwrap_or("meta")
}

/// Whether an error means "nothing is there", which a part reports as empty rather than
/// failing the grounding.
fn is_absent(error: &Error) -> bool {
    matches!(error, Error::Unresolved(_) | Error::NotFound(_))
}

// ------------------------------------------------------------------------- manifold

/// What the caller may do: each offered endpoint pattern and the verbs offered on it.
#[derive(Default)]
pub(crate) struct Manifold {
    offered: BTreeMap<String, BTreeSet<&'static str>>,
}

impl Manifold {
    /// Read the manifold, one verb at a time (each read is the kernel's, cached and hung
    /// from the binding-change thread).
    pub(crate) async fn read(inv: &Invocation<'_>) -> Result<Manifold> {
        let mut manifold = Manifold::default();
        let requests = VERBS
            .iter()
            .map(|(_, name)| request(Verb::Source, ACTIONS_IRI, &[("verb", name)]))
            .collect::<Result<Vec<_>>>()?;
        for ((_, name), result) in VERBS.iter().zip(inv.fan_out(requests).await) {
            for line in text(&result?)?.lines() {
                let line = line.trim();
                if !line.is_empty() {
                    manifold
                        .offered
                        .entry(line.to_string())
                        .or_default()
                        .insert(name);
                }
            }
        }
        Ok(manifold)
    }

    /// Whether the manifold offers `verb` on exactly this pattern.
    pub(crate) fn offers(&self, pattern: &str, verb: &str) -> bool {
        self.offered
            .get(pattern)
            .is_some_and(|verbs| verbs.contains(verb))
    }
}

// -------------------------------------------------------------------------- actions

/// Every offered action with its contract, from each endpoint's JSON Meta face.
pub(crate) async fn actions(inv: &Invocation<'_>, manifold: &Manifold) -> Result<Vec<Action>> {
    let mut targets = Vec::new();
    for pattern in manifold.offered.keys() {
        targets.push((pattern.clone(), describable(pattern)?));
    }
    let requests = targets
        .iter()
        .map(|(_, target)| request(Verb::Meta, target, &[("as", JSON)]))
        .collect::<Result<Vec<_>>>()?;
    let mut actions = Vec::new();
    for ((pattern, target), result) in targets.iter().zip(inv.fan_out(requests).await) {
        let rep = result.map_err(|e| {
            Error::Endpoint(format!(
                "the grounding describes every action the manifold offers, and `Meta {target} \
                 as={JSON}` failed: {e}. The host's kernel needs a JSON Meta renderer \
                 (`ikigai_vocab::TurtleRenderer` has one)"
            ))
        })?;
        let description: Description = serde_json::from_slice(&rep.bytes).map_err(|e| {
            Error::Endpoint(format!("`Meta {target}` was not a JSON description: {e}"))
        })?;
        let exact = Iri::parse(pattern.as_str()).is_ok();
        for spec in description.action_specs() {
            let verb = verb_name(spec.verb);
            if !manifold.offers(pattern, verb) {
                continue;
            }
            actions.push(Action {
                action: format!(
                    "urn:ikigai:endpoint:{}:action:{verb}",
                    ikigai_core::escape_iri_fragment(&description.id)
                ),
                endpoint: exact.then(|| pattern.clone()),
                template: (!exact).then(|| pattern.clone()),
                id: description.id.clone(),
                title: description.title.clone(),
                summary: description.summary.clone(),
                verb: verb.to_string(),
                action_summary: spec.summary.clone(),
                requires: spec.requires.clone(),
                inputs: spec
                    .inputs
                    .iter()
                    .map(|i| Input {
                        name: i.name.clone(),
                        required: i.required,
                        binding: i.source == ikigai_core::InputSource::Binding,
                        class: i.class.clone(),
                        default: i.default.clone(),
                        one_of: i.one_of.clone(),
                        summary: i.summary.clone(),
                    })
                    .collect(),
                outputs: spec.outputs.clone(),
            });
        }
    }
    Ok(actions)
}

/// The IRI to ask `Meta` of for a manifold line: the line itself when it is an IRI, the
/// template filled with a probe value when it is a template.
fn describable(pattern: &str) -> Result<String> {
    if Iri::parse(pattern).is_ok() {
        return Ok(pattern.to_string());
    }
    let template = UriTemplate::parse(pattern)
        .map_err(|e| Error::Endpoint(format!("manifold line `{pattern}`: {e}")))?;
    let mut bindings = Bindings::new();
    for var in template.variables() {
        bindings.insert(var, PROBE);
    }
    template
        .expand(&bindings)
        .ok_or_else(|| Error::Endpoint(format!("manifold line `{pattern}` did not expand")))
}

// ----------------------------------------------------------------------- vocabulary

/// The vocabulary part: its `owl:versionInfo` and its classes and properties, or `None`
/// when no vocabulary is bound.
pub(crate) async fn vocabulary(
    inv: &Invocation<'_>,
) -> Result<Option<(Option<String>, Vec<Term>)>> {
    let iri = Iri::parse(VOCAB_IRI).map_err(|e| Error::Endpoint(e.to_string()))?;
    let rep = match inv.source(&iri).await {
        Ok(rep) => rep,
        Err(e) if is_absent(&e) => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut version = None;
    let mut subjects: BTreeMap<String, Term> = BTreeMap::new();
    let mut types: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for triple in oxttl::TurtleParser::new().for_slice(&rep.bytes) {
        let triple =
            triple.map_err(|e| Error::Endpoint(format!("{VOCAB_IRI} is not Turtle: {e}")))?;
        let NamedOrBlankNode::NamedNode(subject) = &triple.subject else {
            continue;
        };
        let subject = subject.as_str().to_string();
        let predicate = triple.predicate.as_str();
        let object = match &triple.object {
            RdfTerm::NamedNode(n) => n.as_str().to_string(),
            RdfTerm::Literal(l) => l.value().to_string(),
            _ => continue,
        };
        if predicate == RDF_TYPE {
            types.entry(subject).or_default().insert(object);
            continue;
        }
        if predicate == format!("{OWL}versionInfo") {
            version = Some(object);
            continue;
        }
        let term = subjects.entry(subject.clone()).or_insert_with(|| Term {
            iri: subject,
            kind: String::new(),
            label: None,
            comment: None,
            domain: Vec::new(),
            range: Vec::new(),
            broader: Vec::new(),
        });
        match predicate.strip_prefix(RDFS) {
            Some("label") => term.label = Some(object),
            Some("comment") => term.comment = Some(object),
            Some("domain") => term.domain.push(object),
            Some("range") => term.range.push(object),
            Some("subClassOf") | Some("subPropertyOf") => term.broader.push(object),
            _ => {}
        }
    }
    let mut terms = Vec::new();
    for (iri, typed) in types {
        let kind =
            if typed.contains(&format!("{RDFS}Class")) || typed.contains(&format!("{OWL}Class")) {
                "class"
            } else if typed.contains(RDF_PROPERTY)
                || typed
                    .iter()
                    .any(|t| t.starts_with(OWL) && t.ends_with("Property"))
            {
                "property"
            } else {
                continue;
            };
        let mut term = subjects.remove(&iri).unwrap_or_else(|| Term {
            iri: iri.clone(),
            kind: String::new(),
            label: None,
            comment: None,
            domain: Vec::new(),
            range: Vec::new(),
            broader: Vec::new(),
        });
        term.kind = kind.to_string();
        terms.push(term);
    }
    Ok(Some((version, terms)))
}

// --------------------------------------------------------------------------- graphs

/// The graphs part: every named graph the caller may read (sorted), and the shapes of the
/// first `max_graphs` of them. `None` when the store's listing is not offered to this
/// capability.
///
/// With a `focus`, graphs whose IRI mentions it are summarized first, so a focus on one
/// graph reaches it however many graphs sort ahead of it.
pub(crate) async fn graphs(
    inv: &Invocation<'_>,
    manifold: &Manifold,
    config: &SpaceConfig,
    focus: Option<&str>,
) -> Result<Option<(usize, Vec<GraphShape>)>> {
    if !manifold.offers(GRAPHS_IRI, "source") {
        return Ok(None);
    }
    let iri = Iri::parse(GRAPHS_IRI).map_err(|e| Error::Endpoint(e.to_string()))?;
    let rep = match inv.source(&iri).await {
        Ok(rep) => rep,
        Err(e) if is_absent(&e) => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut listed: Vec<String> = text(&rep)?
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    listed.sort();
    listed.dedup();
    if let Some(focus) = focus {
        // Stable: within each group the IRIs stay sorted.
        listed.sort_by_key(|graph| !crate::focus::mentions(focus, [graph.as_str()]));
    }
    let of = listed.len();
    let mut shapes = Vec::new();
    for graph in listed.into_iter().take(config.max_graphs) {
        shapes.push(shape(inv, manifold, config, &graph).await);
    }
    Ok(Some((of, shapes)))
}

/// One graph's shape. A failure is reported IN the shape, never dropped: a graph the
/// listing says this caller may read and the grounding then omits would be a silent lie.
async fn shape(
    inv: &Invocation<'_>,
    manifold: &Manifold,
    config: &SpaceConfig,
    graph: &str,
) -> GraphShape {
    let mut shape = GraphShape {
        graph: graph.to_string(),
        identity: String::new(),
        triples: 0,
        subjects: 0,
        properties: 0,
        objects: 0,
        classes: 0,
        class_partitions: Vec::new(),
        property_partitions: Vec::new(),
        samples: Vec::new(),
        error: None,
    };
    if let Err(e) = fill(inv, manifold, config, &mut shape).await {
        shape.error = Some(e.to_string());
    }
    shape.identity = crate::model::digest(&shape);
    shape
}

async fn fill(
    inv: &Invocation<'_>,
    manifold: &Manifold,
    config: &SpaceConfig,
    shape: &mut GraphShape,
) -> Result<()> {
    let graph = shape.graph.clone();
    // The scoped door when the caller holds this graph's grant (root holds every one);
    // the whole-dataset door for a broad reader, which the scoped door refuses. Both read
    // the one graph: the queries say `GRAPH ?g` and `?g` is bound to it.
    let scoped = inv
        .capability
        .allows(&format!("urn:cap:store:read:graph:{graph}"))
        && manifold.offers(GRAPH_SELECT_IRI, "source");
    let door = if scoped {
        GRAPH_SELECT_IRI
    } else if manifold.offers(SELECT_IRI, "source") {
        SELECT_IRI
    } else {
        return Err(Error::Denied(format!(
            "the store lists {graph} as readable, but neither {GRAPH_SELECT_IRI} with its \
             grant nor {SELECT_IRI} is offered to this capability"
        )));
    };
    // The graph reaches the query as a typed value through the store's `bindings=`, never
    // as query text: the store rejects a binding the query does not project, and these
    // queries all project `?g`.
    let bindings = serde_json::json!({ "g": { "type": "uri", "value": graph } }).to_string();
    let partitions = config.max_partitions.to_string();
    let samples = config.samples.to_string();
    let queries = [
        TOTALS.to_string(),
        CLASS_TOTAL.to_string(),
        CLASSES.replace("{limit}", &partitions),
        PROPERTIES.replace("{limit}", &partitions),
        SAMPLES.replace("{limit}", &samples),
    ];
    let requests = queries
        .iter()
        .map(|query| {
            let mut args = vec![
                ("query", query.as_str()),
                ("bindings", bindings.as_str()),
                ("as", SPARQL_JSON),
            ];
            if scoped {
                args.push(("graph", graph.as_str()));
            }
            request(Verb::Source, door, &args)
        })
        .collect::<Result<Vec<_>>>()?;
    let mut results = Vec::new();
    for result in inv.fan_out(requests).await {
        results.push(Rows::parse(&result?)?);
    }
    let [totals, class_total, classes, properties, samples] = <[Rows; 5]>::try_from(results)
        .map_err(|_| Error::Endpoint("expected five query results".to_string()))?;
    if let Some(row) = totals.0.first() {
        shape.triples = row.count("triples");
        shape.subjects = row.count("subjects");
        shape.properties = row.count("properties");
        shape.objects = row.count("objects");
    }
    if let Some(row) = class_total.0.first() {
        shape.classes = row.count("classes");
    }
    let partition = |row: &Row, var: &str| {
        row.iri(var).map(|iri| Partition {
            iri,
            count: row.count("count"),
        })
    };
    shape.class_partitions = classes
        .0
        .iter()
        .filter_map(|r| partition(r, "class"))
        .collect();
    shape.property_partitions = properties
        .0
        .iter()
        .filter_map(|r| partition(r, "property"))
        .collect();
    if config.samples > 0 {
        shape.samples = samples
            .0
            .iter()
            .filter_map(|row| {
                Some(format!(
                    "{} {} {} .",
                    row.ntriples("s")?,
                    row.ntriples("p")?,
                    row.ntriples("o")?
                ))
            })
            .collect();
    }
    Ok(())
}

/// A SPARQL JSON result set, as rows of variable → term.
struct Rows(Vec<Row>);

pub(crate) struct Row(pub(crate) BTreeMap<String, ResultTerm>);

#[derive(Deserialize)]
struct ResultSet {
    results: ResultBindings,
}

#[derive(Deserialize)]
struct ResultBindings {
    bindings: Vec<BTreeMap<String, ResultTerm>>,
}

/// One term in the SPARQL 1.1 JSON results format.
#[derive(Deserialize)]
pub(crate) struct ResultTerm {
    #[serde(rename = "type")]
    kind: String,
    value: String,
    datatype: Option<String>,
    #[serde(rename = "xml:lang")]
    lang: Option<String>,
}

impl Rows {
    fn parse(rep: &Representation) -> Result<Rows> {
        let set: ResultSet = serde_json::from_slice(&rep.bytes).map_err(|e| {
            Error::Endpoint(format!(
                "the store's answer was not SPARQL JSON results: {e}"
            ))
        })?;
        Ok(Rows(set.results.bindings.into_iter().map(Row).collect()))
    }
}

impl Row {
    fn count(&self, var: &str) -> u64 {
        self.0
            .get(var)
            .and_then(|t| t.value.parse().ok())
            .unwrap_or(0)
    }

    fn iri(&self, var: &str) -> Option<String> {
        self.0
            .get(var)
            .filter(|t| t.kind == "uri")
            .map(|t| t.value.clone())
    }

    /// The term as N-Triples, serialized by `oxrdf` (the grammar's owner), never by hand.
    pub(crate) fn ntriples(&self, var: &str) -> Option<String> {
        let term = self.0.get(var)?;
        Some(match term.kind.as_str() {
            "uri" => oxrdf::NamedNode::new(&term.value).ok()?.to_string(),
            "bnode" => oxrdf::BlankNode::new(&term.value).ok()?.to_string(),
            "literal" | "typed-literal" => match (&term.lang, &term.datatype) {
                (Some(lang), _) => oxrdf::Literal::new_language_tagged_literal(&term.value, lang)
                    .ok()?
                    .to_string(),
                (None, Some(datatype)) => oxrdf::Literal::new_typed_literal(
                    &term.value,
                    oxrdf::NamedNode::new(datatype).ok()?,
                )
                .to_string(),
                (None, None) => oxrdf::Literal::new_simple_literal(&term.value).to_string(),
            },
            _ => return None,
        })
    }
}

// ------------------------------------------------------------------------- examples

/// The examples part: its source, how many were offered, and the examples.
pub(crate) struct Gathered {
    pub(crate) source: Option<String>,
    pub(crate) of: usize,
    pub(crate) items: Vec<Example>,
    pub(crate) note: Option<String>,
}

/// `ikigai-script`'s catalog row: only the fields this crate reads.
#[derive(Deserialize)]
struct CatalogRow {
    name: String,
    state: Option<String>,
}

#[derive(Deserialize)]
struct Catalog {
    scripts: Vec<CatalogRow>,
}

/// `ikigai-script`'s script document (`urn:script:{name}` as JSON): only the fields this
/// crate reads, and `parameters` passed through whatever its shape.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScriptDocument {
    iri: String,
    name: String,
    version: String,
    state: String,
    #[serde(default)]
    public: bool,
    version_iri: Option<String>,
    language: String,
    #[serde(default)]
    requires: Vec<String>,
    source: String,
    parameters: Option<serde_json::Value>,
}

pub(crate) async fn examples(
    inv: &Invocation<'_>,
    manifold: &Manifold,
    config: &SpaceConfig,
) -> Result<Gathered> {
    let (source, names, of, note) = match &config.examples {
        Examples::None => {
            return Ok(Gathered {
                source: None,
                of: 0,
                items: Vec::new(),
                note: Some("this host gives no worked examples".to_string()),
            })
        }
        Examples::Catalog => {
            if !manifold.offers(CATALOG_IRI, "source") {
                return Ok(Gathered {
                    source: Some(CATALOG_IRI.to_string()),
                    of: 0,
                    items: Vec::new(),
                    note: Some(format!(
                        "{CATALOG_IRI} is not offered to this capability, or not bound"
                    )),
                });
            }
            let rep = match inv
                .issue(request(Verb::Source, CATALOG_IRI, &[("as", JSON)])?)
                .await
            {
                Ok(rep) => rep,
                Err(e) if is_absent(&e) => {
                    return Ok(Gathered {
                        source: Some(CATALOG_IRI.to_string()),
                        of: 0,
                        items: Vec::new(),
                        note: Some(format!("{CATALOG_IRI} is not bound")),
                    })
                }
                Err(e) => return Err(e),
            };
            let catalog: Catalog = serde_json::from_slice(&rep.bytes).map_err(|e| {
                Error::Endpoint(format!(
                    "{CATALOG_IRI} was not the catalog's JSON face: {e}"
                ))
            })?;
            let mut names: Vec<String> = catalog
                .scripts
                .into_iter()
                .filter(|row| row.state.as_deref() == Some("published"))
                .map(|row| row.name)
                .collect();
            names.sort();
            let of = names.len();
            (Some(CATALOG_IRI.to_string()), names, of, None)
        }
        // `of` is counted below, over what this caller could read: a configured script
        // it may not read contributes nothing, not its name and not to a count.
        Examples::Named(names) => (None, names.clone(), 0, None),
    };
    let named = matches!(config.examples, Examples::Named(_));
    let mut items = Vec::new();
    for name in names {
        // The catalog's list is already the caller's, so its bound stops the reads; a
        // configured list is read in full (it is short, and each read is cached) so its
        // count is this caller's.
        if !named && items.len() >= config.max_examples {
            break;
        }
        // A configured name the caller holds no read grant for is not asked about: the
        // answer would be a refusal, and a refusal is never cached.
        if named
            && !inv
                .capability
                .allows(&format!("urn:cap:script:read:{name}"))
            && !inv.capability.allows("urn:cap:script:read:public")
        {
            continue;
        }
        let target = format!("urn:script:{name}");
        let Ok(rep) = inv
            .issue(request(Verb::Source, &target, &[("as", JSON)])?)
            .await
        else {
            continue;
        };
        let Ok(doc) = serde_json::from_slice::<ScriptDocument>(&rep.bytes) else {
            continue;
        };
        if doc.state != "published" {
            continue;
        }
        items.push(Example {
            iri: doc.iri,
            name: doc.name,
            version: doc.version,
            version_iri: doc.version_iri,
            language: doc.language,
            public: doc.public,
            requires: doc.requires,
            source: doc.source,
            parameters: doc.parameters,
        });
    }
    let of = if named { items.len() } else { of };
    items.truncate(config.max_examples);
    Ok(Gathered {
        source,
        of,
        items,
        note,
    })
}
