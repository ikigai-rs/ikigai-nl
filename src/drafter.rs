//! `urn:nl:sparql`: natural language to a DRAFT SPARQL query. The model is a drafter,
//! never an actor.
//!
//! The funnel, each step a resource call under the CALLER's capability:
//!
//! 1. **Ground**: `urn:nl:grounding` (focused by `graph=`), what the caller may read.
//! 2. **Draft**: `urn:nl:prompt:sparql` filled with the ask and the grounding, sent to the
//!    host's LLM door ([`crate::Llm`]): its default backend, or `urn:llm:select` under the
//!    host's `needs` or escalation policy.
//! 3. **Validate**: `urn:nl:sparql:check` (bound and parse, derived authority against the
//!    caller's, every predicate and class against the grounding, a dry run).
//! 4. **Repair**: a refused draft goes back to the model with `urn:nl:prompt:sparql-repair`
//!    and what was refused, up to the host's bound ([`crate::SpaceConfig::max_attempts`]).
//!    A model that answers a draft it already gave is stopped there: it will not converge.
//! 5. **Save as a draft**: a Sink on `urn:script:{name}` with `language=sparql` and
//!    `state=draft`, whose text opens with the drafting's provenance as N-Triples comment
//!    lines ([`prov_of`] reads them back). The script host analyzes it again and refuses
//!    a draft whose derived authority the caller does not hold, so its refusal is the last
//!    validation. Never published, never run: a person publishes.

use std::sync::Arc;

use async_trait::async_trait;
use ikigai_core::{
    ActionSpec, ArgSpec, Description, Endpoint, Error, Invocation, ReprType, Representation,
    Result, Verb,
};
use oxrdf::{Literal, NamedNode, Term, Triple};
use serde::{Deserialize, Serialize};

use crate::check::{self, count, detail, optional, Check, CHECK_IRI};
use crate::config::SpaceConfig;
use crate::gather::request;
use crate::model::{sha256, Grounding};
use crate::prompt::{self, REPAIR_PROMPT_IRI, SPARQL_PROMPT_IRI};
use crate::NS;

/// `urn:nl:sparql`.
pub const SPARQL_IRI: &str = "urn:nl:sparql";

/// The version of a draft's JSON face.
pub const DRAFT_SCHEMA: u32 = 1;

/// The line that opens a saved draft's provenance block.
pub const PROV_MARKER: &str = "# Provenance (PROV-O as N-Triples):";

const PLAIN: &str = "text/plain";
const JSON: &str = "application/json";
const TURTLE: &str = "text/turtle";
const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";
const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const PROV: &str = "http://www.w3.org/ns/prov#";
const DCTERMS: &str = "http://purl.org/dc/terms/";

/// The script names the script host reserves.
const RESERVED_NAMES: [&str; 3] = ["eval", "catalog", "public"];

/// What became of an ask.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// A valid draft, saved as a draft script.
    Saved,
    /// A valid draft the script host would not keep for this caller (no write grant on
    /// the name, or a concurrent writer): answered, not saved.
    Unsaved,
    /// No attempt validated: every attempt and what was wrong with it, and no query.
    Failed,
}

/// One draft a model gave, and what the checks said of it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attempt {
    /// 1-based.
    pub n: usize,
    /// The LLM door it was asked through (`urn:llm:ask`, or the backend selected).
    pub backend: String,
    /// The model that answered, as its door named it.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub model: Option<String>,
    /// The query extracted from the answer.
    pub text: String,
    /// `sha256:` over `text`.
    pub identity: String,
    /// Whether it passed every check (and the script host kept it, when it was saved).
    pub valid: bool,
    /// What was wrong with it.
    pub errors: Vec<String>,
    /// What may be wrong with it.
    pub warnings: Vec<String>,
}

/// What `urn:nl:sparql` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    /// [`DRAFT_SCHEMA`].
    pub schema: u32,
    /// What became of the ask.
    pub status: Status,
    /// The ask.
    pub ask: String,
    /// `urn:nl:sparql:drafting:{hex}`: this drafting, as a PROV activity.
    pub activity: String,
    /// The script name the draft is (or would be) saved under.
    pub name: String,
    /// `urn:script:{name}`.
    pub script: String,
    /// The saved version's IRI, as the script host named it.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub version: Option<String>,
    /// The draft's text as saved: the provenance block, then the query. `None` when no
    /// attempt validated.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub query: Option<String>,
    /// The check of the valid attempt, with its preview.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub check: Option<Check>,
    /// Every attempt, in order.
    pub attempts: Vec<Attempt>,
    /// The grounding drafted from (`urn:nl:grounding:sha256:…`).
    pub grounding: String,
    /// Its identity.
    pub grounding_identity: String,
    /// Why the draft was not saved, or how it failed, in a sentence.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub note: Option<String>,
    /// The provenance, as N-Triples.
    pub prov: String,
}

/// The provenance block of a saved draft's text, as N-Triples, or `None` when the text
/// carries none.
///
/// ```
/// // (`##` is how rustdoc spells a line that starts with `#`.)
/// let text = "# Drafted by urn:nl:sparql.\n\
///             ## Provenance (PROV-O as N-Triples):\n\
///             ## <urn:a> <http://www.w3.org/ns/prov#used> <urn:b> .\n\
///             ##\n\
///             ASK { GRAPH <urn:g> { ?s ?p ?o } }";
/// assert_eq!(
///     ikigai_nl::prov_of(text).unwrap(),
///     "<urn:a> <http://www.w3.org/ns/prov#used> <urn:b> .\n"
/// );
/// assert!(ikigai_nl::prov_of("ASK {}").is_none());
/// ```
pub fn prov_of(text: &str) -> Option<String> {
    let mut lines = text.lines().skip_while(|l| l.trim() != PROV_MARKER);
    lines.next()?;
    let mut out = String::new();
    for line in lines {
        match line.strip_prefix("# <") {
            Some(rest) => {
                out.push('<');
                out.push_str(rest);
                out.push('\n');
            }
            None => break,
        }
    }
    Some(out)
}

/// A script name: `[a-z0-9][a-z0-9_-]*`, at most 64 bytes, not reserved. The script host
/// checks it again; this refuses a bad `save=` before any model is asked.
fn valid_name(name: &str) -> Result<()> {
    let ok = name.len() <= 64
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        && !RESERVED_NAMES.contains(&name);
    if ok {
        Ok(())
    } else {
        Err(Error::InvalidArgument {
            name: "save".to_string(),
            detail: format!(
                "`{name}` is not a script name: lowercase ASCII letters, digits, `_` and `-`, \
                 starting with a letter or digit, at most 64 bytes, and not {}",
                RESERVED_NAMES.join(", ")
            ),
        })
    }
}

/// A script's head, as far as the drafter needs it.
#[derive(Deserialize)]
struct Head {
    version: String,
    state: String,
}

/// What a save may overwrite: `Some(if-version)`, or `None` when the name holds a
/// published script, which a draft must never replace.
async fn if_version(inv: &Invocation<'_>, name: &str) -> Result<Option<String>> {
    let target = format!("urn:script:{name}");
    match inv
        .issue(request(Verb::Source, &target, &[("as", JSON)])?)
        .await
    {
        Ok(rep) => {
            let head: Head = serde_json::from_slice(&rep.bytes).map_err(|e| {
                Error::Endpoint(format!("{target} did not answer its JSON head: {e}"))
            })?;
            Ok((head.state != "published").then_some(head.version))
        }
        // Absent, or not readable by this caller: create only. If it exists after all,
        // the script host answers Conflict and the draft is answered unsaved.
        Err(Error::NotFound(_) | Error::Unresolved(_) | Error::Denied(_)) => {
            Ok(Some("none".to_string()))
        }
        Err(e) => Err(e),
    }
}

fn published(name: &str) -> Error {
    Error::InvalidArgument {
        name: "save".to_string(),
        detail: format!(
            "urn:script:{name} is a published script, and a draft saved there would replace \
             it; choose another name"
        ),
    }
}

/// The model's answer.
#[derive(Deserialize)]
struct Answer {
    text: String,
    model: Option<String>,
}

/// A selection door's answer.
#[derive(Deserialize)]
struct Selected {
    backend: String,
    model: Option<String>,
}

/// One draft from the host's LLM door: (backend, model, answer text).
async fn ask_model(
    inv: &Invocation<'_>,
    config: &SpaceConfig,
    attempt: usize,
    prompt: &str,
) -> Result<(String, Option<String>, String)> {
    let llm = &config.llm;
    let (backend, selected_model) = match llm.needs_for(attempt) {
        Some(needs) => {
            let rep = inv
                .issue(request(
                    Verb::Source,
                    &llm.select,
                    &[("needs", needs), ("as", JSON)],
                )?)
                .await?;
            let selected: Selected = serde_json::from_slice(&rep.bytes).map_err(|e| {
                Error::Endpoint(format!("{} did not answer its JSON form: {e}", llm.select))
            })?;
            (selected.backend, selected.model)
        }
        None => (llm.ask.clone(), None),
    };
    let mut args = vec![("prompt", prompt), ("as", JSON)];
    if let Some(t) = llm.temperature.as_deref() {
        args.push(("temperature", t));
    }
    let rep = inv.issue(request(Verb::Source, &backend, &args)?).await?;
    let answer: Answer = serde_json::from_slice(&rep.bytes)
        .map_err(|e| Error::Endpoint(format!("{backend} did not answer its JSON form: {e}")))?;
    Ok((backend, answer.model.or(selected_model), answer.text))
}

/// Read a prompt template through the kernel: (text, identity).
async fn template(inv: &Invocation<'_>, iri: &str) -> Result<(String, String)> {
    let rep = inv.issue(request(Verb::Source, iri, &[])?).await?;
    let text = String::from_utf8(rep.bytes)
        .map_err(|_| Error::Endpoint(format!("{iri} was not UTF-8")))?;
    let identity = sha256(text.as_bytes());
    Ok((text, identity))
}

/// The drafting's identity: what it was asked, what it was given, and what came back.
#[derive(Serialize)]
struct ActivityKey<'a> {
    ask: &'a str,
    focus: Option<&'a str>,
    grounding: &'a str,
    prompt: &'a str,
    attempts: Vec<&'a str>,
}

/// Everything the provenance states.
struct Drafting<'a> {
    ask: &'a str,
    activity: String,
    grounding: &'a Grounding,
    prompts: Vec<(&'static str, String)>,
    attempts: &'a [Attempt],
    script: Option<String>,
}

/// PROV-O, as N-Triples, serialized by `oxrdf`: no blank node, every node an IRI.
fn prov(d: &Drafting<'_>) -> String {
    let mut out = String::new();
    let mut push = |s: &str, p: &str, o: Term| {
        if let (Ok(s), Ok(p)) = (NamedNode::new(s), NamedNode::new(p)) {
            out.push_str(&Triple::new(s, p, o).to_string());
            out.push_str(" .\n");
        }
    };
    let iri = |s: &str| -> Term {
        NamedNode::new(s)
            .map(Term::from)
            .unwrap_or_else(|_| Literal::new_simple_literal(s).into())
    };
    let lit = |s: &str| -> Term { Literal::new_simple_literal(s).into() };
    let a = d.activity.as_str();
    let g = d.grounding;

    push(a, RDF_TYPE, iri(&format!("{PROV}Activity")));
    push(a, &format!("{NS}ask"), lit(d.ask));
    push(a, &format!("{PROV}used"), iri(&g.iri));
    push(&g.iri, RDF_TYPE, iri(&format!("{NS}Grounding")));
    push(&g.iri, &format!("{DCTERMS}identifier"), lit(&g.identity));
    if let Some(focus) = &g.focus {
        push(&g.iri, &format!("{NS}focus"), lit(focus));
    }
    let parts: [(&str, &Option<String>, &str); 4] = [
        ("actions", &g.actions.source, &g.actions.identity),
        ("vocabulary", &g.vocabulary.source, &g.vocabulary.identity),
        ("graphs", &g.graphs.source, &g.graphs.identity),
        ("examples", &g.examples.source, &g.examples.identity),
    ];
    for (name, source, identity) in parts {
        let part = format!("{}:{name}", g.iri);
        push(&g.iri, &format!("{DCTERMS}hasPart"), iri(&part));
        push(&part, &format!("{DCTERMS}identifier"), lit(identity));
        if let Some(source) = source {
            push(&part, &format!("{PROV}wasDerivedFrom"), iri(source));
        }
    }
    for (prompt, identity) in &d.prompts {
        push(a, &format!("{PROV}used"), iri(prompt));
        push(prompt, &format!("{DCTERMS}identifier"), lit(identity));
    }
    let mut previous: Option<String> = None;
    for attempt in d.attempts {
        let e = format!("{a}:attempt:{}", attempt.n);
        push(&e, RDF_TYPE, iri(&format!("{PROV}Entity")));
        push(&e, &format!("{PROV}wasGeneratedBy"), iri(a));
        push(
            a,
            &format!("{PROV}wasAssociatedWith"),
            iri(&attempt.backend),
        );
        push(&e, &format!("{PROV}wasAttributedTo"), iri(&attempt.backend));
        if let Some(model) = &attempt.model {
            push(&e, &format!("{NS}model"), lit(model));
        }
        push(&e, &format!("{PROV}value"), lit(&attempt.text));
        push(&e, &format!("{DCTERMS}identifier"), lit(&attempt.identity));
        push(
            &e,
            &format!("{NS}valid"),
            Literal::from(attempt.valid).into(),
        );
        for error in &attempt.errors {
            push(&e, &format!("{NS}error"), lit(error));
        }
        for warning in &attempt.warnings {
            push(&e, &format!("{NS}warning"), lit(warning));
        }
        if let Some(prev) = &previous {
            push(&e, &format!("{PROV}wasRevisionOf"), iri(prev));
        }
        previous = Some(e);
    }
    if let (Some(script), Some(last)) = (&d.script, &previous) {
        push(script, &format!("{PROV}wasGeneratedBy"), iri(a));
        push(script, &format!("{PROV}wasDerivedFrom"), iri(last));
    }
    out
}

/// The text a draft is saved as: a provenance block of comment lines, then the query
/// (whose own `# @param` lines stay in the leading comment block, where they declare).
fn saved_text(prov: &str, query: &str) -> String {
    let mut out = String::from(
        "# Drafted by urn:nl:sparql from a natural-language ask: a DRAFT for a person to \
         review and publish.\n",
    );
    out.push_str(PROV_MARKER);
    out.push('\n');
    for line in prov.lines() {
        out.push_str("# ");
        out.push_str(line);
        out.push('\n');
    }
    out.push_str("#\n");
    out.push_str(query);
    out.push('\n');
    out
}

pub(crate) struct SparqlEndpoint {
    pub(crate) config: Arc<SpaceConfig>,
}

impl SparqlEndpoint {
    async fn draft(&self, inv: &Invocation<'_>) -> Result<Draft> {
        let config = &self.config;
        let ask = match (optional(inv, "ask")?, optional(inv, "content")?) {
            (Some(a), Some(c)) if a != c => {
                return Err(Error::InvalidArgument {
                    name: "content".to_string(),
                    detail: "the ask arrived twice, as `ask=` and as piped content, and they \
                             differ; pass one"
                        .to_string(),
                })
            }
            (Some(a), _) | (None, Some(a)) => a,
            (None, None) => return Err(Error::MissingArgument("ask".to_string())),
        };
        let focus = optional(inv, "graph")?;
        let n_examples = count(inv, "examples", config.prompt_examples, 16)?;
        let save = optional(inv, "save")?;
        if let Some(name) = save {
            valid_name(name)?;
            // Before any model is asked: a published script is never replaced by a draft.
            if if_version(inv, name).await?.is_none() {
                return Err(published(name));
            }
        }

        // 1. Ground, under the caller.
        let grounding = check::grounding(inv, focus).await?;
        let grounding_text = prompt::grounding_text(&grounding);
        let examples_text = prompt::examples_text(&grounding, n_examples);

        // 2-4. Draft, validate, repair.
        let (main, main_id) = template(inv, SPARQL_PROMPT_IRI).await?;
        let first = prompt::fill(
            &main,
            &[
                ("ask", ask),
                ("grounding", &grounding_text),
                ("examples", &examples_text),
            ],
        );
        let mut prompts: Vec<(&'static str, String)> = vec![(SPARQL_PROMPT_IRI, main_id)];
        let mut repair: Option<String> = None;
        let mut attempts: Vec<Attempt> = Vec::new();
        let mut valid: Option<(Check, String)> = None;
        let mut saved: Option<String> = None;
        let mut note: Option<String> = None;
        let mut name = String::new();
        let max = config.max_attempts.clamp(1, crate::config::MAX_ATTEMPTS);
        for n in 1..=max {
            let prompt = match attempts.last() {
                None => first.clone(),
                Some(previous) => {
                    if repair.is_none() {
                        let (text, id) = template(inv, REPAIR_PROMPT_IRI).await?;
                        prompts.push((REPAIR_PROMPT_IRI, id));
                        repair = Some(text);
                    }
                    let errors: Vec<String> =
                        previous.errors.iter().map(|e| format!("- {e}")).collect();
                    format!(
                        "{first}\n{}",
                        prompt::fill(
                            repair.as_deref().unwrap_or_default(),
                            &[("previous", &previous.text), ("errors", &errors.join("\n"))],
                        )
                    )
                }
            };
            let (backend, model, answer) = ask_model(inv, config, n, &prompt).await?;
            let text = prompt::extract(&answer);
            let mut attempt = Attempt {
                n,
                backend,
                model,
                identity: sha256(text.as_bytes()),
                text,
                valid: false,
                errors: Vec::new(),
                warnings: Vec::new(),
            };
            if let Some(same) = attempts.iter().find(|a| a.identity == attempt.identity) {
                attempt.errors.push(format!(
                    "the same draft as attempt {}; the model is not converging, so drafting \
                     stops here",
                    same.n
                ));
                attempts.push(attempt);
                break;
            }
            let rep = inv
                .issue(request(
                    Verb::Source,
                    CHECK_IRI,
                    &[
                        ("content", &attempt.text),
                        ("focus", focus.unwrap_or_default()),
                        (
                            "preview",
                            &config.preview_rows.min(check::MAX_PREVIEW).to_string(),
                        ),
                        ("as", JSON),
                    ],
                )?)
                .await?;
            let checked: Check = serde_json::from_slice(&rep.bytes).map_err(|e| {
                Error::Endpoint(format!("{CHECK_IRI} did not answer its JSON form: {e}"))
            })?;
            attempt.errors = checked.errors.clone();
            attempt.warnings = checked.warnings.clone();
            attempt.valid = checked.valid;
            attempts.push(attempt);
            if !checked.valid {
                continue;
            }

            // 5. Save as a draft, under the caller. The script host's analysis and its
            // no-elevation check are the last validation.
            let activity = activity(ask, focus, &grounding, &prompts, &attempts);
            name = match save {
                Some(s) => s.to_string(),
                None => format!("nl-sparql-{}", &activity[activity.len() - 64..][..12]),
            };
            let script = format!("urn:script:{name}");
            let text = saved_text(
                &prov(&Drafting {
                    ask,
                    activity: activity.clone(),
                    grounding: &grounding,
                    prompts: prompts.clone(),
                    attempts: &attempts,
                    script: Some(script.clone()),
                }),
                &attempts[attempts.len() - 1].text,
            );
            let Some(if_version) = if_version(inv, &name).await? else {
                return Err(published(&name));
            };
            let request = request(
                Verb::Sink,
                &script,
                &[
                    ("content", &text),
                    ("language", "sparql"),
                    ("state", "draft"),
                    ("if-version", &if_version),
                ],
            )?;
            match inv.issue(request).await {
                Ok(rep) => {
                    saved = Some(String::from_utf8_lossy(&rep.bytes).trim().to_string());
                    valid = Some((checked, text));
                    break;
                }
                Err(Error::InvalidArgument { detail, .. }) => {
                    let last = attempts.len() - 1;
                    attempts[last].valid = false;
                    attempts[last]
                        .errors
                        .push(format!("the script host refused the draft: {detail}"));
                }
                Err(e @ (Error::Denied(_) | Error::Conflict(_))) => {
                    note = Some(format!(
                        "a valid draft, not saved: the script host refused to keep it at \
                         {script} for this caller ({})",
                        detail(&e)
                    ));
                    valid = Some((checked, text));
                    break;
                }
                Err(e) => return Err(e),
            }
        }

        let activity = activity(ask, focus, &grounding, &prompts, &attempts);
        if name.is_empty() {
            name = match save {
                Some(s) => s.to_string(),
                None => format!("nl-sparql-{}", &activity[activity.len() - 64..][..12]),
            };
        }
        let script = format!("urn:script:{name}");
        let status = match (&valid, &saved) {
            (Some(_), Some(_)) => Status::Saved,
            (Some(_), None) => Status::Unsaved,
            (None, _) => Status::Failed,
        };
        if status == Status::Failed {
            note = Some(format!(
                "no draft validated in {} attempt{} (the bound is {max}); every attempt and \
                 what was wrong with it is listed, and no query is answered",
                attempts.len(),
                if attempts.len() == 1 { "" } else { "s" },
            ));
        }
        let prov = prov(&Drafting {
            ask,
            activity: activity.clone(),
            grounding: &grounding,
            prompts,
            attempts: &attempts,
            script: (status == Status::Saved).then(|| script.clone()),
        });
        let (check, query) = match valid {
            Some((c, q)) => (Some(c), Some(q)),
            None => (None, None),
        };
        Ok(Draft {
            schema: DRAFT_SCHEMA,
            status,
            ask: ask.to_string(),
            activity,
            name,
            script,
            version: saved,
            query,
            check,
            attempts,
            grounding: grounding.iri.clone(),
            grounding_identity: grounding.identity.clone(),
            note,
            prov,
        })
    }
}

/// `urn:nl:sparql:drafting:{hex}` over everything the drafting was given and got.
fn activity(
    ask: &str,
    focus: Option<&str>,
    grounding: &Grounding,
    prompts: &[(&'static str, String)],
    attempts: &[Attempt],
) -> String {
    let key = ActivityKey {
        ask,
        focus,
        grounding: &grounding.identity,
        prompt: prompts
            .first()
            .map(|(_, id)| id.as_str())
            .unwrap_or_default(),
        attempts: attempts.iter().map(|a| a.identity.as_str()).collect(),
    };
    let digest = sha256(&serde_json::to_vec(&key).unwrap_or_default());
    format!("urn:nl:sparql:drafting:{}", &digest["sha256:".len()..])
}

/// The plain face: the saved draft and its preview, or every attempt of a failed one.
fn plain(draft: &Draft) -> String {
    let mut out = String::new();
    match draft.status {
        Status::Saved => out.push_str(&format!(
            "{} (a DRAFT of {}: review it, then publish it with state=published)\n",
            draft.version.as_deref().unwrap_or(&draft.script),
            draft.script
        )),
        Status::Unsaved => out.push_str(&format!(
            "a valid draft, NOT saved: {}\n",
            draft.note.as_deref().unwrap_or_default()
        )),
        Status::Failed => out.push_str(&format!(
            "no valid draft: {}\n",
            draft.note.as_deref().unwrap_or_default()
        )),
    }
    if let Some(last) = draft
        .attempts
        .last()
        .filter(|_| draft.status != Status::Failed)
    {
        out.push('\n');
        out.push_str(&last.text);
        out.push_str("\n\n");
        if let Some(check) = &draft.check {
            for w in &check.warnings {
                out.push_str(&format!("warning: {w}\n"));
            }
            if let Some(p) = &check.preview {
                out.push_str(&p.to_text());
            }
        }
    } else {
        for a in &draft.attempts {
            out.push_str(&format!(
                "\nattempt {} ({}{}):\n{}\n",
                a.n,
                a.backend,
                a.model
                    .as_deref()
                    .map(|m| format!(", {m}"))
                    .unwrap_or_default(),
                a.text
            ));
            for e in &a.errors {
                out.push_str(&format!("  error: {e}\n"));
            }
        }
    }
    out
}

#[async_trait]
impl Endpoint for SparqlEndpoint {
    async fn invoke(&self, inv: &Invocation<'_>) -> Result<Representation> {
        if inv.request.verb != Verb::Sink {
            return Err(Error::Endpoint(format!(
                "nl-sparql answers Sink (it writes a draft), not {:?}",
                inv.request.verb
            )));
        }
        let want = match optional(inv, "as")? {
            None | Some(PLAIN) => PLAIN,
            Some(JSON) => JSON,
            Some(TURTLE) => TURTLE,
            Some(other) => {
                return Err(Error::InvalidArgument {
                    name: "as".to_string(),
                    detail: format!(
                        "`{other}`: this resource answers {PLAIN}, {JSON} or {TURTLE} (the \
                         provenance)"
                    ),
                })
            }
        };
        let draft = self.draft(inv).await?;
        let (repr_type, bytes) = match want {
            JSON => {
                let mut bytes = serde_json::to_vec_pretty(&draft)
                    .map_err(|e| Error::Endpoint(format!("serializing the draft: {e}")))?;
                bytes.push(b'\n');
                (ReprType::new(JSON), bytes)
            }
            // N-Triples is Turtle.
            TURTLE => (
                ReprType::new(TURTLE).with_param("charset", "utf-8"),
                draft.prov.clone().into_bytes(),
            ),
            _ => (
                ReprType::new(PLAIN).with_param("charset", "utf-8"),
                plain(&draft).into_bytes(),
            ),
        };
        Ok(Representation::new(repr_type, bytes))
    }

    fn name(&self) -> &str {
        "nl-sparql"
    }

    fn describe(&self) -> Description {
        Description::new("nl-sparql")
            .title("Draft a SPARQL query from natural language")
            .summary(
                "Draft a SPARQL query from an ask, with a model as the DRAFTER, never an \
                 actor, all under the CALLER's capability: ground (urn:nl:grounding), draft \
                 through the host's LLM door, validate (urn:nl:sparql:check: bound and \
                 parse, derived authority against the caller's, every predicate and class \
                 against the grounding, a dry run), repair a refused draft up to the host's \
                 bound, and save the valid one as a DRAFT script (urn:script:{name}, \
                 language=sparql, state=draft) whose text opens with its provenance: the \
                 ask, each attempt with its model and validation result, the grounding and \
                 the prompts it came from. Never published, never run: a person publishes. \
                 A draft that never validates is answered as a failed draft with every \
                 attempt, never as a query.",
            )
            .verb(Verb::Meta)
            .action(
                ActionSpec::new(Verb::Sink)
                    .summary("Draft, validate, repair and save one query.")
                    .input(
                        ArgSpec::new("ask")
                            .summary(
                                "What the query should answer, in natural language. Required \
                                 unless it is piped (then it arrives as `content`).",
                            )
                            .class(XSD_STRING)
                            .optional(),
                    )
                    .input(
                        ArgSpec::new("content")
                            .summary("The ask, piped.")
                            .class(XSD_STRING)
                            .optional(),
                    )
                    .input(
                        ArgSpec::new("graph")
                            .summary(
                                "Draft against this graph: the grounding's focus (a graph \
                                 IRI or a topic word).",
                            )
                            .class(XSD_STRING)
                            .optional(),
                    )
                    .input(
                        ArgSpec::new("examples")
                            .summary(
                                "How many published SPARQL scripts the prompt shows as worked \
                                 examples (0 to 16); the host's default when absent.",
                            )
                            .class(XSD_INTEGER)
                            .optional(),
                    )
                    .input(
                        ArgSpec::new("save")
                            .summary(
                                "The script name to save the draft under (needs \
                                 `urn:cap:script:write:{save}`); `nl-sparql-` and a digest \
                                 of the drafting when absent. A published script is never \
                                 replaced.",
                            )
                            .class(XSD_STRING)
                            .optional(),
                    )
                    .input(
                        ArgSpec::new("as")
                            .summary(
                                "The draft and its preview (default), the JSON form, or the \
                                 provenance as Turtle.",
                            )
                            .class(XSD_STRING)
                            .one_of([PLAIN, JSON, TURTLE])
                            .default_value(PLAIN)
                            .optional(),
                    )
                    .output(PLAIN)
                    .output(JSON)
                    .output(TURTLE),
            )
    }
}
