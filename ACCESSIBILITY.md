# Accessibility

What `ikigai-nl` emits that people read, what of it is this repository's and what is a model's,
the rules its own text follows, how it is tested, and how to report a barrier. It is not a
conformance claim. Adapted from the
[ACCESSIBILITY.md template](https://github.com/mgifford/ACCESSIBILITY.md) (MIT).

| Field | Value |
|---|---|
| Target | WCAG 2.2 Level AA, for any page that presents this output; this repository writes text only (plain text, JSON and Turtle), no HTML or SVG |
| Status | Evaluation in progress. No conformance claim. |
| Owner | the maintainers of this repository |
| Report a barrier | [a GitHub issue](https://github.com/ikigai-rs/ikigai-nl/issues/new?labels=accessibility) with the `accessibility` label; anything security-sensitive through [private vulnerability reporting](https://github.com/ikigai-rs/ikigai-nl/security/advisories/new) |
| Last reviewed | 2026-10-31 |

## What it emits that people read

- **A drafting's answer** from `urn:nl:sparql` (`plain` in `src/drafter.rs`): a first line that
  says what happened in words (the saved version and `a DRAFT of <script>: review it, then
  publish it with state=published`; `a valid draft, NOT saved: …`; or `no valid draft: …`, with
  `drafting stopped: the model repeated attempt N` when it did), then the kept query, its
  warnings and its preview; or, for a failed drafting, every attempt with its backend, model,
  text and `error:` lines. Also as JSON and as a provenance graph in Turtle.
- **A check** from `urn:nl:sparql:check` (`to_text` in `src/check.rs`): `valid <form>` or
  `invalid <form>` and the graphs it reads, then `requires:`, `parameter:`, `error:` and
  `warning:` lines, and the preview (`preview (LIMIT n):`, a header row of `?column` names and
  tab-separated rows).
- **A grounding** from `urn:nl:grounding` (`src/render.rs`), as Turtle or JSON, with short notes
  such as `1 of 2 graphs mention the focus …`; read mostly by a model, sometimes by a person.
- **The prompts** `urn:nl:prompt:sparql` and `urn:nl:prompt:sparql-repair` (`src/prompts/`),
  filled with grounding text from `src/prompt.rs`.
- **Refusals** (`src/endpoint.rs`, `src/check.rs`, `src/drafter.rs`, `src/gather.rs`): a missing
  `ask` or `content`, an input outside its form, `focus=` and `graph=` that disagree, a
  published script a draft may not replace, a resource that did not answer the form expected.

**The query text in a draft and every failed attempt's text are the model's, not this
repository's.** This crate checks a draft as a query (it parses, reads only graphs the caller
may read, names known terms) and labels it a draft, but it does not change or describe the
model's words. The values in a preview and the samples in a grounding are the caller's own data.
Not this repository's either: the sentence each error kind is wrapped in (ikigai-core's
`Display`), errors from the script host passed back as findings, and any page a host shows this
in.

## The rules that output follows

- What happened is said in words on the first line, before any query: a draft is called a
  draft and says how to publish it, an unsaved or failed drafting says so, and a draft with an
  empty preview says that the preview is empty. Nothing is shown as published.
- Every finding line starts with a word: `error:`, `warning:`, `requires:`, `parameter:`.
- A graph the caller cannot read is refused in the same words as one that does not exist.
- No color is written.

## How it is tested

- `tests/sparql.rs`: `a_draft_that_never_validates_comes_back_failed_with_every_attempt` (the
  answer starts `no valid draft`), `the_check_is_a_resource_of_its_own` (`preview (LIMIT 5)`),
  `a_draft_reaching_a_graph_the_caller_cannot_read_is_refused_without_leaking_it`.
- `tests/drafting.rs`: `a_repeat_says_drafting_stopped_and_is_recorded_as_a_repeat_not_a_failure`
  (the first line says `drafting stopped: the model repeated attempt 1`),
  `a_draft_still_empty_after_the_last_attempt_is_saved_marked_never_silently`.
- **No automated accessibility checker, browser or assistive technology has been used on this
  output.**

## Known limitations

Found by reading the source on 2026-10-31:

- The preview is a tab-separated table with no alignment, and its values are written as RDF
  terms (`<urn:…>`, `"first"`), so a screen reader reads the brackets and quotes and a sighted
  reader sees ragged columns.
- A saved draft's first line opens with a long version IRI (a sha256 digest) before the words
  that say it is a draft.
- Some failures name a wire detail rather than what to do (`… did not answer its JSON form: …`,
  `serializing the check: …`).
- This repository's own text is English only; the model may answer in any language, and nothing
  states which.

## Contributing

Read this file before changing any output listed above: the plain answers, the prompts or the
refusals. Keep meaning in words, keep the first line saying what happened, and keep the model's
text separate from this crate's. Say in the pull request what was tested, with what, and what
was not.

## Reporting a barrier

[Open an issue](https://github.com/ikigai-rs/ikigai-nl/issues/new?labels=accessibility) with the
`accessibility` label (the link sets it when GitHub lets you add labels; otherwise a maintainer
adds it): what you were doing, what happened, and your terminal or other tool and any assistive
technology, if you are comfortable sharing them. Anything security-sensitive goes through
[private vulnerability reporting](https://github.com/ikigai-rs/ikigai-nl/security/advisories/new).
**We acknowledge a report within 7 days**, and promise nothing further.

## Change history

| Date | Change |
|---|---|
| 2026-10-31 | First version. |
