---
name: adr-format
description: Format des Architecture Decision Records de Trame et criteres pour savoir quand en ecrire un. A lire avant de creer un ADR, de modifier une decision existante, ou quand on hesite a documenter un choix de conception.
---

# Format des ADR — Trame

## Quand en ecrire un

Oui a **une** de ces questions suffit :

- Revenir sur ce choix imposerait-il une reecriture plutot qu'un refactor ?
- Ce choix ferme-t-il une porte — une plateforme, un modele, un protocole, une licence ?
- Un developpeur competent proposerait-il spontanement le contraire ?
- Le choix parait-il arbitraire sans son contexte ?

## Quand ne pas en ecrire

- Un choix de nommage, une signature, un decoupage de module.
- Une decision qui se defait en une apres-midi.
- Une regle de codage : ca va dans une skill, pas dans un ADR. Un ADR enregistre une
  decision datee ; une skill prescrit un comportement permanent.
- Une reformulation d'un ADR existant. On le modifie ou on le remplace.

## Le format

Nom de fichier : `NNNN-titre-en-kebab-case.md`, numerote sequentiellement, jamais
renumerote. Cinq sections, dans cet ordre.

```markdown
# NNNN — Titre a l'imperatif ou au constat

- **Statut** : Proposee | Acceptee | Remplacee par [NNNN](...)
- **Date** : AAAA-MM-JJ

## Contexte

Les faits, pas la conclusion. Ce qui rend le choix necessaire, quelles contraintes
existent, ce qu'on sait et ce qu'on ne sait pas. Un lecteur qui s'arrete ici doit
pouvoir arriver seul a la decision.

## Decision

Ce qu'on fait. A l'affirmatif, au present. Assez precis pour qu'une violation soit
identifiable dans une revue de code.

## Consequences

Les bonnes **et** les mauvaises. Un ADR qui ne liste que des benefices n'a pas ete
reflechi. Y compris les couts qu'on accepte de payer et les problemes qu'on laisse
ouverts.

## Alternatives ecartees

Ce qu'on a envisage, et **pourquoi non**. C'est la section qui evite de refaire le
debat dans six mois. Une alternative sans motif de rejet ne compte pas.

## Ce qui invaliderait cette decision

La condition observable qui justifierait de rouvrir le sujet.
```

## La derniere section est celle qui compte

> **Un ADR sans condition de reexamen est un dogme, pas une decision.**

Elle doit etre **observable**, pas rhetorique.

✅ **Correct** :

```markdown
## Ce qui invaliderait cette decision

Un taux de faux positifs qui reste inacceptable **apres** avoir tourne les deux
cadrans disponibles — le filtre de read-set et la fenetre de decroissance — et apres
etre passe a une granularite hunk. C'est le declencheur explicite du passage aux
hunks, prevu en v0.4.
```

Il y a un seuil, un ordre d'essai, et une echeance. Quelqu'un peut constater que la
condition est remplie.

❌ **Contre-exemple** :

```markdown
## Ce qui invaliderait cette decision

Si on se rend compte que ce n'etait pas une bonne idee, ou si les besoins evoluent.
```

Vrai de tout, donc sans information. Ecrire ca vaut moins que ne rien ecrire : ca
donne l'impression que la question a ete traitee.

Si aucune condition n'est identifiable, l'ecrire tel quel — « Rien de previsible » —
avec la raison. C'est une reponse legitime, pas une echappatoire (voir l'ADR 0004).

## Statuts et cycle de vie

- **Proposee** — en discussion, pas encore appliquee dans le code.
- **Acceptee** — appliquee. Le code la respecte.
- **Remplacee par [NNNN]** — on a change d'avis.

> **On ne supprime jamais un ADR.** On le marque remplace et on ecrit le nouveau, qui
> reference l'ancien. L'historique des decisions a plus de valeur que la coherence
> apparente de l'index.

Modifier un ADR **accepte** : autorise pour corriger une erreur factuelle ou preciser
une consequence. Interdit pour changer la decision — ca, c'est un nouvel ADR.

### L'exemple de reference dans ce depot

L'[ADR 0009](../../../docs/adr/0009-licence-fsl-1-1-mit.md) (licence FSL) a ete
remplace par le [0013](../../../docs/adr/0013-licence-open-source-mit-apache.md)
(open source, MIT OR Apache-2.0). Le patron a reproduire :

- le **corps du 0009 est intact** — on ne reecrit pas l'histoire, meme quand elle a
  tort ;
- son en-tete porte le statut `Remplacee par [0013]` et un encadre qui previent le
  lecteur, en signalant nommement la regle qui ne s'applique plus ;
- le 0013 porte un champ `Remplace : [0009]`, et sa section Contexte explique
  **pourquoi le raisonnement initial n'a pas tenu** plutot que de le passer sous
  silence ;
- l'index barre l'ancien plutot que de le supprimer.

Un ADR remplace reste utile : il documente une impasse, ce qui evite d'y retourner.

## Apres avoir ecrit un ADR

Trois choses, sinon il est invisible :

1. Ajouter la ligne dans le tableau de `docs/adr/README.md`.
2. Si la decision figure dans le tableau de `AGENTS.md`, y ajouter le lien.
3. Referencer l'ADR dans la documentation de module concernee — `//! (ADR 0007)`. Un
   ADR qu'on ne trouve qu'en fouillant `docs/` n'est pas lu.

## Ton

Court. Les ADR de ce depot font entre 40 et 90 lignes. On ecrit pour quelqu'un qui
n'a pas assiste a la discussion et qui doit decider s'il peut deroger.

Le domaine s'ecrit en francais. Les termes techniques gardent leur forme d'origine —
read-set, hunk, worktree, backpressure — plutot qu'une traduction inventee.
