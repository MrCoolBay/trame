# 0010 — Le parallelisme se fait par projets, pas par sessions

- **Statut** : Acceptee
- **Date** : 2026-08-11

## Contexte

Sans isolation (ADR 0002), le nombre de sessions simultanees dans un meme repertoire
de travail a une limite pratique : passe trois ou quatre agents, la probabilite de
recouvrement croit plus vite que la valeur ajoutee, et le taux d'avis de lecture
perimee devient du bruit.

Les concurrents achetent le parallelisme en payant des worktrees. Il existe un axe
moins cher, et il est deja isole :

> **Deux sessions dans deux projets differents ne peuvent physiquement pas entrer en
> collision.** Repertoires de travail distincts, depots distincts, index distincts.
> L'isolation est gratuite et parfaite.

C'est aussi ce que le developpeur fait deja naturellement : le back, le front,
l'infra, le side project.

## Decision

Le parallelisme se scale sur l'axe des **projets**. Deux a cinq sessions par projet,
et autant de projets qu'on veut.

```
5 projets × 3 sessions = 15 agents actifs
… sans jamais sortir du point de fonctionnement sur (3 par working dir)
```

Consequence architecturale, et c'est **le seul choix irreversible du projet** : le
Supervisor et le registre par projet doivent exister **des le premier commit**, meme
si l'interface n'affiche qu'un seul projet. Retrofitter un registre singleton global
en registre par projet est une reecriture, pas un refactor.

### Ce qui est par projet, ce qui est global

| Par projet | Global (workspace) |
|---|---|
| Write Registry (un acteur) | Journal SQLite unique, colonne `project_id` |
| **Compteur de sequence** | Reservations de ressources (ports, bases de dev) |
| Working directory, backend VCS | Budget de concurrence (CPU, RAM) |
| Watcher FSEvents | Quotas et rate limits API (lies au compte) |
| Branches virtuelles, config agent | Identifiants dans le Keychain |

Le point subtil : **les reservations de ressources doivent etre globales**. Le port
3000 est machine-wide. Deux projets qui lancent chacun leur dev server, c'est le
premier vrai conflit inter-projets — et c'est ce qui justifie enfin un registre de
ressources qui, en mono-projet, etait marginal.

Corollaire, inscrit comme invariant : **le numero de sequence est par projet, jamais
global**, avec `UNIQUE(project_id, seq)` en base (ADR 0008).

## Consequences

- Deux registres ne se parlent jamais. Pas de coordination inter-acteurs, pas de
  deadlock possible (ADR 0006).
- Tout ce qui ne sert qu'au-dela de cinq sessions par projet est **hors scope** :
  scheduler distribue, copy-on-write, cinquante sessions dans un projet.
- La detection de toolchain a l'ajout d'un projet n'est pas cosmetique : elle
  determine ce qui constitue l'etat partage du projet (`node_modules` et les ports,
  `target/`, `.venv`), donc les ressources a reserver globalement.
- Fermer un projet n'est pas le supprimer : on relache le watcher et les backends,
  les sessions restent persistees et reprennent a la reouverture.
- La v0.1 n'expose qu'un projet dans l'interface, mais paie deja le cout structurel
  du multi-projet. C'est deliberement du travail sans benefice visible immediat.

## Alternatives ecartees

- **Un registre global unique, multi-projet plus tard.** L'exacte erreur que cet ADR
  existe pour empecher. Le `ProjectId` se propage dans le journal, les messages
  d'acteurs, le compteur de sequence et les claims : l'ajouter apres coup touche tout.
- **Scaler par sessions dans un projet.** Ramene au probleme que l'ADR 0002 assume,
  mais a une echelle ou il n'est plus gerable.
- **Un registre par session.** Il n'y aurait plus rien a comparer entre sessions.

## Ce qui invaliderait cette decision

Rien qui soit connaissable a l'avance. Si l'usage reel montrait que huit sessions
dans un projet passent tres bien, le cadrage produit bougerait — mais l'architecture
par projet resterait correcte. C'est l'interet d'avoir choisi l'axe le plus
contraignant en premier.
