---
name: journal-schema
description: Conventions SQLite du journal Trame — append-only, migrations, UNIQUE(project_id, seq), regles de requetage et de nommage des colonnes. A lire avant de toucher au schema, d'ajouter une table, ou d'ecrire une requete dans trame-journal.
---

# Le journal SQLite — Trame

Une base unique, globale, dans `~/Library/Application Support/Trame/trame.sqlite`.
**Jamais dans le depot** : ca ne pollue pas les projets, ca survit a leur
suppression, et ca permet la question transverse — « qu'est-ce que j'ai fait cette
semaine, tous projets confondus ». Cette derniere est la raison principale (ADR 0008).

## Regle 1 — Append-only

On **n'`UPDATE` pas**, on **n'efface pas**. L'etat courant d'une entite est le
dernier evenement la concernant.

✅ **Correct** — un changement d'etat est une ligne de plus :

```sql
INSERT INTO session_events (session_id, state, detail, ts) VALUES (?1, ?2, ?3, ?4);

-- L'etat courant se lit :
SELECT state FROM session_events WHERE session_id = ?1 ORDER BY ts DESC, id DESC LIMIT 1;
```

❌ **Contre-exemple** — l'histoire est perdue, et avec elle l'auditabilite :

```sql
UPDATE sessions SET state = 'failed' WHERE id = ?1;
```

`ORDER BY ts DESC, id DESC` et pas seulement `ts` : deux evenements peuvent partager
une milliseconde, et `id` autoincremente departage dans l'ordre d'insertion reel.

## Regle 2 — `UNIQUE(project_id, seq)`

Le numero de sequence est **local au projet, jamais global** (ADR 0010). La contrainte
n'est pas decorative : elle fait appliquer l'invariant par la base et pas seulement
par le code, donc un bug de compteur echoue a l'insertion au lieu de produire
silencieusement un journal faux.

```sql
CREATE TABLE writes (
    id          INTEGER PRIMARY KEY,
    project_id  TEXT    NOT NULL REFERENCES projects(id),
    session_id  TEXT    NOT NULL REFERENCES sessions(id),
    seq         INTEGER NOT NULL,
    path        TEXT    NOT NULL,
    hash_before TEXT,               -- NULL = creation de fichier
    hash_after  TEXT    NOT NULL,
    verdict     TEXT    NOT NULL,   -- Verdict::label(), valeur stable
    ts          TEXT    NOT NULL,   -- ISO-8601 UTC
    UNIQUE (project_id, seq)
);

CREATE INDEX writes_project_ts  ON writes (project_id, ts DESC);
CREATE INDEX writes_path        ON writes (project_id, path);
CREATE INDEX writes_session     ON writes (session_id);
```

## Regle 3 — Les libelles persistes sont des constantes stables

Toute valeur d'enum stockee passe par une methode `label()` cote Rust. **Changer une
valeur de `label()` exige une migration** : le journal est append-only, les anciennes
lignes ne se reecrivent pas.

Concernes : `Verdict::label()`, `Harness::label()`, `SessionState::label()`,
`TaskSourceKind::label()`.

❌ **Contre-exemple** :

```rust
// Persister le Debug d'un enum. Le jour ou on renomme la variante, la base ment.
stmt.execute(params![format!("{verdict:?}")])?;
```

## Regle 4 — Types de colonnes

| Donnee | Type SQLite | Forme |
|---|---|---|
| Identifiants (`ProjectId`, `SessionId`) | `TEXT` | UUID en minuscules avec tirets |
| Horodatages | `TEXT` | ISO-8601 UTC, **jamais** d'heure locale |
| Empreintes | `TEXT` | hex blake3, 64 caracteres |
| Sequences | `INTEGER` | |
| Chemins | `TEXT` | **relatifs a la racine du projet** |
| Verdicts, etats, harness | `TEXT` | la valeur de `label()` |

Les chemins sont relatifs : un chemin absolu casse des que le projet est deplace, et
il fait fuiter l'arborescence personnelle dans un journal cense etre partageable.

Les horodatages en texte ISO-8601 plutot qu'en entier : le journal reste lisible a
l'oeil nu, ce qui compte pour un outil dont l'argument principal est l'auditabilite.

## Regle 5 — Migrations

- Un fichier `.sql` numerote par migration, jamais modifie apres coup.
- Une table `schema_version` a une seule ligne.
- Les migrations sont **additives** : ajouter une table, ajouter une colonne
  nullable. Jamais renommer, jamais supprimer, jamais changer un type.
- Chaque migration tourne dans une transaction. Une migration a moitie appliquee est
  pire qu'une migration echouee.
- Une migration qui doit reinterpreter d'anciennes lignes ecrit une **nouvelle
  colonne** et laisse l'ancienne en place.

## Regle 6 — Requetage

```rust
// ✅ Parametres lies, toujours.
conn.execute(
    "INSERT INTO reads (project_id, session_id, path, hash, ts) VALUES (?1, ?2, ?3, ?4, ?5)",
    params![project_id.to_string(), session_id.to_string(), rel_path, hash.to_hex(), ts.to_rfc3339()],
)?;
```

```rust
// ❌ Concatenation de chaines. Injection, et rebuild du plan a chaque appel.
conn.execute(&format!("INSERT INTO reads (path) VALUES ('{path}')"), [])?;
```

- **Colonnes nommees explicitement**, jamais `SELECT *`. Une migration additive
  casserait tous les indices de colonnes.
- **Aucune requete sur le chemin chaud de l'admission.** Le registre repond depuis sa
  memoire ; le journal est un puits, pas une source. Une lecture SQLite dans
  `Admit` transformerait un verdict en microsecondes en verdict en millisecondes.
- Le journal s'ecrit **apres** que le verdict est rendu, jamais avant.
- Mode **WAL** active a l'ouverture : un ecrivain n'empeche pas les lecteurs, ce qui
  compte avec une base partagee entre projets.

## Ce qu'on doit pouvoir demander a ce journal

Si une de ces questions devient difficile a ecrire, le schema a derive :

1. Qui a ecrit ce fichier, dans quelle session, en reponse a quel prompt ?
2. Quels verdicts non-`Clean` sur ce projet cette semaine ? (le taux de faux positifs
   de l'ADR 0007 se mesure la)
3. Qu'est-ce que j'ai fait cette semaine, tous projets confondus ?
4. Quelle est la chaine complete d'un work item : issue, session, ecritures, branche ?
