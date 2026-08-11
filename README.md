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
`git add`, ni `git push` — voir [`CLAUDE.md`](CLAUDE.md).

## Licence : Fair Source

Trame est publie sous **FSL-1.1-MIT** (Functional Source License). En trois lignes :

1. Le code est public — lecture, modification, fork, contribution, usage interne
   et professionnel, education et recherche non commerciales : tout est permis.
2. La seule chose interdite est l'**usage commercial concurrent** : expedier un
   produit ou un service commercial qui se substitue a Trame.
3. Chaque version passe **automatiquement sous licence MIT deux ans** apres sa
   mise a disposition, sans restriction d'aucune sorte.

C'est le modele **Fair Source**. La clause de non-concurrence le rend non
compatible OSI, donc le terme consacre est bien Fair Source et pas autre chose —
la nuance compte, autant l'ecrire correctement des le depart.

Texte complet et dates de conversion : [`LICENSE.md`](LICENSE.md).
