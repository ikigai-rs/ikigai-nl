# ikigai-nl

**Natural language to ikigai, with the LLM as a drafter and never an actor.** Before a
model drafts a query or a script for someone, it has to know what is real for *that
person*: what they may do, the words the system speaks, what their data looks like, and
what has worked before. This crate serves that as one resource, computed under the
caller's own capability, so a model can never be told about a graph, a script or an
action its user could not reach directly.

```text
urn:nl:grounding        Source   focus=  as=text/turtle|application/json
urn:nl:sparql           Sink     ask= (or piped)  graph=  examples=  save=  as=text/plain|application/json|text/turtle
urn:nl:sparql:check     Source   content= (or piped)  focus=  preview=  as=text/plain|application/json
urn:nl:prompt:{name}    Source   sparql | sparql-repair
```

`urn:nl:sparql` is the first drafter: it grounds, drafts a query, validates it
mechanically, repairs it within a bound, and saves a DRAFT script for a person to
publish. `urn:nl:script` (a plan, then Lisp) comes next.

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

## `urn:nl:sparql`: an ask to a draft query

```text
sink urn:nl:sparql ask="titles of the ledger's items" save=titles
urn:script:titles:version:sha256:… (a DRAFT of urn:script:titles: review it, then publish it with state=published)

SELECT ?item ?title WHERE { GRAPH <urn:example:ledger> { ?item <http://purl.org/dc/terms/title> ?title } } ORDER BY ?item

preview (LIMIT 5):
?item	?title
<urn:example:item:1>	"first"
<urn:example:item:2>	"second"
```

The model is a **drafter, never an actor**. Every step is a resource call under the
CALLER's capability, never more:

1. **Ground**: `urn:nl:grounding`, focused by `graph=`. The prompt states each graph's
   classes, predicates and samples, and the vocabulary terms those graphs use (all the
   focused terms, when `graph=` is given). The actions are left out: a query is not an
   action call, and the store's query doors are the host's choice, not the model's.
2. **Draft**: `urn:nl:prompt:sparql`, filled with the ask, the grounding and up to
   `examples=` published SPARQL scripts, sent to the host's LLM door (`SpaceConfig::llm`):
   the default backend at `urn:llm:ask`, or `urn:llm:select` under the host's `needs` or
   its escalation policy (`Escalation { after, needs }`: local first, a frontier model only
   after that many failed attempts). The model is asked for the query and, where the ask
   implies them, its parameters in ikigai-script's `# @param` form.
3. **Validate**: `urn:nl:sparql:check`, a resource of its own (so a person editing a draft
   can ask the same question):
   - bounded before the parser sees a byte and parsed, by the script host's own analysis
     (form, declared parameters, the dataset rules); an UPDATE is refused, a draft reads;
   - the capability the script host would DERIVE from the graphs the text names, against
     the caller's. ★ A graph the caller cannot read is refused in the same words as a
     graph that does not exist, so neither the caller nor the model learns which it was;
   - every predicate and class (`rdf:type` object) the query names, against the graphs it
     reads and the vocabulary: unknown is an error, or a warning when a graph's partitions
     were bounded, because then the grounding cannot know;
   - a dry run: the parameters' defaults bound into the algebra (never spliced), under
     `LIMIT preview_rows`, through the store's graph-scoped door.

   Every finding is a value in the answer, never a refusal, so the check caches.
4. **Repair**: a refused draft goes back with `urn:nl:prompt:sparql-repair`: the previous
   draft and exactly what was refused, nothing else. **The bound is 3 attempts** by
   default (`max_attempts`, at most `MAX_ATTEMPTS` = 8), and drafting also stops the first
   time the model answers a draft it already gave, because a model repeating itself is not
   converging. 3 is a cost bound, not a measured optimum: each attempt is one model call,
   and no corpus of real drafts exists yet to measure where repair stops paying (the stub
   model in the tests cannot say). Measure it once real drafts accumulate.
5. **Always a draft**: the valid draft is saved with a Sink on `urn:script:{name}`
   (`language=sparql`, `state=draft`, `if-version` so a concurrent writer is noticed),
   under the caller, named by `save=` or `nl-sparql-` and a digest of the drafting. The
   script host analyzes it again and refuses a draft whose derived authority the caller
   does not hold: that is the last validation, and an `InvalidArgument` there is fed back
   like any other. A published script is never replaced (refused before any model is
   asked). A caller who may not write the script gets the valid draft answered,
   **unsaved**. A draft that never validates is answered as **failed**, with every attempt
   and its errors, and no query.

### Provenance

A saved draft's text opens with its provenance, as N-Triples comment lines, above the
model's `# @param` block (so the parameters still declare). It is part of the content, so
it is in the version's digest, and `ikigai_nl::prov_of(text)` reads it back. The same
graph is the answer's `as=text/turtle` face, and the JSON face carries it as `prov`:

```text
<urn:nl:sparql:drafting:{hex}> a prov:Activity ;
    nl:ask "…" ;
    prov:used <urn:nl:grounding:sha256:…>, <urn:nl:prompt:sparql>, <urn:nl:prompt:sparql-repair> ;
    prov:wasAssociatedWith <urn:llm:ask> .
<urn:nl:grounding:sha256:…> a nl:Grounding ; dcterms:identifier "sha256:…" ;
    dcterms:hasPart <…:actions>, <…:vocabulary>, <…:graphs>, <…:examples> .   # each with its identity and source
<urn:nl:prompt:sparql> dcterms:identifier "sha256:…" .
<urn:nl:sparql:drafting:{hex}:attempt:1> a prov:Entity ;
    prov:wasGeneratedBy <urn:nl:sparql:drafting:{hex}> ;
    prov:wasAttributedTo <urn:llm:ask> ; nl:model "…" ;
    prov:value "the query" ; dcterms:identifier "sha256:…" ;
    nl:valid false ; nl:error "…" ; nl:warning "…" .
<urn:nl:sparql:drafting:{hex}:attempt:2> … prov:wasRevisionOf <…:attempt:1> .
<urn:script:{name}> prov:wasGeneratedBy <urn:nl:sparql:drafting:{hex}> ;
    prov:wasDerivedFrom <…:attempt:2> .
```

Literals are serialized by `oxrdf`, which escapes a newline, so an ask can never end its
comment line and become query text (a test drafts from an ask that tries).

### The prompts are resources

`urn:nl:prompt:sparql` and `urn:nl:prompt:sparql-repair` are text templates with
`{{placeholders}}`, sourced through the kernel, filled in one pass (a value is never
scanned for placeholders), and cited by sha256 in the provenance. A host that wants its
own binds the same names in front of this crate's space.

### What the host configures

```rust,no_run
use ikigai_nl::{Escalation, Llm, SpaceConfig};

let config = SpaceConfig::new()
    .llm(Llm::default().escalate(Escalation { after: 2, needs: "cost<=premium".into() }))
    .max_attempts(3)   // drafts per ask, clamped to 1..=8
    .preview_rows(5)
    .prompt_examples(3)
    .store_prefix("urn:iki:store:");
```

The model call runs under the caller's capability, so a caller who cannot reach the host's
LLM door cannot draft: a host that wants anonymous drafting grants its anonymous principal
that door (and, to save, the script names it may write).

## Vocabulary

Actions are said with `ik:`, the vocabulary with `rdfs:`, each graph's shape with
[VoID](http://rdfs.org/ns/void#), examples with `schema:`, identity and origin with
`dcterms:` and `prov:`, a draft's provenance with PROV-O. The few terms nothing else has
(`nl:Grounding`, `nl:Part`, `nl:focus`, `nl:shown`, `nl:of`, `nl:sample`,
`nl:samplesShown`, `nl:classesShown`, `nl:propertiesShown`, and for provenance `nl:ask`,
`nl:model`, `nl:valid`, `nl:error`, `nl:warning`) are defined in `src/nl.ttl`, exported as
`ikigai_nl::VOCABULARY`, under `https://ikigai-rs.dev/ns/nl#`. A test holds the renderers
to exactly that list.

## Copies, until ikigai-script publishes

`src/script_sparql.rs` and `src/limits.rs` are byte-for-byte copies of ikigai-script's
`sparql.rs` and `limits.rs` (the latter itself ikigai-store's), changed only where they
name their crate. The check must analyze a draft exactly as the script host will when it
is saved, and ikigai-script is not published. When it is, both go and this crate depends
on it. The test host's stand-in for the script host analyzes with the same copy, so it is
faithful to the real Sink's rules, not to a second reading of them.

## Not in this version

- **`urn:nl:script`**, the plan-then-Lisp drafter: the next arc. It can reuse the
  grounding, the prompt resources (with its own templates), the repair loop's shape, the
  provenance builder and the save-as-draft step; it needs the actions part, which a graph
  focus drops today (a `parts=` selector on the grounding would fix that).
- **A measured repair bound.** See above: 3 is a cost bound.
- **IRIs beyond predicates and classes.** The check does not judge instance IRIs, IRIs in
  expressions, `FILTER EXISTS` patterns or a CONSTRUCT template.
- **Example parameters in Turtle.** A script document's `parameters` are passed through
  in the JSON face as the script host states them; the Turtle face does not state them
  yet.
- **A resolvable grounding IRI.** `urn:nl:grounding:sha256:…` names a grounding by its
  content; nothing stores one to resolve it by. A draft records the grounding's identity
  and each part's, not the grounding itself.
- **Per-graph invalidation.** The store hangs every read from its three write threads,
  not from the graph it read, so a write to any graph re-queries every graph's shape.

## License

MIT OR Apache-2.0.
