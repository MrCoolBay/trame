---
name: journal-schema
description: SQLite conventions for Trame's journal — append-only, migrations, UNIQUE(project_id, seq), query and column-naming rules. Read before touching the schema, adding a table, or writing a query in trame-journal.
---

# The SQLite journal — Trame

One database, global, at `~/Library/Application Support/Trame/trame.sqlite`. **Never inside
the repository**: it does not pollute projects, it survives their deletion, and it makes the
cross-project question possible — "what did I do this week, across every project". That last
one is the main reason (ADR 0008).

## Rule 1 — Append-only

We do **not `UPDATE`**, and we do **not delete**. An entity's current state is the last
event concerning it.

✅ **Correct** — a state change is one more row:

```sql
INSERT INTO session_events (session_id, state, detail, ts) VALUES (?1, ?2, ?3, ?4);

-- The current state is read as:
SELECT state FROM session_events WHERE session_id = ?1 ORDER BY ts DESC, id DESC LIMIT 1;
```

❌ **Counter-example** — the history is lost, and auditability with it:

```sql
UPDATE sessions SET state = 'failed' WHERE id = ?1;
```

`ORDER BY ts DESC, id DESC` and not `ts` alone: two events can share a millisecond, and the
autoincrementing `id` breaks the tie in real insertion order.

## Rule 2 — `UNIQUE(project_id, seq)`

The sequence number is **per project, never global** (ADR 0010). The constraint is not
decorative: it makes the database enforce the invariant rather than the code alone, so a
counter bug fails at insertion instead of silently producing a false journal.

```sql
CREATE TABLE writes (
    id          INTEGER PRIMARY KEY,
    project_id  TEXT    NOT NULL REFERENCES projects(id),
    session_id  TEXT    NOT NULL REFERENCES sessions(id),
    seq         INTEGER NOT NULL,
    path        TEXT    NOT NULL,
    hash_before TEXT,               -- NULL = file creation
    hash_after  TEXT    NOT NULL,
    verdict     TEXT    NOT NULL,   -- Verdict::label(), a stable value
    ts          TEXT    NOT NULL,   -- ISO-8601 UTC
    UNIQUE (project_id, seq)
);

CREATE INDEX writes_project_ts  ON writes (project_id, ts DESC);
CREATE INDEX writes_path        ON writes (project_id, path);
CREATE INDEX writes_session     ON writes (session_id);
```

## Rule 3 — Persisted labels are stable constants

Every stored enum value goes through a `label()` method on the Rust side. **Changing a
`label()` value requires a migration**: the journal is append-only, and old rows do not
rewrite themselves.

Concerned: `Verdict::label()`, `Harness::label()`, `SessionState::label()`,
`TaskSourceKind::label()`.

❌ **Counter-example**:

```rust
// Persisting an enum's Debug. The day the variant is renamed, the database lies.
stmt.execute(params![format!("{verdict:?}")])?;
```

## Rule 4 — Column types

| Data | SQLite type | Form |
|---|---|---|
| Identifiers (`ProjectId`, `SessionId`) | `TEXT` | lowercase UUID with hyphens |
| Timestamps | `TEXT` | ISO-8601 UTC, **never** local time |
| Fingerprints | `TEXT` | blake3 hex, 64 characters |
| Sequences | `INTEGER` | |
| Paths | `TEXT` | **relative to the project root** |
| Verdicts, states, harness | `TEXT` | the `label()` value |

Paths are relative: an absolute path breaks the moment the project is moved, and it leaks a
personal directory tree into a journal that is meant to be shareable.

Timestamps as ISO-8601 text rather than integers: the journal stays readable by eye, which
matters for a tool whose main argument is auditability.

## Rule 5 — Migrations

- One numbered `.sql` file per migration, never modified afterwards — **the rule starts
  from the first published version**. Before that, no deployed database exists: amending
  migration 1 is then cleaner than stacking a corrective migration that would rename a
  column, which the rules below forbid precisely. Saying "never" without that nuance would
  make the rule false, and therefore ignored.
- A `schema_version` table with a single row.
- Migrations are **additive**: add a table, add a nullable column. Never rename, never
  drop, never change a type.
- Every migration runs inside a transaction. A half-applied migration is worse than a
  failed one.
- A migration that needs to reinterpret old rows writes a **new column** and leaves the old
  one in place.

## Rule 6 — Querying

```rust
// ✅ Bound parameters, always.
conn.execute(
    "INSERT INTO reads (project_id, session_id, path, hash, ts) VALUES (?1, ?2, ?3, ?4, ?5)",
    params![project_id.to_string(), session_id.to_string(), rel_path, hash.to_hex(), ts.to_rfc3339()],
)?;
```

```rust
// ❌ String concatenation. Injection, plus a plan rebuild on every call.
conn.execute(&format!("INSERT INTO reads (path) VALUES ('{path}')"), [])?;
```

- **Columns named explicitly**, never `SELECT *`. An additive migration would break every
  column index.
- **No query on admission's hot path.** The registry answers from memory; the journal is a
  sink, not a source. A SQLite read inside `Admit` would turn a microsecond verdict into a
  millisecond one.
- The journal is written **after** the verdict is returned, never before.
- **WAL** mode enabled on open: a writer does not block readers, which matters for a
  database shared across projects.

## What this journal must be able to answer

If one of these questions becomes hard to write, the schema has drifted:

1. Who wrote this file, in which session, in response to which prompt?
2. Which non-`Clean` verdicts on this project this week? (ADR 0007's false-positive rate is
   measured there)
3. What did I do this week, across every project?
4. What is a work item's full chain: issue, session, writes, branch?
