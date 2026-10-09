//! What a host decides: the bounds, where worked examples come from, and how the drafter
//! reaches a model.

/// Where the grounding's worked examples come from.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Examples {
    /// Every published script the caller may read, from `urn:script:catalog`.
    ///
    /// ⚠ **The catalog is live** (`ikigai-script` records a run without a write through
    /// any name the catalog could hang from), and a grounding is only as cacheable as its
    /// least cacheable part, so a grounding that reads it is recomputed on every read.
    /// Its parts stay cached (each manifold read, description, store query and script
    /// head), so a recompute is an assembly, not a re-query, and `urn:kernel:uncached`
    /// names `urn:script:catalog` as the reason. [`Examples::Named`] keeps the grounding
    /// itself cached.
    #[default]
    Catalog,
    /// These scripts, by name, read one by one (`urn:script:{name}`): each read is cached
    /// and cut when that script is republished, so the grounding stays cacheable. Only
    /// published scripts the caller may read contribute. Name scripts every caller may
    /// read (public ones): a caller who holds `urn:cap:script:read:public` but not a
    /// private script's grant is refused that read, and a refusal is never cached, so its
    /// grounding is recomputed on every read.
    Named(Vec<String>),
    /// No examples.
    None,
}

/// How a host configures `urn:nl:grounding`.
///
/// ```
/// use ikigai_nl::{Examples, SpaceConfig};
///
/// let config = SpaceConfig::new()
///     .max_bytes(512 * 1024)
///     .samples(3)
///     .examples(Examples::Named(vec!["stale-urgent".into()]));
/// assert_eq!(config.samples, 3);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpaceConfig {
    /// The largest grounding, in bytes of the face asked for. A larger one is REFUSED,
    /// naming its size and this bound, so the caller narrows it with `focus=`.
    pub max_bytes: usize,
    /// The most graphs to summarize; past it, the first N of M are shown and the part
    /// says so.
    pub max_graphs: usize,
    /// The most classes and the most predicates listed per graph.
    pub max_partitions: usize,
    /// The most sample triples per graph.
    pub samples: usize,
    /// The most worked examples.
    pub max_examples: usize,
    /// Where the examples come from.
    pub examples: Examples,
    /// How `urn:nl:sparql` reaches a model.
    pub llm: Llm,
    /// The most drafts `urn:nl:sparql` asks for per ask: the first, then a repair for each
    /// failure, up to this many in all. Clamped to `1..=`[`MAX_ATTEMPTS`].
    pub max_attempts: usize,
    /// The rows (or triples) a dry run previews.
    pub preview_rows: usize,
    /// The most published SPARQL scripts a drafting prompt shows as worked examples, when
    /// the ask does not say (`examples=`).
    pub prompt_examples: usize,
    /// Where the store's graph-scoped doors are bound: `urn:iki:store:` for
    /// `urn:iki:store:graph-select` and its siblings.
    pub store_prefix: String,
}

/// The hard ceiling on [`SpaceConfig::max_attempts`].
pub const MAX_ATTEMPTS: usize = 8;

/// How the drafter reaches a model: through the host's LLM doors (`ikigai-llm`'s
/// `urn:llm:ask` and `urn:llm:select`, or anything speaking their contract), under the
/// CALLER's capability.
///
/// Local first: with no `needs`, every draft goes to [`ask`](Llm::ask), the host's default
/// backend. A frontier model is reached only by the host's own policy,
/// [`escalate`](Llm::escalate): from the attempt after `after` failures, the drafter asks
/// [`select`](Llm::select) for a backend meeting `needs` and drafts there.
///
/// ```
/// use ikigai_nl::{Escalation, Llm};
///
/// let llm = Llm::default().escalate(Escalation { after: 2, needs: "cost<=premium".into() });
/// assert_eq!(llm.ask, "urn:llm:ask");
/// assert_eq!(llm.needs_for(1), None);
/// assert_eq!(llm.needs_for(3), Some("cost<=premium"));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Llm {
    /// The ask door: Source, `prompt=`, `as=application/json` answering
    /// `{"text", "model", …}`.
    pub ask: String,
    /// The selection door: Source, `needs=`, `as=application/json` answering
    /// `{"backend", "provider", "model", …}`.
    pub select: String,
    /// Requirements for every draft (`needs=` in `ikigai-llm`'s grammar), when the host
    /// wants selection rather than its default backend.
    pub needs: Option<String>,
    /// The host's escalation policy, if any.
    pub escalate: Option<Escalation>,
    /// The `temperature=` each draft is asked at; `None` leaves the backend's default.
    pub temperature: Option<String>,
}

/// From the attempt after `after` failed ones, draft with a backend meeting `needs`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Escalation {
    /// Failed attempts before escalating.
    pub after: usize,
    /// The requirements of the backend escalated to.
    pub needs: String,
}

impl Default for Llm {
    fn default() -> Self {
        Llm {
            ask: "urn:llm:ask".to_string(),
            select: "urn:llm:select".to_string(),
            needs: None,
            escalate: None,
            temperature: Some("0".to_string()),
        }
    }
}

impl Llm {
    /// Set [`needs`](Llm::needs).
    pub fn needs(mut self, needs: impl Into<String>) -> Self {
        self.needs = Some(needs.into());
        self
    }

    /// Set [`escalate`](Llm::escalate).
    pub fn escalate(mut self, escalation: Escalation) -> Self {
        self.escalate = Some(escalation);
        self
    }

    /// The requirements attempt `attempt` (1-based) drafts under: the escalation's after
    /// `after` failed attempts, else [`needs`](Llm::needs).
    pub fn needs_for(&self, attempt: usize) -> Option<&str> {
        match &self.escalate {
            Some(e) if attempt > e.after => Some(e.needs.as_str()),
            _ => self.needs.as_deref(),
        }
    }
}

impl Default for SpaceConfig {
    fn default() -> Self {
        SpaceConfig {
            max_bytes: 1024 * 1024,
            max_graphs: 32,
            max_partitions: 25,
            samples: 5,
            max_examples: 16,
            examples: Examples::Catalog,
            llm: Llm::default(),
            max_attempts: 3,
            preview_rows: 5,
            prompt_examples: 3,
            store_prefix: "urn:iki:store:".to_string(),
        }
    }
}

impl SpaceConfig {
    /// The defaults: 1 MiB, 32 graphs, 25 classes and 25 predicates and 5 samples per
    /// graph, 16 examples from the catalog; for the drafter, the host's default model at
    /// temperature 0, three attempts, five preview rows, three worked examples, and the
    /// store at `urn:iki:store:`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set [`max_bytes`](Self::max_bytes).
    pub fn max_bytes(mut self, bytes: usize) -> Self {
        self.max_bytes = bytes;
        self
    }

    /// Set [`max_graphs`](Self::max_graphs).
    pub fn max_graphs(mut self, graphs: usize) -> Self {
        self.max_graphs = graphs;
        self
    }

    /// Set [`max_partitions`](Self::max_partitions).
    pub fn max_partitions(mut self, partitions: usize) -> Self {
        self.max_partitions = partitions;
        self
    }

    /// Set [`samples`](Self::samples).
    pub fn samples(mut self, samples: usize) -> Self {
        self.samples = samples;
        self
    }

    /// Set [`max_examples`](Self::max_examples).
    pub fn max_examples(mut self, examples: usize) -> Self {
        self.max_examples = examples;
        self
    }

    /// Set where the examples come from.
    pub fn examples(mut self, examples: Examples) -> Self {
        self.examples = examples;
        self
    }

    /// Set how the drafter reaches a model.
    pub fn llm(mut self, llm: Llm) -> Self {
        self.llm = llm;
        self
    }

    /// Set [`max_attempts`](Self::max_attempts), clamped to `1..=`[`MAX_ATTEMPTS`].
    pub fn max_attempts(mut self, attempts: usize) -> Self {
        self.max_attempts = attempts.clamp(1, MAX_ATTEMPTS);
        self
    }

    /// Set [`preview_rows`](Self::preview_rows).
    pub fn preview_rows(mut self, rows: usize) -> Self {
        self.preview_rows = rows;
        self
    }

    /// Set [`prompt_examples`](Self::prompt_examples).
    pub fn prompt_examples(mut self, examples: usize) -> Self {
        self.prompt_examples = examples;
        self
    }

    /// Set [`store_prefix`](Self::store_prefix).
    pub fn store_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.store_prefix = prefix.into();
        self
    }
}
