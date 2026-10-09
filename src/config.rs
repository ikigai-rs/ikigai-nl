//! What a host decides: the bounds, and where worked examples come from.

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
        }
    }
}

impl SpaceConfig {
    /// The defaults: 1 MiB, 32 graphs, 25 classes and 25 predicates and 5 samples per
    /// graph, 16 examples from the catalog.
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
}
