# ikigai-nl

**Natural language to ikigai, with the LLM as a drafter and never an actor.** Before a
model drafts a query or a script for someone, it has to know what is real for *that
person*: what they may do, the words the system speaks, what their data looks like, and
what has worked before. This crate serves that as one resource, computed under the
caller's own capability, so a model can never be told about a graph, a script or an
action its user could not reach directly.

```text
urn:nl:grounding    Source    focus=  as=text/turtle|application/json
```

The drafters that consume it, `urn:nl:sparql` and `urn:nl:script`, come next in this
repo. They ground, draft in the most analyzable language that can do the job (a query,
then a plan, then Lisp), validate mechanically, and save a draft for a human to publish.

## What a grounding holds

It is a **composition**: it holds no state, and every part is a read through the kernel
under the caller's capability.

| part | read from | holds |
| --- | --- | --- |
| actions | `urn:kernel:actions` (once per verb), then `Meta as=application/json` of each offered endpoint | every action the caller may take: endpoint or template, verb, the grants it declares, its arguments (name, required, class, default, allowed values) and outputs |
| vocabulary | `urn:ikigai:vocab` | the `ik:` classes and properties, with labels, comments, domains and ranges |
| graphs | `urn:iki:store:graphs`, then five stored SPARQL queries per graph | VoID-like totals (triples, subjects, predicates, objects, classes), the classes and predicates used with counts, and a few sample triples |
| examples | `urn:script:catalog` then `urn:script:{name}` (or a configured list) | published scripts the caller may read: language, version, declared grants, source |

The test host's grounding for root, focused on one graph (abridged):

```text
source urn:nl:grounding focus=urn:example:ledger
<urn:nl:grounding:sha256:3fcb…> a nl:Grounding ;
    dcterms:identifier "sha256:3fcb…" ;
    nl:focus "urn:example:ledger" ;
    dcterms:hasPart <…:actions>, <…:vocabulary>, <…:graphs>, <…:examples> .
<…:graphs> a nl:Part ;
    prov:wasDerivedFrom <urn:iki:store:graphs> ;
    dcterms:identifier "sha256:2c75…" ;
    nl:shown 1 ; nl:of 2 ;
    rdfs:comment "1 of 2 graphs mention the focus `urn:example:ledger`" ;
    dcterms:hasPart <urn:nl:shape:1180ba9d89603646> .
<urn:nl:shape:1180ba9d89603646> a void:Dataset ;
    prov:wasDerivedFrom <urn:example:ledger> ;
    void:triples 6 ; void:classes 1 ; nl:samplesShown 5 ;
    void:classPartition <urn:nl:shape:1180ba9d89603646:class:210e637540fea61c> ;
    nl:sample "<urn:example:item:1> <http://purl.org/dc/terms/title> \"first\" ." .
```

The JSON face (`as=application/json`) is the same content as `ikigai_nl::Grounding`, so
a drafter in Rust reads it back into the type.

## The rules it keeps

- **Everything is under the caller's capability.** A graph the caller cannot read
  contributes nothing, not even its name: the graphs come from the store's own listing of
  what the caller may read, and a focus on an unreadable graph finds exactly what a focus
  on a graph that does not exist finds. The same holds for scripts, configured ones
  included. An anonymous caller gets an anonymous grounding (the actions anyone may take,
  and the vocabulary).
- **Nothing is read that would be refused.** A resource the caller's manifold does not
  offer (the store's listing, the script catalog) is not asked, because a refusal is
  never cached and asking would make the grounding uncacheable for that caller.
- **Citable.** Each part names the resource it came from and carries a sha256 identity
  over what it holds; each graph's shape carries its own; the vocabulary carries its
  `owl:versionInfo`; each example carries its script version digest. The grounding is
  named by the sha256 of its parts (`urn:nl:grounding:sha256:…`), so a draft records
  exactly which grounding it was drafted from, and two callers with the same view cite
  the same grounding.
- **Bounded and honest.** Every part says how many items it shows against how many were
  offered (`nl:shown` of `nl:of`), each graph says how many classes, predicates and
  samples it lists against its totals, and a part narrowed by a bound says so in a
  sentence. A grounding over the host's size bound is **refused** with its size, its
  bound and its contents counted, never truncated: narrow it with `focus=`.
- **Cached per capability.** The kernel keys the cache on the caller's capability and
  hangs the grounding from every thread its parts hang from, so a write to a graph it
  summarized, a change of bindings, or a republished example recomputes it.
- **Skolemized.** No blank nodes. Actions use the catalog's own IRIs
  (`urn:ikigai:endpoint:{id}:action:{verb}`) and terms, so the grounding joins the catalog
  graph; a graph's shape is `urn:nl:shape:{hash of the graph IRI}`, stable across
  groundings, so two groundings diff.

### `focus=`

A graph IRI or a topic word. Every part keeps the items that mention it,
case-insensitively; a graph also stays when a class or predicate it uses mentions it, and
a vocabulary term also stays when a kept graph uses it. So a focus on one graph keeps its
shape and the `ik:` terms it is written in, and graphs whose IRI mentions the focus are
summarized first, so a focus reaches its graph past the graph bound.

## Mounting it

A host library: no binary. A host mounts `space(config)` beside what it composes. The
kernel needs a JSON Meta renderer (`ikigai_vocab::TurtleRenderer`): each action's
arguments come from the endpoint's JSON description, and the grounding fails loudly
without one rather than offering actions with no arguments.

```rust,no_run
use std::sync::Arc;
use ikigai_core::{Fallback, Kernel, Space};
use ikigai_nl::{space, Examples, SpaceConfig};

let config = SpaceConfig::new()
    .max_bytes(1024 * 1024)   // refuse a grounding larger than this
    .max_graphs(32)           // summarize at most this many graphs
    .max_partitions(25)       // classes and predicates per graph
    .samples(5)               // sample triples per graph
    .max_examples(16)
    .examples(Examples::Catalog);
let root = Fallback::new(vec![
    Arc::new(space(config)) as Arc<dyn Space>,
    Arc::new(ikigai_vocab::space()) as Arc<dyn Space>,
    // …the store, the script host, and the rest of the host's space
]);
let kernel = Kernel::with_meta_renderer(Arc::new(root), Arc::new(ikigai_vocab::TurtleRenderer));
```

The grounding itself declares no grant: it discloses nothing its caller could not read
directly, and each resource it reads enforces its own.

### ⚠ Examples from the catalog make the grounding live

`ikigai-script`'s catalog is live (a run is recorded without a write through any name the
catalog could hang from), and a composite is only as cacheable as its least cacheable
part. So with the default `Examples::Catalog`, a caller who can read the catalog gets a
grounding recomputed on every read. Its parts stay cached (each manifold read,
description, store query and script head), so a recompute is an assembly rather than a
re-query, and `urn:kernel:uncached` names `urn:script:catalog` as the reason.
`Examples::Named(names)` reads the named scripts one by one instead; each read is cached
and cut when that script is republished, so the grounding stays cached. Name scripts every
caller may read (public ones): a caller holding `urn:cap:script:read:public` but not a
private script's grant is refused that one read, and a refusal is never cached.

## The graph queries are stored SPARQL

`src/queries/*.rq` are constant texts. The graph reaches them as a typed value through
the store's `bindings=` (`?g`), never as query text, and the only thing formatted into
them is a `LIMIT` the crate computes itself. They read through
`urn:iki:store:graph-select` when the caller holds the graph's grant and through
`urn:iki:store:select` for a broad reader, and hang from the store's write threads. When
`ikigai-script` gains SPARQL as a script language they become published scripts.

## Vocabulary

Actions are said with `ik:`, the vocabulary with `rdfs:`, each graph's shape with
[VoID](http://rdfs.org/ns/void#), examples with `schema:`, identity and origin with
`dcterms:` and `prov:`. The few terms nothing else has (`nl:Grounding`, `nl:Part`,
`nl:focus`, `nl:shown`, `nl:of`, `nl:sample`, `nl:samplesShown`, `nl:classesShown`,
`nl:propertiesShown`) are defined in `src/nl.ttl`, exported as `ikigai_nl::VOCABULARY`,
under `https://ikigai-rs.dev/ns/nl#`. A test holds the renderer to exactly that list.

## Not in this version

- **The drafters**, `urn:nl:sparql` and `urn:nl:script`: the next arcs.
- **Example parameters in Turtle.** A script document's `parameters` are passed through
  in the JSON face as the script host states them; the Turtle face does not state them
  yet.
- **A resolvable grounding IRI.** `urn:nl:grounding:sha256:…` names a grounding by its
  content; nothing stores one to resolve it by. A drafter that records the grounding it
  used will make it resolvable.
- **Per-graph invalidation.** The store hangs every read from its three write threads,
  not from the graph it read, so a write to any graph re-queries every graph's shape.

## License

MIT OR Apache-2.0.
