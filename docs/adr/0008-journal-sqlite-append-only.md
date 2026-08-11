# 0008 — Journal SQLite append-only, global au workspace

- **Statut** : Acceptee
- **Date** : 2026-08-11

## Contexte

Chaque lecture, chaque ecriture admise et chaque verdict doit etre enregistre avec
sa provenance. Ce module a de la valeur **tout seul** : meme sans aucune detection de
conflit, un outil qui repond a « qui a ecrit cette ligne, dans quel projet, dans
quelle session, en reponse a quel prompt » est immediatement utile. C'est aussi
l'angle auditabilite du produit.

Deux questions : quel format, et ou.

## Decision

**SQLite via `rusqlite`, append-only, dans `~/Library/Application Support/Trame/`.**

Une base unique pour tous les projets, avec une colonne `project_id`.

```sql
projects(id, path, name, toolchain, added_at, last_opened_at)
sessions(id, project_id, name, harness, target_branch, work_item, state, created_at)
prompts(id, session_id, content, ts)
reads(id, project_id, session_id, path, hash, ts)
writes(id, project_id, session_id, seq, path, hash_before, hash_after, verdict, ts)
resource_claims(resource, project_id, session_id, claimed_at)

UNIQUE(project_id, seq)   -- la sequence est locale au projet, jamais globale
```

Append-only : on n'`UPDATE` pas, on n'efface pas. L'etat courant d'une session est
le dernier evenement la concernant.

## Consequences

- **Base globale, jamais dans le depot.** Trois raisons : ca ne pollue pas les
  projets, ca survit a leur suppression, et ca permet la question transverse —
  « qu'est-ce que j'ai fait cette semaine, tous projets confondus ». Cette derniere
  est la raison principale.
- Les libelles persistes (`verdict`, `harness`, `state`, source de work item) sont
  des **constantes stables**. Les changer demande une migration : le journal est
  append-only, les anciennes lignes ne se reecrivent pas.
- `UNIQUE(project_id, seq)` fait appliquer l'invariant du compteur par projet
  (ADR 0010) par la base, pas seulement par le code.
- L'append-only rend l'histoire reconstituable et supprime toute une classe de bugs
  d'etat incoherent. En echange, les requetes d'etat courant sont des
  `GROUP BY ... HAVING MAX(ts)` plutot que des `SELECT` simples.
- Un `Write` par ecriture admise, y compris les `Clean`. C'est volumineux et c'est
  le point : mesurer le taux de faux positifs (ADR 0007) suppose d'avoir aussi les
  cas propres.
- Une seule base signifie un seul point de contention en ecriture entre projets. Le
  mode WAL et le fait qu'il n'y ait pas de lecture sur le chemin chaud rendent ca
  negligeable a l'echelle visee.

## Alternatives ecartees

- **JSONL append-only.** Plus simple a ecrire, ingerable a interroger. La question
  « tous projets confondus, cette semaine » devient un script au lieu d'une requete,
  et l'auditabilite d'un outil dont on ne peut pas interroger le journal est
  theorique.
- **Une base par projet.** Perd la timeline transverse, qui est la raison d'etre du
  choix global. Et elle ne survivrait pas a la suppression du projet.
- **Une base dans le depot** (`.trame/journal.sqlite`). Pollue le projet, se
  retrouve dans les diffs ou dans le `.gitignore` de quelqu'un d'autre, et
  disparait avec le clone.
- **Mutable, avec l'etat courant en table.** Supprime l'auditabilite, qui est la
  moitie de la valeur de ce module.

## Ce qui invaliderait cette decision

Un volume qui rendrait la base ingerable. A quinze agents, l'ordre de grandeur est
de quelques milliers de lignes par jour ; SQLite est plusieurs ordres de grandeur
au-dessus. Le cas echeant, la reponse serait une politique de rotation par age, pas
un changement de stockage.
