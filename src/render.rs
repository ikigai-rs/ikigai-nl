//! The grounding's Turtle face.
//!
//! Every triple goes through `oxrdf` terms and `oxttl`'s serializer, so nothing here
//! escapes anything by hand: the only correct escaper for a grammar is the one that owns
//! it. No blank nodes: every node is an IRI that is stable across groundings
//! (`urn:ikigai:endpoint:…` for actions, as the catalog names them; `urn:nl:shape:…` for a
//! graph's shape), so two groundings diff and union.

use std::collections::HashSet;

use ikigai_core::{ActionSpec, ArgSpec, Description, InputSource, Verb};
use oxrdf::{Literal, NamedNode, Triple};

use crate::model::{sha256, Action, GraphShape, Grounding, Part};
use crate::NS;

const IK: &str = "https://ikigai-rs.dev/ns#";
const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const RDFS: &str = "http://www.w3.org/2000/01/rdf-schema#";
const OWL: &str = "http://www.w3.org/2002/07/owl#";
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
const DCTERMS: &str = "http://purl.org/dc/terms/";
const PROV: &str = "http://www.w3.org/ns/prov#";
const SCHEMA: &str = "http://schema.org/";
/// VoID, the W3C interest-group vocabulary for describing datasets.
pub const VOID: &str = "http://rdfs.org/ns/void#";

const PREFIXES: [(&str, &str); 10] = [
    ("ik", IK),
    ("nl", NS),
    ("void", VOID),
    ("dcterms", DCTERMS),
    ("prov", PROV),
    ("schema", SCHEMA),
    ("rdf", RDF),
    ("rdfs", RDFS),
    ("owl", OWL),
    ("xsd", XSD),
];

/// Triples in the order they were added, each once.
#[derive(Default)]
struct Graph {
    triples: Vec<Triple>,
    seen: HashSet<Triple>,
}

impl Graph {
    fn push(&mut self, subject: &str, predicate: &str, object: oxrdf::Term) {
        let (Ok(subject), Ok(predicate)) = (NamedNode::new(subject), NamedNode::new(predicate))
        else {
            return;
        };
        let triple = Triple::new(subject, predicate, object);
        if self.seen.insert(triple.clone()) {
            self.triples.push(triple);
        }
    }

    fn iri(&mut self, subject: &str, predicate: &str, object: &str) {
        if let Ok(object) = NamedNode::new(object) {
            self.push(subject, predicate, object.into());
        }
    }

    fn lit(&mut self, subject: &str, predicate: &str, value: &str) {
        self.push(
            subject,
            predicate,
            Literal::new_simple_literal(value).into(),
        );
    }

    fn int(&mut self, subject: &str, predicate: &str, value: u64) {
        self.push(subject, predicate, Literal::from(value).into());
    }

    fn a(&mut self, subject: &str, class: &str) {
        self.iri(subject, &format!("{RDF}type"), class);
    }

    /// Parse a Turtle document produced elsewhere in the ecosystem and keep its triples.
    fn turtle(&mut self, text: &str) {
        for triple in oxttl::TurtleParser::new().for_slice(text).flatten() {
            if self.seen.insert(triple.clone()) {
                self.triples.push(triple);
            }
        }
    }

    fn serialize(self) -> Vec<u8> {
        let mut serializer = oxttl::TurtleSerializer::new();
        for (prefix, iri) in PREFIXES {
            serializer = serializer
                .with_prefix(prefix, iri)
                .expect("the prefixes are constant, valid IRIs");
        }
        // Each subject's triples together, subjects in the order they first appeared, so
        // the serializer writes one block per node (it groups only consecutive triples).
        let mut first: std::collections::HashMap<&oxrdf::NamedOrBlankNode, usize> =
            std::collections::HashMap::new();
        for (index, triple) in self.triples.iter().enumerate() {
            first.entry(&triple.subject).or_insert(index);
        }
        let mut ordered: Vec<&Triple> = self.triples.iter().collect();
        ordered.sort_by_key(|triple| first[&triple.subject]);
        let mut writer = serializer.for_writer(Vec::new());
        for triple in ordered {
            writer
                .serialize_triple(triple)
                .expect("serializing into a Vec cannot fail");
        }
        writer.finish().expect("serializing into a Vec cannot fail")
    }
}

/// The first 16 hex digits of the sha256 of `text`: a short, stable IRI segment for a
/// value (a graph or class IRI) that would be unwieldy spelled out inside another IRI.
fn short(text: &str) -> String {
    sha256(text.as_bytes())["sha256:".len()..][..16].to_string()
}

/// The Turtle face of a grounding.
pub(crate) fn turtle(grounding: &Grounding) -> Vec<u8> {
    let mut g = Graph::default();
    let root = grounding.iri.as_str();
    g.a(root, &format!("{NS}Grounding"));
    g.lit(root, &format!("{DCTERMS}identifier"), &grounding.identity);
    if let Some(focus) = &grounding.focus {
        g.lit(root, &format!("{NS}focus"), focus);
    }

    let actions = part(&mut g, root, "actions", &grounding.actions);
    for action in &grounding.actions.items {
        g.iri(&actions, &format!("{DCTERMS}hasPart"), &action.action);
        render_action(&mut g, action);
    }

    let vocabulary = part(&mut g, root, "vocabulary", &grounding.vocabulary);
    for term in &grounding.vocabulary.items {
        let s = term.iri.as_str();
        g.iri(&vocabulary, &format!("{DCTERMS}hasPart"), s);
        let (class, broader) = if term.kind == "class" {
            (format!("{RDFS}Class"), format!("{RDFS}subClassOf"))
        } else {
            (format!("{RDF}Property"), format!("{RDFS}subPropertyOf"))
        };
        g.a(s, &class);
        if let Some(label) = &term.label {
            g.lit(s, &format!("{RDFS}label"), label);
        }
        if let Some(comment) = &term.comment {
            g.lit(s, &format!("{RDFS}comment"), comment);
        }
        for domain in &term.domain {
            g.iri(s, &format!("{RDFS}domain"), domain);
        }
        for range in &term.range {
            g.iri(s, &format!("{RDFS}range"), range);
        }
        for up in &term.broader {
            g.iri(s, &broader, up);
        }
    }

    let graphs = part(&mut g, root, "graphs", &grounding.graphs);
    for shape in &grounding.graphs.items {
        let node = format!("urn:nl:shape:{}", short(&shape.graph));
        g.iri(&graphs, &format!("{DCTERMS}hasPart"), &node);
        render_shape(&mut g, &node, shape);
    }

    let examples = part(&mut g, root, "examples", &grounding.examples);
    for example in &grounding.examples.items {
        let s = example.iri.as_str();
        g.iri(&examples, &format!("{DCTERMS}hasPart"), s);
        g.a(s, &format!("{SCHEMA}SoftwareSourceCode"));
        g.lit(s, &format!("{SCHEMA}name"), &example.name);
        g.lit(s, &format!("{SCHEMA}version"), &example.version);
        if let Some(version) = &example.version_iri {
            g.iri(s, &format!("{DCTERMS}hasVersion"), version);
        }
        g.lit(
            s,
            &format!("{SCHEMA}programmingLanguage"),
            &example.language,
        );
        g.lit(s, &format!("{SCHEMA}text"), &example.source);
        for scope in &example.requires {
            g.iri(s, &format!("{IK}requires"), scope);
        }
    }
    g.serialize()
}

/// A part's own node, `{grounding}:{name}`, hung from the grounding.
fn part<T>(g: &mut Graph, root: &str, name: &str, part: &Part<T>) -> String {
    let node = format!("{root}:{name}");
    g.iri(root, &format!("{DCTERMS}hasPart"), &node);
    g.a(&node, &format!("{NS}Part"));
    g.lit(&node, &format!("{RDFS}label"), name);
    if let Some(source) = &part.source {
        g.iri(&node, &format!("{PROV}wasDerivedFrom"), source);
    }
    g.lit(&node, &format!("{DCTERMS}identifier"), &part.identity);
    if let Some(version) = &part.version {
        g.lit(&node, &format!("{OWL}versionInfo"), version);
    }
    g.int(&node, &format!("{NS}shown"), part.shown as u64);
    g.int(&node, &format!("{NS}of"), part.of as u64);
    if let Some(note) = &part.note {
        g.lit(&node, &format!("{RDFS}comment"), note);
    }
    node
}

/// One action as the catalog states it (`ikigai_vocab::to_turtle` over a description
/// holding only this verb, so the IRIs and terms are the catalog's own), plus where to
/// invoke it.
fn render_action(g: &mut Graph, action: &Action) {
    let verb = match action.verb.as_str() {
        "sink" => Verb::Sink,
        "exists" => Verb::Exists,
        "delete" => Verb::Delete,
        _ => Verb::Source,
    };
    let mut spec = ActionSpec::new(verb).summary(&action.action_summary);
    for input in &action.inputs {
        let mut arg = ArgSpec::new(&input.name).summary(&input.summary);
        arg.required = input.required;
        arg.class = input.class.clone();
        arg.default = input.default.clone();
        arg.one_of = input.one_of.clone();
        if input.binding {
            arg.source = InputSource::Binding;
        }
        spec = spec.input(arg);
    }
    for output in &action.outputs {
        spec = spec.output(output);
    }
    for scope in &action.requires {
        spec = spec.requires(scope);
    }
    let description = Description::new(&action.id)
        .title(&action.title)
        .summary(&action.summary)
        .verb(verb)
        .action(spec);
    g.turtle(&ikigai_vocab::to_turtle(&description));
    match (&action.endpoint, &action.template) {
        (Some(endpoint), _) => g.iri(&action.action, &format!("{IK}endpoint"), endpoint),
        (None, Some(template)) => g.lit(&action.action, &format!("{IK}template"), template),
        (None, None) => {}
    }
}

fn render_shape(g: &mut Graph, node: &str, shape: &GraphShape) {
    g.a(node, &format!("{VOID}Dataset"));
    g.iri(node, &format!("{PROV}wasDerivedFrom"), &shape.graph);
    g.lit(node, &format!("{DCTERMS}identifier"), &shape.identity);
    g.int(node, &format!("{VOID}triples"), shape.triples);
    g.int(node, &format!("{VOID}distinctSubjects"), shape.subjects);
    g.int(node, &format!("{VOID}properties"), shape.properties);
    g.int(node, &format!("{VOID}distinctObjects"), shape.objects);
    g.int(node, &format!("{VOID}classes"), shape.classes);
    g.int(
        node,
        &format!("{NS}classesShown"),
        shape.class_partitions.len() as u64,
    );
    g.int(
        node,
        &format!("{NS}propertiesShown"),
        shape.property_partitions.len() as u64,
    );
    g.int(
        node,
        &format!("{NS}samplesShown"),
        shape.samples.len() as u64,
    );
    for partition in &shape.class_partitions {
        let p = format!("{node}:class:{}", short(&partition.iri));
        g.iri(node, &format!("{VOID}classPartition"), &p);
        g.iri(&p, &format!("{VOID}class"), &partition.iri);
        g.int(&p, &format!("{VOID}entities"), partition.count);
    }
    for partition in &shape.property_partitions {
        let p = format!("{node}:property:{}", short(&partition.iri));
        g.iri(node, &format!("{VOID}propertyPartition"), &p);
        g.iri(&p, &format!("{VOID}property"), &partition.iri);
        g.int(&p, &format!("{VOID}triples"), partition.count);
    }
    for sample in &shape.samples {
        g.lit(node, &format!("{NS}sample"), sample);
    }
    if let Some(error) = &shape.error {
        g.lit(node, &format!("{RDFS}comment"), error);
    }
}
