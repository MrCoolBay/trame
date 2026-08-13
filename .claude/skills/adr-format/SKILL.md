---
name: adr-format
description: The format of Trame's Architecture Decision Records, and the criteria for knowing when to write one. Read before creating an ADR, before changing an existing decision, or when unsure whether a design choice deserves documenting.
---

# ADR format — Trame

## When to write one

A yes to **one** of these is enough:

- Would reversing this choice mean a rewrite rather than a refactor?
- Does this choice close a door — a platform, a model, a protocol, a licence?
- Would a competent developer spontaneously propose the opposite?
- Does the choice look arbitrary without its context?

## When not to write one

- A naming choice, a signature, a module split.
- A decision that can be undone in an afternoon.
- A coding rule: that belongs in a skill, not an ADR. An ADR records a dated decision; a
  skill prescribes permanent behaviour.
- A restatement of an existing ADR. Amend it, or supersede it.

## The format

File name: `NNNN-title-in-kebab-case.md`, numbered sequentially, never renumbered. Five
sections, in this order.

```markdown
# NNNN — Title, as an imperative or as a finding

- **Status**: Proposed | Accepted | Superseded by [NNNN](...)
- **Date**: YYYY-MM-DD

## Context

The facts, not the conclusion. What makes the choice necessary, which constraints exist,
what is known and what is not. A reader who stops here should be able to reach the
decision unaided.

## Decision

What we do. Affirmative, present tense. Precise enough that a violation is identifiable in
a code review.

## Consequences

The good **and** the bad. An ADR that lists only benefits has not been thought through.
Including the costs we accept paying and the problems we leave open.

## Alternatives rejected

What was considered, and **why not**. This is the section that avoids re-running the debate
in six months. An alternative with no stated reason for rejection does not count.

## What would invalidate this decision

The observable condition that would justify reopening the subject.
```

## The last section is the one that matters

> **An ADR with no re-examination condition is dogma, not a decision.**

It has to be **observable**, not rhetorical.

✅ **Correct**:

```markdown
## What would invalidate this decision

A false-positive rate that stays unacceptable **after** turning both available dials — the
read-set filter and the decay window — and after moving to hunk granularity. That is the
explicit trigger for the move to hunks, planned for v0.4.
```

There is a threshold, an order of attempts, and a deadline. Someone can observe that the
condition is met.

❌ **Counter-example**:

```markdown
## What would invalidate this decision

If we realise it was not a good idea, or if requirements change.
```

True of everything, therefore carrying no information. Writing that is worth less than
writing nothing: it creates the impression the question was handled.

If no condition is identifiable, write it as such — "Nothing foreseeable" — with the reason.
That is a legitimate answer, not an escape hatch (see ADR 0004).

## Statuses and lifecycle

- **Proposed** — under discussion, not yet applied in the code.
- **Accepted** — applied. The code honours it.
- **Superseded by [NNNN]** — we changed our mind.

> **An ADR is never deleted.** It is marked superseded and the new one is written,
> referencing the old. The history of decisions is worth more than the apparent tidiness of
> the index.

Amending an **accepted** ADR: allowed to correct a factual error or sharpen a consequence.
Forbidden to change the decision — that is a new ADR.

### The reference example in this repository

[ADR 0009](../../../docs/adr/0009-licence-fsl-1-1-mit.md) (the FSL licence) was superseded
by [0013](../../../docs/adr/0013-licence-open-source-mit-apache.md) (open source, MIT OR
Apache-2.0). The pattern to reproduce:

- the **body of 0009 is untouched** — history is not rewritten, even when it turned out
  wrong;
- its header carries the status `Superseded by [0013]` and a callout warning the reader,
  naming the rule that no longer applies;
- 0013 carries a `Supersedes: [0009]` field, and its Context section explains **why the
  original reasoning did not hold** rather than passing over it;
- the index strikes the old one through rather than removing it.

A superseded ADR stays useful: it documents a dead end, which keeps people from walking back
into it.

## After writing an ADR

Three things, or it is invisible:

1. Add the row to the table in `docs/adr/README.md`.
2. If the decision belongs in `AGENTS.md`'s table, add the link there.
3. Reference the ADR in the documentation of the module concerned — `//! (ADR 0007)`. An ADR
   you can only find by digging through `docs/` does not get read.

## Tone

Short. The ADRs in this repository run between 40 and 90 lines. You are writing for someone
who was not in the discussion and has to decide whether they may deviate.

**ADRs are written in English**, like the rest of the repository. Technical terms keep their
established form — read-set, hunk, worktree, backpressure — rather than an invented
translation. `just check-language` enforces this, with `docs/adr/` and `docs/sondes/`
temporarily excluded while their translation pass is pending. That exclusion is not
permission: a new ADR is written in English.
