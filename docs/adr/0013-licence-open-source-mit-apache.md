# 0013 — Licence open source : MIT OR Apache-2.0

- **Statut** : Acceptee
- **Date** : 2026-08-11
- **Remplace** : [0009](0009-licence-fsl-1-1-mit.md)

## Contexte

L'ADR 0009 avait choisi FSL-1.1-MIT, une licence *source-available* avec clause de
non-concurrence commerciale et conversion automatique en MIT a deux ans. Le
raisonnement etait defensif : garder le code public tout en empechant un acteur plus
gros d'en faire un service concurrent.

Ce raisonnement a un cout qui avait ete sous-estime :

- **La FSL n'est pas compatible OSI.** Le vocabulaire devait etre police en
  permanence (« ne jamais ecrire open source »), ce qui est un symptome : une licence
  qu'il faut defendre a chaque mention est une friction sur chaque conversation
  publique.
- **La protection reelle etait faible.** La clause vise l'usage commercial concurrent,
  or Trame est une application desktop locale, sans service a concurrencer. Ce qu'elle
  interdisait effectivement, personne n'allait le faire.
- **Le cout de contribution etait reel.** Une licence non OSI ecarte des
  contributeurs, decourage l'inclusion dans les distributions et complique
  l'empaquetage — cask Homebrew, nixpkgs.
- **Un CLA devenait necessaire** pour garder la possibilite de relicencier. C'est de
  l'administratif sur chaque contribution, pour une option qu'on vient d'exercer.

La protection ne venait donc pas de la licence. Pour un outil comme Trame, elle vient
de l'execution, de la marque, et du fait que le mecanisme de coordination est difficile
a copier — pas d'une clause.

## Decision

**MIT OR Apache-2.0**, au choix de l'utilisateur. C'est la convention de l'ecosysteme
Rust.

- `LICENSE-MIT` et `LICENSE-APACHE` a la racine.
- `license = "MIT OR Apache-2.0"` dans `[workspace.package]`, herite par les sept
  crates.
- Le texte d'Apache-2.0 est le fichier canonique recupere sur `apache.org`, verbatim
  (sha256 `cfc7749b…c523d30`). Pas une reproduction de memoire.

Trame est **open source**, sans guillemets et sans precaution de vocabulaire. La regle
qui interdisait le terme dans le depot est supprimee.

## Consequences

- **Plus de CLA.** Sous double licence permissive, une contribution est implicitement
  offerte sous les memes termes ; c'est la convention explicite de l'ecosysteme Rust,
  et le `README` la rappelle.
- **Quiconque peut forker, modifier et vendre** un produit derive de Trame. C'est
  assume. La reponse n'est pas une clause, c'est de continuer a livrer.
- Le dual permet a l'utilisateur de choisir : MIT pour la simplicite, Apache-2.0 pour
  la clause de brevet explicite que MIT n'a pas. Aucune des deux ne se substitue a
  l'autre, et cette combinaison est celle qui pose le moins de questions dans une
  revue juridique d'entreprise.
- **La marque reste la vraie protection.** Un depot open source n'empeche pas de
  deposer le nom, et c'est desormais le seul levier — ce qui etait deja le cas en
  pratique sous FSL.
- Plus de tableau de dates de conversion a tenir : il n'y a plus de conversion.
- Une contribution sous une troisieme licence incompatible est a refuser. C'est le
  seul point de vigilance qui reste.

### Ce que cette decision ne change pas

**La question de la licence de GitButler reste entiere** et strictement independante
(ADR 0003). Passer Trame en MIT/Apache ne donne aucun droit sur le code de GitButler,
exactement comme adopter la FSL ne lui en donnait pas. `but` reste une **dependance
externe installee par l'utilisateur, jamais vendorisee**, et c'est cette non-inclusion
qui porte l'analyse — pas la licence de Trame.

Un point neuf, en revanche : sous MIT/Apache, un tiers peut redistribuer Trame
commercialement. Si ce tiers empaquetait `but` avec, c'est **lui** qui se confronterait
a la clause de non-concurrence de GitButler. Raison supplementaire de ne jamais
vendoriser `but` et de le documenter comme prerequis.

## Alternatives ecartees

- **Rester en FSL-1.1-MIT.** C'est la decision que celui-ci remplace ; les motifs
  sont dans le contexte ci-dessus.
- **MIT seul.** Suffisant et tres lisible, mais sans clause de brevet explicite. Le
  cout du dual est un fichier de plus.
- **Apache-2.0 seul.** Bon choix, mais s'ecarte de la convention Rust et impose la
  licence la plus verbeuse a qui veut juste reutiliser trois lignes.
- **AGPL-3.0.** OSI, et elle garderait une protection contre un service heberge
  concurrent. Ecartee pour la meme raison que dans l'ADR 0009, qui reste valable :
  Trame est une application desktop locale, **la clause reseau ne mord sur rien**.
  Elle aurait donc le cout de repulsion de l'AGPL sans son benefice.

## Ce qui invaliderait cette decision

Un tiers qui prendrait le code pour en faire un service commercial a succes en
contribuant zero en retour, au point de rendre le projet original non viable. Meme
alors, relicencier n'effacerait pas les versions deja publiees : la reponse serait un
changement de licence sur les versions **futures**, et il faudrait le consentement des
contributeurs — c'est le prix explicite de l'abandon du CLA, et il est accepte en
connaissance de cause.
