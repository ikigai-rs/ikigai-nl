//! `urn:nl:sparql:check`: a SPARQL draft, validated mechanically, under the caller.
//!
//! The drafter's validation is a resource of its own, so a person editing a draft can ask
//! the same question the drafter asked, and `urn:nl:script` can reuse it. In order:
//!
//! 1. **Bounded and parsed**, by the script host's own analysis (`ikigai_script::sparql::
//!    analyze`: the pre-parse nesting bound, the parse, the declared parameters, the dataset
//!    rules). A draft must be a query (a read), never an update.
//! 2. **Authority**: the capability the script host would DERIVE from the graphs the text
//!    names, against the caller's own. A graph the caller cannot read is refused in the
//!    same words as a graph that does not exist, so a refusal never says which it was.
//! 3. **IRIs**: every predicate and every class (`rdf:type` object) the query names is
//!    checked against the grounding: the classes and predicates of the graphs it reads, and
//!    the vocabulary. An IRI nothing knows is an error; when a graph's partitions were
//!    bounded ("first N of M"), it is a warning instead, because the grounding cannot know.
//! 4. **A dry run**: the query, its parameters' defaults bound, under a small `LIMIT`,
//!    through the store's graph-scoped door, under the caller's capability. An EMPTY
//!    preview is not an error (an answer can be legitimately empty), but it is a warning
//!    ([`EMPTY_PREVIEW`]) with its evidence: each triple pattern is run alone under the same
//!    `LIMIT`, and those that match nothing on their own are named ([`Check::unmatched`]),
//!    so `"open"` where the data holds `ledger:open` is named, not guessed at. The drafter
//!    repairs on it (ledger #1242).
//!
//! Every finding is a VALUE in the answer, never a refusal: a refusal is never cached, and
//! "this draft is wrong" is a legitimate answer, not a failure of the resource.

use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use ikigai_core::{
    is_deny_scope, ActionSpec, ArgRef, ArgSpec, Capability, Description, Endpoint, Error,
    Invocation, ReprType, Representation, Result, Verb,
};
use serde::{Deserialize, Serialize};
use spargebra::algebra::{GraphPattern, PropertyPathExpression};
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};
use spargebra::{Query, SparqlParser};

use ikigai_script::limits;
// The script host's own analysis, the one it runs when a draft is saved: one contract, so
// what this check passes is what the save accepts (ledger #1244 retired the copy).
use ikigai_script::sparql::{self as script_sparql, Form, SparqlDoor, Value};

use crate::config::SpaceConfig;
use crate::gather::{request, ResultTerm, Row};
use crate::model::{GraphShape, Grounding};
use crate::GROUNDING_IRI;

/// `urn:nl:sparql:check`.
pub const CHECK_IRI: &str = "urn:nl:sparql:check";

/// The most rows (or triples) a dry run may preview.
pub const MAX_PREVIEW: usize = 50;

/// The most triple patterns an empty preview is probed with, one store query each.
pub const MAX_PROBES: usize = 16;

/// How the warning about an empty preview begins, in a check, in an attempt's warnings and
/// in a saved draft's provenance (`ik:draftWarning`), so a reader can find it.
pub const EMPTY_PREVIEW: &str = "the preview is empty";

/// The grounding parts the SPARQL drafter and this check read (`parts=`): the graphs, the
/// vocabulary and the worked examples. Never the actions: a query is not an action call,
/// the store's query doors are the host's choice, and on a host with many doors the
/// actions are most of the grounding (1,529 of them on gonk, about 100 KB per browse
/// root), which took an unfocused ask over the size bound (ledger #1243).
pub const DRAFTING_PARTS: &str = "vocabulary graphs examples";

const PLAIN: &str = "text/plain";
const JSON: &str = "application/json";
const SPARQL_JSON: &str = "application/sparql-results+json";
const NTRIPLES: &str = "application/n-triples";
const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";
const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

/// The version of the check's JSON face.
pub const CHECK_SCHEMA: u32 = 1;

/// What `urn:nl:sparql:check` found.
///
/// ```
/// let check: ikigai_nl::Check = serde_json::from_str(r#"{
///   "schema": 1, "valid": false, "grounding": "urn:nl:grounding:sha256:00",
///   "graphs": [], "anyGraph": false, "requires": [],
///   "errors": ["not a SPARQL query"], "warnings": []
/// }"#).unwrap();
/// assert!(!check.valid && check.preview.is_none());
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    /// [`CHECK_SCHEMA`].
    pub schema: u32,
    /// Whether the draft passed every check: no errors.
    pub valid: bool,
    /// The grounding it was checked against (`urn:nl:grounding:sha256:…`).
    pub grounding: String,
    /// `select`, `ask`, `construct`, `describe` (or `update`, which is refused).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub form: Option<String>,
    /// The parameters it declares.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub parameters: Vec<Param>,
    /// The graphs its text fixes.
    pub graphs: Vec<String>,
    /// Whether it also reads a graph only a run decides (`GRAPH ?g`).
    pub any_graph: bool,
    /// The capability a run needs, DERIVED from the graphs it names.
    pub requires: Vec<String>,
    /// What is wrong with it. Empty when it is valid.
    pub errors: Vec<String>,
    /// What may be wrong with it, which the grounding cannot decide.
    pub warnings: Vec<String>,
    /// The dry run, when every check before it passed and a preview was asked for.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub preview: Option<Preview>,
    /// When the preview is empty: each triple pattern that matches nothing on its own, run
    /// alone under the preview's `LIMIT` in the graphs the query reads, as SPARQL (`GRAPH
    /// <g> { ?item <…#status> "open" }`). Empty when the preview has rows, and when every
    /// pattern matches something (then their combination is what matches nothing).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub unmatched: Vec<String>,
}

/// One declared parameter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Param {
    /// The variable it binds.
    pub name: String,
    /// Its XSD datatype or class IRI.
    #[serde(rename = "type")]
    pub kind: String,
    /// Whether a run must pass it.
    pub required: bool,
    /// Its default, in its lexical form.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub default: Option<String>,
    /// What it means.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub summary: Option<String>,
}

/// A dry run's first rows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    /// The `LIMIT` it ran under.
    pub limit: usize,
    /// The graphs it read.
    pub graphs: Vec<String>,
    /// SELECT: the variables, in order.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub columns: Vec<String>,
    /// SELECT: each row's terms as N-Triples, `""` where a variable is unbound.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub rows: Vec<Vec<String>>,
    /// ASK: the answer.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub boolean: Option<bool>,
    /// CONSTRUCT and DESCRIBE: the first triples, as N-Triples lines.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub triples: Vec<String>,
}

impl Check {
    fn new(grounding: &str) -> Check {
        Check {
            schema: CHECK_SCHEMA,
            valid: false,
            grounding: grounding.to_string(),
            form: None,
            parameters: Vec::new(),
            graphs: Vec::new(),
            any_graph: false,
            requires: Vec::new(),
            errors: Vec::new(),
            warnings: Vec::new(),
            preview: None,
            unmatched: Vec::new(),
        }
    }

    /// The plain-text face: a verdict line, then what was found.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        let form = self.form.as_deref().unwrap_or("text");
        if self.valid {
            out.push_str(&format!("valid {form}"));
        } else {
            out.push_str(&format!("invalid {form}"));
        }
        if !self.graphs.is_empty() {
            let graphs: Vec<String> = self.graphs.iter().map(|g| format!("<{g}>")).collect();
            out.push_str(&format!(" over {}", graphs.join(" ")));
        }
        if self.any_graph {
            out.push_str(" and the caller's readable graphs");
        }
        out.push('\n');
        for scope in &self.requires {
            out.push_str(&format!("requires: {scope}\n"));
        }
        for p in &self.parameters {
            out.push_str(&format!(
                "parameter: {} {}{}\n",
                p.name,
                p.kind,
                match (&p.default, p.required) {
                    (Some(d), _) => format!(" default {d}"),
                    (None, true) => " required".to_string(),
                    (None, false) => " optional".to_string(),
                }
            ));
        }
        for e in &self.errors {
            out.push_str(&format!("error: {e}\n"));
        }
        for w in &self.warnings {
            out.push_str(&format!("warning: {w}\n"));
        }
        if let Some(p) = &self.preview {
            out.push_str(&p.to_text());
        }
        out
    }
}

impl Preview {
    /// Whether the dry run found nothing: no row for a SELECT, no triple for a CONSTRUCT or
    /// DESCRIBE. An ASK is never empty: `false` is its answer.
    ///
    /// ```
    /// let p: ikigai_nl::Preview = serde_json::from_str(
    ///     r#"{ "limit": 5, "graphs": ["urn:g"], "columns": ["n"] }"#,
    /// ).unwrap();
    /// assert!(p.is_empty());
    /// let ask: ikigai_nl::Preview =
    ///     serde_json::from_str(r#"{ "limit": 5, "graphs": ["urn:g"], "boolean": false }"#).unwrap();
    /// assert!(!ask.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.boolean.is_none() && self.rows.is_empty() && self.triples.is_empty()
    }

    /// The plain-text face of the rows.
    pub fn to_text(&self) -> String {
        let mut out = format!("preview (LIMIT {}):\n", self.limit);
        if let Some(b) = self.boolean {
            out.push_str(&format!("{b}\n"));
        }
        if !self.columns.is_empty() {
            let header: Vec<String> = self.columns.iter().map(|c| format!("?{c}")).collect();
            out.push_str(&header.join("\t"));
            out.push('\n');
            for row in &self.rows {
                out.push_str(&row.join("\t"));
                out.push('\n');
            }
        }
        for t in &self.triples {
            out.push_str(t);
            out.push('\n');
        }
        out
    }
}

/// An `InvalidArgument`'s detail, or any other error's text: what a model is shown.
pub(crate) fn detail(error: &Error) -> String {
    match error {
        Error::InvalidArgument { detail, .. } => detail.clone(),
        other => other.to_string(),
    }
}

/// Whether `cap` holds `scope`, reading a trailing `*` as the script host does: "holds
/// some grant under this prefix" (root holds every one).
pub(crate) fn holds(cap: &Capability, scope: &str) -> bool {
    match scope.strip_suffix('*') {
        Some(prefix) => {
            cap.is_root()
                || cap.scopes().is_some_and(|held| {
                    held.iter()
                        .any(|s| s.starts_with(prefix) && !is_deny_scope(s))
                })
        }
        None => cap.allows(scope),
    }
}

/// The grounding's JSON face, as the drafter and this check read it ([`DRAFTING_PARTS`]),
/// under the caller.
pub(crate) async fn grounding(inv: &Invocation<'_>, focus: Option<&str>) -> Result<Grounding> {
    let mut args = vec![("as", JSON), ("parts", DRAFTING_PARTS)];
    if let Some(f) = focus {
        args.push(("focus", f));
    }
    let rep = inv
        .issue(request(Verb::Source, GROUNDING_IRI, &args)?)
        .await?;
    serde_json::from_slice(&rep.bytes)
        .map_err(|e| Error::Endpoint(format!("{GROUNDING_IRI} did not answer its JSON face: {e}")))
}

/// Validate `text` under the caller, against the grounding for `focus`, previewing `rows`.
pub(crate) async fn check(
    inv: &Invocation<'_>,
    config: &SpaceConfig,
    text: &str,
    focus: Option<&str>,
    rows: usize,
) -> Result<Check> {
    let grounding = grounding(inv, focus).await?;
    let mut check = Check::new(&grounding.iri);
    let door = SparqlDoor::store_at(config.store_prefix.clone());

    // 1. Bounded, parsed, and shaped exactly as the script host will at save.
    let analysis = match script_sparql::analyze(text, &door) {
        Ok(analysis) => analysis,
        Err(e) => {
            check.errors.push(detail(&e));
            return Ok(check);
        }
    };
    check.form = Some(analysis.form.as_str().to_string());
    check.parameters = analysis
        .parameters
        .iter()
        .map(|p| Param {
            name: p.name.clone(),
            kind: p.kind.iri().to_string(),
            required: p.required,
            default: p.default.clone(),
            summary: p.summary.clone(),
        })
        .collect();
    check.graphs = analysis.graphs.iter().cloned().collect();
    check.any_graph = analysis.any_graph;
    check.requires = analysis.requires().into_iter().collect();
    if !analysis.form.is_read() {
        check.errors.push(
            "this is an UPDATE, a write; draft a query that reads (SELECT, ASK, CONSTRUCT or \
             DESCRIBE)"
                .to_string(),
        );
        return Ok(check);
    }

    // 2. Authority: what the script host would derive, against the caller's own.
    let listed: BTreeSet<&str> = grounding
        .graphs
        .items
        .iter()
        .map(|s| s.graph.as_str())
        .collect();
    for graph in &analysis.graphs {
        if !inv.capability.allows(&script_sparql::cap_read_graph(graph)) {
            // ★ The same words whether the graph exists or not: a refusal must not tell
            // the caller (or the model) that a graph it cannot read is there.
            check.errors.push(format!(
                "the query reads <{graph}>, which is not a graph this caller may read: read \
                 only the graphs the grounding lists"
            ));
        } else if !listed.contains(graph.as_str()) {
            check.warnings.push(format!(
                "<{graph}> is not in the grounding (it holds nothing yet, or it is past the \
                 grounding's bound)"
            ));
        }
    }
    if analysis.any_graph && !holds(inv.capability, script_sparql::CAP_READ_GRAPH) {
        check.errors.push(
            "the query reads a variable graph (GRAPH ?g: the caller's readable graphs), and \
             this caller may read no graph"
                .to_string(),
        );
    }

    // 3. Every predicate and class it names, against the grounding.
    let named = limits::on_sparql_stack(text, || Ok(Named::of(text)))?;
    let shapes: Vec<&GraphShape> = grounding
        .graphs
        .items
        .iter()
        .filter(|s| analysis.any_graph || analysis.graphs.contains(&s.graph))
        .collect();
    let all_listed = analysis.graphs.iter().all(|g| listed.contains(g.as_str()));
    let terms = |kind: &str| -> BTreeSet<&str> {
        grounding
            .vocabulary
            .items
            .iter()
            .filter(|t| t.kind == kind)
            .map(|t| t.iri.as_str())
            .collect()
    };
    let (vocab_properties, vocab_classes) = (terms("property"), terms("class"));
    let known_properties: BTreeSet<&str> = shapes
        .iter()
        .flat_map(|s| &s.property_partitions)
        .map(|p| p.iri.as_str())
        .chain(vocab_properties)
        .collect();
    let known_classes: BTreeSet<&str> = shapes
        .iter()
        .flat_map(|s| &s.class_partitions)
        .map(|p| p.iri.as_str())
        .chain(vocab_classes)
        .collect();
    let properties_complete = all_listed
        && shapes
            .iter()
            .all(|s| s.error.is_none() && s.property_partitions.len() as u64 >= s.properties);
    let classes_complete = all_listed
        && shapes
            .iter()
            .all(|s| s.error.is_none() && s.class_partitions.len() as u64 >= s.classes);
    for p in &named.predicates {
        if !known_properties.contains(p.as_str()) {
            let finding = format!(
                "the predicate <{p}> is used by none of the graphs this query reads and is \
                 not in the vocabulary"
            );
            if properties_complete {
                check.errors.push(finding);
            } else {
                check.warnings.push(format!(
                    "{finding}, as far as the grounding shows (it lists the first predicates \
                     of a graph, not all)"
                ));
            }
        }
    }
    for c in &named.classes {
        if !known_classes.contains(c.as_str()) {
            let finding = format!(
                "the class <{c}> has no instances in the graphs this query reads and is not \
                 in the vocabulary"
            );
            if classes_complete {
                check.errors.push(finding);
            } else {
                check.warnings.push(format!(
                    "{finding}, as far as the grounding shows (it lists the first classes of \
                     a graph, not all)"
                ));
            }
        }
    }

    // 4. A dry run, only of a draft nothing above refused.
    if check.errors.is_empty() && rows > 0 {
        match preview(
            inv,
            &door,
            text,
            &analysis,
            &grounding,
            rows,
            &mut check.warnings,
        )
        .await
        {
            Ok((p, unmatched)) => {
                check.preview = Some(p);
                check.unmatched = unmatched;
            }
            Err(e) => check.errors.push(format!("the dry run failed: {e}")),
        }
    }
    check.valid = check.errors.is_empty();
    Ok(check)
}

/// The query with its parameters' defaults bound (by the script host's own substitution,
/// never by splicing), under `LIMIT rows`, through the store's graph-scoped door.
async fn preview(
    inv: &Invocation<'_>,
    door: &SparqlDoor,
    text: &str,
    analysis: &script_sparql::Analysis,
    grounding: &Grounding,
    rows: usize,
    warnings: &mut Vec<String>,
) -> std::result::Result<(Preview, Vec<String>), String> {
    let mut values: BTreeMap<String, Value> = BTreeMap::new();
    for p in &analysis.parameters {
        match &p.default {
            Some(d) => {
                values.insert(p.name.clone(), p.value(d).map_err(|e| detail(&e))?);
            }
            None => warnings.push(format!(
                "the preview runs with ?{} unbound: it has no default",
                p.name
            )),
        }
    }
    let bound = script_sparql::bind(text, door, &values).map_err(|e| detail(&e))?;
    let mut dataset = bound.graphs.clone();
    if bound.any_graph {
        dataset.extend(
            grounding
                .graphs
                .items
                .iter()
                .map(|s| s.graph.clone())
                .filter(|g| inv.capability.allows(&script_sparql::cap_read_graph(g))),
        );
    }
    if dataset.is_empty() {
        return Err("it reads no graph this caller may read".to_string());
    }
    let limited = limited(&bound.text, rows).map_err(|e| detail(&e))?;
    let face = match bound.form {
        Form::Select | Form::Ask => SPARQL_JSON,
        _ => NTRIPLES,
    };
    let graphs = dataset.iter().cloned().collect::<Vec<_>>().join(" ");
    let request_for = |query: &str, form: Form, face: &str| {
        request(
            Verb::Source,
            &door.query_iri(form),
            &[("query", query), ("graph", &graphs), ("as", face)],
        )
        .map_err(|e| e.to_string())
    };
    let rep = inv
        .issue(request_for(&limited, bound.form, face)?)
        .await
        .map_err(|e| e.to_string())?;
    let mut preview = Preview {
        limit: rows,
        graphs: dataset.into_iter().collect(),
        columns: Vec::new(),
        rows: Vec::new(),
        boolean: None,
        triples: Vec::new(),
    };
    if face == SPARQL_JSON {
        let results: Results = serde_json::from_slice(&rep.bytes)
            .map_err(|e| format!("the store's answer was not SPARQL JSON results: {e}"))?;
        preview.boolean = results.boolean;
        preview.columns = results.head.vars;
        if let Some(set) = results.results {
            preview.rows = set
                .bindings
                .into_iter()
                .take(rows)
                .map(|b| {
                    let row = Row(b);
                    preview
                        .columns
                        .iter()
                        .map(|c| row.ntriples(c).unwrap_or_default())
                        .collect()
                })
                .collect();
        }
    } else {
        preview.triples = String::from_utf8_lossy(&rep.bytes)
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .take(rows)
            .map(str::to_string)
            .collect();
    }
    if !preview.is_empty() {
        return Ok((preview, Vec::new()));
    }

    // Empty: run each triple pattern alone, in its GRAPH, under the same LIMIT and over the
    // same dataset, and name the ones that match nothing. Evidence for a person and for a
    // repair, never a verdict: an answer can be empty for good reason.
    let patterns = Probe::all(&bound.text);
    let probed: Vec<&Probe> = patterns.iter().take(MAX_PROBES).collect();
    let requests = probed
        .iter()
        .map(|p| request_for(&p.query(rows), Form::Select, SPARQL_JSON))
        .collect::<std::result::Result<Vec<_>, String>>()?;
    let mut unmatched = Vec::new();
    for (probe, result) in probed.iter().zip(inv.fan_out(requests).await) {
        // A probe the store would not run says nothing either way: it is left out.
        let Ok(rep) = result else { continue };
        let Ok(results) = serde_json::from_slice::<Results>(&rep.bytes) else {
            continue;
        };
        if results.results.is_some_and(|set| set.bindings.is_empty()) {
            unmatched.push(probe.text.clone());
        }
    }
    unmatched.dedup();
    let past = if patterns.len() > probed.len() {
        format!(
            " (the first {} of its {} patterns were probed)",
            probed.len(),
            patterns.len()
        )
    } else {
        String::new()
    };
    warnings.push(if unmatched.is_empty() {
        format!(
            "{EMPTY_PREVIEW}: no row matches, yet each of its {} triple patterns matches \
             something on its own{past}, so it is their combination (a join, a FILTER or a \
             parameter's default) that matches nothing",
            probed.len()
        )
    } else {
        format!(
            "{EMPTY_PREVIEW}: no row matches, and these triple patterns match nothing on \
             their own in the graphs it reads{past}: {}. A string written where the data \
             holds an IRI (or the reverse), or a literal of another datatype, matches \
             nothing: compare each with the kind of value its predicate takes",
            unmatched.join("; ")
        )
    });
    Ok((preview, unmatched))
}

/// One triple pattern of a query, in the GRAPH it is read in, to be run alone.
struct Probe {
    graph: Option<NamedNodePattern>,
    pattern: GraphPattern,
    /// The pattern as SPARQL, in its GRAPH: what a person and a model are shown.
    text: String,
}

impl Probe {
    /// Every triple pattern (and property path) a solution must match, in order: the WHERE
    /// clause's basic graph patterns, both sides of a join or union, the left of an
    /// OPTIONAL or MINUS (their right sides cannot empty an answer), and inside every
    /// GRAPH, filter, projection and modifier. Not walked: SERVICE (refused before this),
    /// VALUES, and `FILTER EXISTS` patterns.
    fn all(text: &str) -> Vec<Probe> {
        let mut out = Vec::new();
        if let Ok(query) = SparqlParser::new().parse_query(text) {
            match &query {
                Query::Select { pattern, .. }
                | Query::Ask { pattern, .. }
                | Query::Construct { pattern, .. }
                | Query::Describe { pattern, .. } => Probe::walk(pattern, None, &mut out),
            }
        }
        out
    }

    fn walk(p: &GraphPattern, graph: Option<&NamedNodePattern>, out: &mut Vec<Probe>) {
        match p {
            GraphPattern::Bgp { patterns } => {
                for t in patterns {
                    out.push(Probe::new(
                        graph,
                        GraphPattern::Bgp {
                            patterns: vec![t.clone()],
                        },
                        t.to_string(),
                    ));
                }
            }
            GraphPattern::Path {
                subject,
                path,
                object,
            } => out.push(Probe::new(
                graph,
                p.clone(),
                format!("{subject} {path} {object}"),
            )),
            GraphPattern::Join { left, right } | GraphPattern::Union { left, right } => {
                Probe::walk(left, graph, out);
                Probe::walk(right, graph, out);
            }
            GraphPattern::LeftJoin { left, .. } | GraphPattern::Minus { left, .. } => {
                Probe::walk(left, graph, out)
            }
            GraphPattern::Graph { name, inner } => Probe::walk(inner, Some(name), out),
            GraphPattern::Filter { inner, .. }
            | GraphPattern::Extend { inner, .. }
            | GraphPattern::OrderBy { inner, .. }
            | GraphPattern::Project { inner, .. }
            | GraphPattern::Distinct { inner }
            | GraphPattern::Reduced { inner }
            | GraphPattern::Slice { inner, .. }
            | GraphPattern::Group { inner, .. } => Probe::walk(inner, graph, out),
            // SERVICE is refused by the analysis; VALUES names no pattern; LATERAL (a
            // feature a sibling's build may unify in) is refused by the analysis too.
            #[allow(unreachable_patterns)] // feature unification can add variants
            _ => {}
        }
    }

    fn new(graph: Option<&NamedNodePattern>, pattern: GraphPattern, text: String) -> Probe {
        let text = match graph {
            Some(g) => format!("GRAPH {g} {{ {text} }}"),
            None => text,
        };
        Probe {
            graph: graph.cloned(),
            pattern,
            text,
        }
    }

    /// `SELECT * WHERE { [GRAPH g] { pattern } } LIMIT rows`, serialized by spargebra.
    fn query(&self, rows: usize) -> String {
        let inner = match &self.graph {
            Some(name) => GraphPattern::Graph {
                name: name.clone(),
                inner: Box::new(self.pattern.clone()),
            },
            None => self.pattern.clone(),
        };
        Query::Select {
            dataset: None,
            pattern: GraphPattern::Slice {
                inner: Box::new(inner),
                start: 0,
                length: Some(rows.max(1)),
            },
            base_iri: None,
        }
        .to_string()
    }
}

/// SPARQL 1.1 JSON results, as much of them as a preview reads.
#[derive(Deserialize)]
struct Results {
    #[serde(default)]
    head: Head,
    results: Option<Bindings>,
    boolean: Option<bool>,
}

#[derive(Deserialize, Default)]
struct Head {
    #[serde(default)]
    vars: Vec<String>,
}

#[derive(Deserialize)]
struct Bindings {
    bindings: Vec<BTreeMap<String, ResultTerm>>,
}

/// `text` (a bound query with no dataset clauses) under `LIMIT rows`: an existing smaller
/// limit stands. Rewritten in the algebra and serialized by spargebra, as the binding was.
fn limited(text: &str, rows: usize) -> Result<String> {
    limits::check_sparql(text, "content")?;
    limits::on_sparql_stack(text, || {
        let mut query =
            SparqlParser::new()
                .parse_query(text)
                .map_err(|e| Error::InvalidArgument {
                    name: "content".to_string(),
                    detail: e.to_string(),
                })?;
        match &mut query {
            Query::Select { pattern, .. }
            | Query::Construct { pattern, .. }
            | Query::Describe { pattern, .. } => {
                if let GraphPattern::Slice { length, .. } = pattern {
                    *length = Some(length.map_or(rows, |l| l.min(rows)));
                } else {
                    let inner = std::mem::replace(pattern, GraphPattern::Bgp { patterns: vec![] });
                    *pattern = GraphPattern::Slice {
                        inner: Box::new(inner),
                        start: 0,
                        length: Some(rows),
                    };
                }
            }
            Query::Ask { .. } => {}
        }
        Ok(query.to_string())
    })
}

/// The predicates and classes a query names in its WHERE patterns.
///
/// Not walked: a CONSTRUCT template (what it writes, not what it reads), `FILTER EXISTS`
/// patterns, a negated property set, and IRIs in expressions or in subject and object
/// positions (instances, which the grounding samples but does not list).
#[derive(Default)]
struct Named {
    predicates: BTreeSet<String>,
    classes: BTreeSet<String>,
}

impl Named {
    /// The text has already passed the analysis, so it parses; a text that does not
    /// names nothing.
    fn of(text: &str) -> Named {
        let mut named = Named::default();
        if let Ok(query) = SparqlParser::new().parse_query(text) {
            match &query {
                Query::Select { pattern, .. }
                | Query::Ask { pattern, .. }
                | Query::Construct { pattern, .. }
                | Query::Describe { pattern, .. } => named.pattern(pattern),
            }
        }
        named
    }

    fn triple(&mut self, t: &TriplePattern) {
        if let NamedNodePattern::NamedNode(p) = &t.predicate {
            self.predicates.insert(p.as_str().to_string());
            if p.as_str() == RDF_TYPE {
                if let TermPattern::NamedNode(c) = &t.object {
                    self.classes.insert(c.as_str().to_string());
                }
            }
        }
    }

    fn path(&mut self, p: &PropertyPathExpression) {
        match p {
            PropertyPathExpression::NamedNode(n) => {
                self.predicates.insert(n.as_str().to_string());
            }
            PropertyPathExpression::Reverse(a)
            | PropertyPathExpression::ZeroOrMore(a)
            | PropertyPathExpression::OneOrMore(a)
            | PropertyPathExpression::ZeroOrOne(a) => self.path(a),
            PropertyPathExpression::Sequence(a, b) | PropertyPathExpression::Alternative(a, b) => {
                self.path(a);
                self.path(b);
            }
            PropertyPathExpression::NegatedPropertySet(_) => {}
        }
    }

    fn pattern(&mut self, p: &GraphPattern) {
        match p {
            GraphPattern::Bgp { patterns } => {
                for t in patterns {
                    self.triple(t);
                }
            }
            GraphPattern::Path { path, .. } => self.path(path),
            GraphPattern::Join { left, right }
            | GraphPattern::Union { left, right }
            | GraphPattern::Minus { left, right }
            | GraphPattern::LeftJoin { left, right, .. } => {
                self.pattern(left);
                self.pattern(right);
            }
            GraphPattern::Filter { inner, .. }
            | GraphPattern::Graph { inner, .. }
            | GraphPattern::Extend { inner, .. }
            | GraphPattern::OrderBy { inner, .. }
            | GraphPattern::Project { inner, .. }
            | GraphPattern::Distinct { inner }
            | GraphPattern::Reduced { inner }
            | GraphPattern::Slice { inner, .. }
            | GraphPattern::Group { inner, .. }
            | GraphPattern::Service { inner, .. } => self.pattern(inner),
            // VALUES names no predicate; LATERAL (a feature a sibling's build may unify in)
            // is refused by the analysis before this walk is reached.
            #[allow(unreachable_patterns)] // feature unification can add variants
            _ => {}
        }
    }
}

// ------------------------------------------------------------------------- the endpoint

pub(crate) struct CheckEndpoint {
    pub(crate) config: std::sync::Arc<SpaceConfig>,
}

/// An optional inline string argument; present but unreadable is refused.
pub(crate) fn optional<'a>(inv: &'a Invocation<'_>, name: &str) -> Result<Option<&'a str>> {
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

/// A count argument: absent is `default`; present must be a whole number up to `max`.
pub(crate) fn count(inv: &Invocation<'_>, name: &str, default: usize, max: usize) -> Result<usize> {
    match optional(inv, name)? {
        None => Ok(default.min(max)),
        Some(text) => match text.parse::<usize>() {
            Ok(n) if n <= max => Ok(n),
            _ => Err(Error::InvalidArgument {
                name: name.to_string(),
                detail: format!("`{text}` is not a whole number from 0 to {max}"),
            }),
        },
    }
}

#[async_trait]
impl Endpoint for CheckEndpoint {
    async fn invoke(&self, inv: &Invocation<'_>) -> Result<Representation> {
        if inv.request.verb != Verb::Source {
            return Err(Error::Endpoint(format!(
                "nl-sparql-check answers Source, not {:?}",
                inv.request.verb
            )));
        }
        let want = match optional(inv, "as")? {
            None | Some(PLAIN) => PLAIN,
            Some(JSON) => JSON,
            Some(other) => {
                return Err(Error::InvalidArgument {
                    name: "as".to_string(),
                    detail: format!("`{other}`: this resource answers {PLAIN} or {JSON}"),
                })
            }
        };
        // The text is read as given (not trimmed): a parameter block is line-sensitive.
        let text = match inv.request.args.get("content") {
            Some(ArgRef::Inline(bytes)) => {
                std::str::from_utf8(bytes).map_err(|_| Error::InvalidArgument {
                    name: "content".to_string(),
                    detail: "not UTF-8".to_string(),
                })?
            }
            Some(_) => {
                return Err(Error::InvalidArgument {
                    name: "content".to_string(),
                    detail: "pass the query inline (or pipe it), not by reference".to_string(),
                })
            }
            None => return Err(Error::MissingArgument("content".to_string())),
        };
        let focus = optional(inv, "focus")?;
        let rows = count(inv, "preview", self.config.preview_rows, MAX_PREVIEW)?;
        let check = check(inv, &self.config, text, focus, rows).await?;
        let (repr_type, bytes) = if want == JSON {
            let mut bytes = serde_json::to_vec_pretty(&check)
                .map_err(|e| Error::Endpoint(format!("serializing the check: {e}")))?;
            bytes.push(b'\n');
            (ReprType::new(JSON), bytes)
        } else {
            (
                ReprType::new(PLAIN).with_param("charset", "utf-8"),
                check.to_text().into_bytes(),
            )
        };
        // A pure function of the text, the grounding and the store: cacheable as far as
        // those are, keyed on the caller's capability by the kernel.
        Ok(Representation::new(repr_type, bytes).cacheable())
    }

    fn name(&self) -> &str {
        "nl-sparql-check"
    }

    fn describe(&self) -> Description {
        Description::new("nl-sparql-check")
            .title("Check a SPARQL draft")
            .summary(
                "Validate a SPARQL query mechanically, under the CALLER's capability, the way \
                 urn:nl:sparql validates each draft: bounded and parsed by the script host's \
                 analysis (form, declared parameters, dataset); its DERIVED authority \
                 against the caller's (a graph the caller cannot read is refused in the same \
                 words as one that does not exist); every predicate and class it names \
                 against the grounding; and a dry run under a small LIMIT, which, when it \
                 is empty, names the triple patterns that match nothing on their own. Every \
                 finding is a value in the answer, never a refusal.",
            )
            .verb(Verb::Meta)
            .action(
                ActionSpec::new(Verb::Source)
                    .summary("The check of one query.")
                    .input(
                        ArgSpec::new("content")
                            .summary("The query, with its `# @param` block (piped, or by name).")
                            .class(XSD_STRING),
                    )
                    .input(
                        ArgSpec::new("focus")
                            .summary(
                                "The grounding's focus (a graph IRI or a topic word), as \
                                 urn:nl:sparql's `graph=` passes it.",
                            )
                            .class(XSD_STRING)
                            .optional(),
                    )
                    .input(
                        ArgSpec::new("preview")
                            .summary("How many rows (or triples) the dry run previews; 0 skips it.")
                            .class(XSD_INTEGER)
                            .default_value("5")
                            .optional(),
                    )
                    .input(
                        ArgSpec::new("as")
                            .summary("A plain summary (default) or the JSON form.")
                            .class(XSD_STRING)
                            .one_of([PLAIN, JSON])
                            .default_value(PLAIN)
                            .optional(),
                    )
                    .output(PLAIN)
                    .output(JSON),
            )
    }
}
