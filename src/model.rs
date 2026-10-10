//! The grounding as a value: what `urn:nl:grounding` answers, before it is rendered.
//!
//! The JSON face is this module's serde form, field for field, so a drafter in this crate
//! (or anywhere else) reads it back into these types. The Turtle face says the same
//! things in RDF; `render` holds that mapping.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The version of the JSON face. Raised when a field changes meaning or disappears;
/// a new optional field does not raise it. 2: an action's `action` (the old
/// `urn:ikigai:endpoint:{id}:action:{verb}`, which every copy of an id shared) became
/// `match` and `contract`, core 0.1.91's two identities for it (ledger #1034).
pub const SCHEMA: u32 = 2;

/// What a drafter may know, under one caller's capability.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grounding {
    /// [`SCHEMA`].
    pub schema: u32,
    /// `urn:nl:grounding:sha256:…`: this grounding, named by its content.
    pub iri: String,
    /// `sha256:` and 64 lowercase hex digits over the parts' identities and the focus.
    /// What a draft cites.
    pub identity: String,
    /// The graph IRI or topic word the grounding was narrowed to.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub focus: Option<String>,
    /// The actions the caller may take: the manifold under its capability.
    pub actions: Part<Action>,
    /// The `ik:` vocabulary's classes and properties.
    pub vocabulary: Part<Term>,
    /// The shape of each graph the caller may read.
    pub graphs: Part<GraphShape>,
    /// Published scripts the caller may read, as worked examples.
    pub examples: Part<Example>,
}

/// One part of a grounding: what it holds, where it came from, and how much of what was
/// offered it shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Part<T> {
    /// The resource the part was derived from (`urn:kernel:actions`, `urn:ikigai:vocab`,
    /// `urn:iki:store:graphs`, `urn:script:catalog`). `None` when the part came from no
    /// single resource: configured examples.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub source: Option<String>,
    /// `sha256:` over this part's canonical JSON (everything but this field).
    pub identity: String,
    /// The source's own version, when it states one (the vocabulary's `owl:versionInfo`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub version: Option<String>,
    /// How many items this part holds.
    pub shown: usize,
    /// How many the source offered this caller, before focus and bounds. `shown < of`
    /// is "first N of M", never a silent truncation.
    pub of: usize,
    /// Why the part is empty or narrowed, in a sentence, when it is.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub note: Option<String>,
    /// The items.
    pub items: Vec<T>,
}

impl<T: Serialize> Part<T> {
    /// A part over `items`, its identity computed.
    pub(crate) fn new(
        source: Option<String>,
        version: Option<String>,
        of: usize,
        note: Option<String>,
        items: Vec<T>,
    ) -> Self {
        let mut part = Part {
            source,
            identity: String::new(),
            version,
            shown: items.len(),
            of,
            note,
            items,
        };
        part.identity = digest(&part);
        part
    }
}

/// One action the caller may take: one verb at one door, with its contract.
///
/// It has two identities, both core's and neither minted here (ledger #948): the
/// **match**, one per door and verb, and the **contract** it satisfies, content-addressed,
/// which two doors serving identical contracts share. An endpoint's id names neither: a
/// mounted copy and a second door carry the same id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    /// `urn:ikigai:match:{verb}:{pattern}`, `ikigai_core::match_iri`: the subject of this
    /// door and verb's row in the manifold (`urn:kernel:actions`).
    #[serde(rename = "match")]
    pub match_iri: String,
    /// `urn:ikigai:contract:{id}:{verb}:b3:{hex}`, `ActionSpec::contract_iri`: the
    /// catalog's node for this verb's contract, and the manifold's `ik:contract`.
    pub contract: String,
    /// The IRI to invoke, for an exact binding.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub endpoint: Option<String>,
    /// The URI template to fill, for a template binding.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub template: Option<String>,
    /// The endpoint's description id.
    pub id: String,
    /// The endpoint's title.
    pub title: String,
    /// The endpoint's summary.
    pub summary: String,
    /// `source`, `sink`, `exists` or `delete`.
    pub verb: String,
    /// What this verb of it does.
    pub action_summary: String,
    /// The grants it declares (all of them, each held by this caller or a family it holds).
    pub requires: Vec<String>,
    /// Its declared arguments.
    pub inputs: Vec<Input>,
    /// The media types it can answer.
    pub outputs: Vec<String>,
}

/// One declared argument.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Input {
    /// Its name.
    pub name: String,
    /// Whether a call must supply it.
    pub required: bool,
    /// Whether it is a URI-template variable (filled into the IRI) rather than an argument.
    pub binding: bool,
    /// Its `rdfs:Class` (an entity) or XSD datatype (a scalar).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub class: Option<String>,
    /// Its default.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub default: Option<String>,
    /// The values it admits, when it is an enumeration.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub one_of: Vec<String>,
    /// What it is.
    pub summary: String,
}

/// One vocabulary term.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Term {
    /// The term's IRI.
    pub iri: String,
    /// `class` or `property`.
    pub kind: String,
    /// Its `rdfs:label`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub label: Option<String>,
    /// Its `rdfs:comment`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub comment: Option<String>,
    /// Its `rdfs:domain`s.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub domain: Vec<String>,
    /// Its `rdfs:range`s.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub range: Vec<String>,
    /// Its `rdfs:subClassOf` or `rdfs:subPropertyOf`.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub broader: Vec<String>,
}

/// The shape of one named graph the caller may read: VoID-like totals, the classes and
/// predicates it uses, and a few triples.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphShape {
    /// The graph's IRI.
    pub graph: String,
    /// `sha256:` over this shape's canonical JSON (everything but this field).
    pub identity: String,
    /// Triples in it.
    pub triples: u64,
    /// Distinct subjects.
    pub subjects: u64,
    /// Distinct predicates.
    pub properties: u64,
    /// Distinct objects.
    pub objects: u64,
    /// Distinct classes (`rdf:type` objects).
    pub classes: u64,
    /// The classes, most instances first: the first `class_partitions.len()` of `classes`.
    pub class_partitions: Vec<Partition>,
    /// The predicates, most triples first: the first `property_partitions.len()` of
    /// `properties`.
    pub property_partitions: Vec<Partition>,
    /// A few triples as N-Triples lines: the first `samples.len()` of `triples`.
    pub samples: Vec<String>,
    /// Why the shape could not be read in full, when it could not.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub error: Option<String>,
}

/// A class or predicate with how often it occurs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Partition {
    /// The class or predicate IRI.
    pub iri: String,
    /// Instances of the class, or triples with the predicate.
    pub count: u64,
}

/// A published script the caller may read, as a worked example.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Example {
    /// `urn:script:{name}`.
    pub iri: String,
    /// The script's name.
    pub name: String,
    /// Its head version, `sha256:…`, as the script host named it.
    pub version: String,
    /// `urn:script:{name}:version:{digest}`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub version_iri: Option<String>,
    /// Its language (`lisp`, `sparql`, `plan`).
    pub language: String,
    /// Whether it is public.
    pub public: bool,
    /// The grants it declares.
    pub requires: Vec<String>,
    /// Its source text.
    pub source: String,
    /// Its declared parameters, exactly as the script host states them, when it states
    /// any. Passed through, not interpreted: the host owns that shape.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub parameters: Option<serde_json::Value>,
}

/// `sha256:` and 64 lowercase hex digits over a value's canonical JSON, with any top-level
/// `identity` member removed first (so a value can carry its own identity).
///
/// Canonical because every map in this module is a struct (fixed field order) and every
/// list is built in a deterministic order, so equal values serialize to equal bytes.
pub(crate) fn digest<T: Serialize>(value: &T) -> String {
    let mut json = serde_json::to_value(value).unwrap_or(serde_json::Value::Null);
    if let Some(object) = json.as_object_mut() {
        object.remove("identity");
    }
    sha256(&serde_json::to_vec(&json).unwrap_or_default())
}

/// `sha256:` and 64 lowercase hex digits over `bytes`: the ecosystem's tagged digest.
///
/// ```
/// let d = ikigai_nl::model::sha256(b"");
/// assert_eq!(
///     d,
///     "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
/// );
/// ```
pub fn sha256(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes);
    let mut out = String::with_capacity(7 + 64);
    out.push_str("sha256:");
    for byte in hash {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
