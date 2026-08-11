# Trame

Plusieurs agents de code en parallele, dans **un seul repertoire de travail** par
projet, avec une coordination explicite et observable au lieu de silencieuse.

macOS. Rust. Local-first : un binaire, pas de compte, pas de serveur, pas de cloud.

## La these

Tous les outils concurrents isolent chaque agent — un worktree git ou une copie de
repertoire par session. L'isolation supprime les collisions, mais elle rend aussi
la coordination *impossible* : chaque agent est aveugle aux autres par construction.

Trame fait le pari inverse — repertoire partage, coordination appliquee :

> Quand l'agent A s'apprete a ecrire, si un fichier qu'il a **lu** a ete modifie
> depuis par une autre session, A raisonne sur un monde qui n'existe plus.
> Trame le detecte et **l'en informe**.

Le mode d'echec que ca attrape ne produit aucune collision d'ecriture :

```
1. Session A lit auth.rs, memorise la signature de verify_token()
2. Session B modifie auth.rs, renomme verify_token() -> validate_token()
3. Session A ecrit handlers.rs, appelle verify_token()

Deux fichiers differents. Un systeme de verrous par fichier ne voit rien.
L'arbre est casse.
```

Trame ne bloque pas : il informe l'agent, qui relit et s'adapte tout seul dans la
grande majorité des cas.

## Etat

Phase 0 : fondations. Les frontieres de crates et les coutures sont posees, le
registre d'admission ne l'est pas encore. Voir [`docs/concept.md`](docs/concept.md)
pour le cadrage complet et [`docs/adr/`](docs/adr/) pour les decisions prises.

## Developper

```sh
just check     # compile tout, tests inclus
just test      # la suite complete
just lint      # fmt + clippy, zero warning tolere
just run       # le daemon
just tui       # l'interface
```

Prerequis : macOS sur Apple Silicon, une toolchain Rust stable, et la CLI
[GitButler](https://gitbutler.com) (`but`) installee et sur le `PATH`. Trame
appelle `but` comme dependance externe ; il ne l'embarque pas.

Ce depot est en **workspace mode GitButler**. On n'utilise ni `git commit`, ni
`git add`, ni `git push` — voir [`AGENTS.md`](AGENTS.md).

## Contribuer

Les regles du projet, les invariants d'architecture et les decisions prises sont dans
[`AGENTS.md`](AGENTS.md). Les lire avant d'ouvrir une change request fait gagner un
aller-retour.

Deux choses sont non negociables et verifiees par la CI : `just lint` sans le moindre
warning, et `just test` vert. Le reste se discute.

## Licence

Trame est **open source**, sous double licence au choix :

- [MIT](LICENSE-MIT)
- [Apache-2.0](LICENSE-APACHE)

Usage, modification, fork, redistribution y compris commerciale : tout est permis.
C'est la convention de l'ecosysteme Rust — MIT pour la concision, Apache-2.0 pour la
clause de brevet explicite que MIT n'a pas.

Sauf mention contraire de votre part, toute contribution que vous soumettez
intentionnellement pour inclusion dans Trame est offerte sous ces memes deux licences,
sans condition supplementaire. **Il n'y a pas de CLA.**

Le raisonnement derriere ce choix — et pourquoi le projet a quitte la FSL-1.1-MIT — est
dans l'[ADR 0013](docs/adr/0013-licence-open-source-mit-apache.md).

> Note : la CLI GitButler (`but`) est un logiciel distinct, sous FSL-1.1-MIT. Trame
> l'appelle comme prerequis externe et ne l'embarque **jamais**. Si vous redistribuez
> Trame, n'empaquetez pas `but` avec.
