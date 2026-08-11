# 0002 — Aucune isolation : un repertoire de travail unique par projet

- **Statut** : Acceptee
- **Date** : 2026-08-11

## Contexte

L'orchestration multi-agents repose aujourd'hui sur un modele dominant : **un
worktree git par session**. Conductor, Xirp, Crystal, pdb-env font tous ca.
L'isolation physique est reelle et elle supprime effectivement les collisions
d'ecriture.

Elle a un cout direct — duplication du workspace, reinstallation des dependances,
recompilation par worktree, N branches a relire et faire atterrir separement — mais
ce n'est pas le probleme principal. Le probleme principal est plus profond :

> **L'isolation ne supprime pas seulement les collisions. Elle supprime la
> possibilite de coordonner.**

Deux agents dans deux worktrees ne peuvent pas savoir qu'ils travaillent sur des
hypotheses contradictoires. Ils sont aveugles l'un a l'autre **par construction**,
et aucune couche ajoutee par-dessus ne peut y remedier : au niveau de la forge il
est trop tard, le code est deja ecrit ; au niveau du systeme de fichiers c'est
impossible, il n'y a rien a observer en commun.

## Decision

**Aucune isolation.** Un repertoire de travail unique par projet, partage par
toutes ses sessions. Pas de worktree, pas de copie, pas de copy-on-write, pas de
conteneur, pas de microVM.

Ce n'est pas un renoncement a l'isolation faute de temps. C'est la **condition de
possibilite** du seul mecanisme qui differencie Trame : un registre d'admission ne
peut comparer des read-sets que si les sessions lisent le meme arbre.

## Consequences

- Les collisions deviennent possibles. C'est assume, et c'est precisement ce que
  le registre existe pour rendre **bruyant** au lieu de silencieux (ADR 0007).
- L'attribution est un simple etiquetage plutot qu'un merge : les changements sont
  marques, pas isoles, donc **le conflit n'a pas d'endroit ou naitre** (ADR 0003).
- Une seule installation de dependances, un seul cache de compilation, un seul
  environnement configure par projet.
- Le parallelisme ne peut plus s'obtenir en multipliant les sessions dans un
  projet. Il s'obtient en ajoutant des projets (ADR 0010).
- Les ecritures hors-bande — `sed -i`, hooks git, formatters, build — touchent le
  meme arbre que les agents. Elles sont rattrapees par FSEvents mais jamais
  admises. Assumer et afficher.

## Alternatives ecartees

- **Un worktree par session.** Supprime le produit avec le probleme.
- **Repertoire partage plus verrous par fichier.** Ne voit rien du scenario qui
  compte : A lit `auth.rs`, B ecrit `auth.rs`, A ecrit `handlers.rs`. Deux fichiers
  differents, aucune collision d'ecriture, arbre casse.
- **Isolation optionnelle, activable par session.** Deux modes a maintenir, deux
  modeles mentaux pour l'utilisateur, et le mode isole rendrait la fonctionnalite
  principale silencieusement inoperante. Le pire des deux mondes.

## Ce qui invaliderait cette decision

Un taux de collisions destructrices assez eleve, mesure sur usage reel, pour que le
cout depasse la valeur de la coordination — **et** l'echec des reponses moins
radicales (granularite plus fine, blocage du niveau 3, detecteur de quiescence).
Dans ce cas la reponse ne serait pas « isoler », ce serait « reduire le nombre de
sessions par projet », ce qui est deja le cadrage produit : deux a cinq.
