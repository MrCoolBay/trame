# 0009 — Licence FSL-1.1-MIT (Fair Source)

- **Statut** : **Remplacee par [0013](0013-licence-open-source-mit-apache.md)**
- **Date** : 2026-08-11
- **Remplacee le** : 2026-08-11

> ⚠️ **Cet ADR ne decrit plus le projet.** Trame est passe en open source sous
> **MIT OR Apache-2.0** — voir l'[ADR 0013](0013-licence-open-source-mit-apache.md),
> qui explique pourquoi le raisonnement ci-dessous n'a pas tenu. Le texte est conserve
> intact : l'historique des decisions a plus de valeur que la coherence apparente de
> l'index.
>
> En particulier, la regle « ne jamais ecrire open source » de la section
> « Vocabulaire » ci-dessous **ne s'applique plus**.

## Contexte

Trame est un projet solo qui pourrait devenir un produit. Il faut un code public —
pour la confiance, la contribution et l'angle auditabilite — sans offrir a un acteur
plus gros la possibilite d'en faire un service concurrent.

## Decision

**FSL-1.1-MIT**, la Functional Source License avec MIT comme licence future.
Exactement le modele GitButler.

- Le code est public : lecture, modification, contribution, usage interne, fork,
  produits non concurrents, education et recherche non commerciales.
- Interdit : expedier un produit ou service **commercial** qui se substitue a Trame.
- **Conversion automatique en MIT au deuxieme anniversaire de chaque version.** Les
  dates sont tenues dans un tableau au bas de `LICENSE.md`.

### Vocabulaire — ce point n'est pas anecdotique

> **Ne jamais ecrire « open source ».** Le terme est **Fair Source**.

La clause de non-concurrence rend la FSL non compatible OSI. GitButler s'est fait
reprendre publiquement la-dessus au moment de son annonce. Sur HN et Reddit, ce
n'est pas un detail de vocabulaire, et un projet solo n'a pas besoin de ce proces en
ouverture.

Corollaire pratique : le mot n'apparait nulle part dans le depot, hors la copie du
document de concept ou il est justement question de ne pas l'employer.

## Consequences

- Un **CLA** est a prevoir sur les contributions si on veut garder la possibilite de
  relicencier plus tard. A poser avant la premiere contribution externe, pas apres.
- Une marque deposee sur le nom est la vraie protection dans ce type de licence. La
  licence protege le code ; la marque protege le produit.
- La conversion a deux ans est un engagement **irrevocable**. C'est ce qui rend le
  modele acceptable pour un contributeur, et c'est aussi la porte de sortie de tout
  le monde, Trame compris.
- « Pas forkable » n'est pas ce que fait la FSL. Elle **autorise** le fork, la
  modification et la redistribution ; elle interdit l'usage commercial concurrent.
  Si l'objectif etait vraiment d'empecher le fork, il faudrait une licence
  proprietaire source-available — ce qui isole beaucoup plus de la communaute pour
  un gain de protection reelle faible.

### Le point que cet ADR ne regle pas

**Adopter la FSL pour Trame ne donne aucun droit sur le code de GitButler.** Ce sont
deux questions independantes, et se rassurer a bon compte ici serait une erreur. La
question GitButler est traitee dans l'ADR 0003 et reste ouverte.

### Verification a faire

Le texte de `LICENSE.md` doit etre compare au texte canonique publie sur
`fsl.software` avant toute publication du depot. Une licence reproduite de memoire
n'est pas une licence.

## Alternatives ecartees

- **MIT ou Apache-2.0 d'emblee.** Aucune protection contre un service concurrent.
- **AGPL.** Protege contre le SaaS concurrent, mais Trame est une application
  desktop locale : la clause reseau ne mord sur rien. Et l'AGPL effraie les
  contributeurs professionnels sans rien apporter ici.
- **Proprietaire source-available.** Plus protecteur en apparence, beaucoup plus
  isolant, et sans reelle protection supplementaire.
- **BUSL-1.1.** Meme famille, mais la conversion se fait vers une licence choisie
  par l'editeur et la clause de restriction est plus large et moins lisible. La FSL
  est plus courte et plus explicite.

## Ce qui invaliderait cette decision

Un choix de positionnement qui ferait de Trame un projet communautaire plutot qu'un
produit. Dans ce cas, MIT directement — et le CLA aurait servi a rendre ce
changement possible.
