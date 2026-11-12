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
| Last reviewed | 2026-11-12 |

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

Every named check has a result: Pass, Fail or Blocked, with evidence. A Pass covers only
that check, not overall accessibility. On 2026-11-12, `cargo test --test sparql --test drafting`
passed all 25 tests on main at `e48040033e3a403d256c01bb3e5b7cf2b2f7db3e`, before this
documentation change, using Cargo 1.99.0 and rustc 1.99.0. The five checks below ran as part of
that command; CI repeats those suites through the shared Rust workflow in
`.github/workflows/ci.yml`. These are assertions on library output and behavior, not browser
or assistive-technology evaluations. They do not assess the model's words, the caller's data,
a host's presentation, or overall WCAG 2.2 AA conformance.

| Check | Result | Evidence |
|---|---|---|
| Failed drafting says what happened and lists every attempt | Pass | `tests/sparql.rs`: `a_draft_that_never_validates_comes_back_failed_with_every_attempt`, run with the command and versions above. Asserts `no valid draft` at the start and three attempt sections; does not assess how understandable each error is. |
| Check output names its form and preview limit | Pass | `tests/sparql.rs`: `the_check_is_a_resource_of_its_own`, same run. Asserts `valid select over` and `preview (LIMIT 5)`; does not assess table reading order or alignment. |
| Draft reaching an unreadable graph is refused without leaking its content | Pass | `tests/sparql.rs`: `a_draft_reaching_a_graph_the_caller_cannot_read_is_refused_without_leaking_it`, same run. Checks refusal, no save and no graph content reaching the model; does not establish that every refusal is equally understandable. |
| Repeated attempt is explained on the first line | Pass | `tests/drafting.rs`: `a_repeat_says_drafting_stopped_and_is_recorded_as_a_repeat_not_a_failure`, same run. Asserts `drafting stopped: the model repeated attempt 1` on the first line and records the repeat in JSON and Turtle. |
| Saved draft with an empty preview says so on the first line | Pass | `tests/drafting.rs`: `a_draft_still_empty_after_the_last_attempt_is_saved_marked_never_silently`, same run. Asserts `preview is EMPTY` on the first line and a provenance warning; does not assess spoken pronunciation. |
| Automated accessibility checker | Blocked | No automated accessibility checker was run for this review; no host page or browser checker is configured here. Rust tests are not an accessibility checker. |
| Browser presentation, zoom, reflow and contrast | Blocked | No browser evaluation was run for this review. This library emits text, JSON and Turtle; a host presentation and browser session are needed to evaluate their rendering. |
| Manual keyboard | Blocked | No person performed a keyboard pass for this review; no interactive host session was available. |
| Screen reader | Blocked | No screen reader was used for this review; no screen reader session was available. The preview limitation below is a source observation, not a screen reader test result. |
| Screen magnifier | Blocked | No screen magnifier was used for this review; no magnifier session was available. |
| Speech input | Blocked | No speech-input software was used for this review; no speech-input session was available. |
| Switch access | Blocked | No switch-access software was used for this review; no switch-access session was available. |
| Braille display | Blocked | No braille display was used for this review; no braille display was available. |

Unrun checks remain Blocked. A historical run that CI does not repeat must also be recorded
as Blocked with its date and missing current evidence, rather than carried forward as Pass.

## Known limitations

Observed in the source on 2026-10-31 and reviewed again on 2026-11-12; these are
limitations, not measured assistive-technology results:

- The preview is a tab-separated table with no alignment, and its values are written as RDF
  terms (`<urn:…>`, `"first"`), which may make columns and RDF punctuation difficult to follow.
  `Preview::to_text` in `src/check.rs` joins columns with tabs; no screen reader was used
  to establish how it speaks them.
- A saved draft's first line opens with a long version IRI (a sha256 digest) before the words
  that say it is a draft.
- Some failures name a wire detail rather than what to do (`… did not answer its JSON form: …`,
  `serializing the check: …`).
- This repository's own text is English only; the model may answer in any language, and nothing
  states which.

## Contributing

Read this file before changing any output listed above: the plain answers, the prompts or the
refusals. Keep meaning in words, keep the first line saying what happened, and keep the model's
text separate from this crate's. In the pull request's Accessibility section, give each check
a row in a `Check | Result | Evidence` table: Pass, Fail or Blocked, with the test or command,
tool and version used, or why it could not run. Keep unrun checks as Blocked, including
assistive technologies nobody used.

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
| 2026-11-12 | Record current output tests and blocked browser and assistive-technology checks with evidence and scope. |
