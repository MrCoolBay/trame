---
name: rust-conventions
description: Idiomes Rust du projet Trame. A lire avant d'ecrire ou de modifier du code Rust ici — decoupage thiserror/anyhow, instrumentation tracing, interdiction d'unwrap, structure des modules, conventions de nommage.
---

# Conventions Rust — Trame

Six regles. Chacune est appliquee par clippy au niveau du workspace, donc une
violation echoue la CI.

## 1. Erreurs : `thiserror` en bibliotheque, `anyhow` en binaire

Une bibliotheque qui renvoie `anyhow::Error` force son appelant a faire du pattern
matching sur des chaines de caracteres. Les seuls `anyhow` du depot sont dans
`crates/trame-daemon/src/main.rs` et `apps/trame-tui/src/main.rs`.

✅ **Correct** — bibliotheque, erreur typee, contexte porte par les variantes :

```rust
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("session inconnue du registre : {0}")]
    UnknownSession(SessionId),
    #[error("impossible de hasher {path}")]
    Hash {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub fn admit(&mut self, session: SessionId) -> Result<Verdict, RegistryError> {
    let state = self
        .sessions
        .get_mut(&session)
        .ok_or(RegistryError::UnknownSession(session))?;
    // ...
}
```

❌ **Contre-exemple** — l'appelant ne peut rien decider :

```rust
pub fn admit(&mut self, session: SessionId) -> anyhow::Result<Verdict> {
    let state = self
        .sessions
        .get_mut(&session)
        .context("session inconnue")?;   // <- une String. Bon courage pour reagir.
}
```

Regles secondaires :
- `#[source]` sur l'erreur d'origine, toujours. Une chaine d'erreurs cassee perd la
  cause reelle.
- Messages en minuscules, sans point final, en francais. Ils sont concatenes par
  l'affichage de la chaine.
- Un enum d'erreur public est `#[non_exhaustive]` : ajouter une variante ne doit pas
  etre un changement cassant.

## 2. Aucun `unwrap()`, `expect()`, `panic!()` hors des tests

`unwrap_used`, `expect_used` et `panic` sont en `deny` dans
`[workspace.lints.clippy]`. `clippy.toml` porte `allow-unwrap-in-tests = true` et
ses equivalents : dans un test, `unwrap()` est la facon la plus lisible d'echouer,
et il est autorise.

✅ **Correct** :

```rust
let Some(read) = self.read_set.get(path) else {
    return Ok(Verdict::Clean);   // pas de lecture enregistree : rien a signaler
};
```

❌ **Contre-exemple** :

```rust
let read = self.read_set.get(path).unwrap();   // panique = daemon mort = sessions perdues
```

Un `todo!()` est autorise et voulu pour une couture non implementee : il est
explicite et localise. `todo = "allow"` dans le workspace.

## 3. Toute I/O est instrumentee avec `tracing`

Jamais de `println!` ni `eprintln!` — `print_stdout` et `print_stderr` sont en
`deny`. Les logs vont sur **stderr** : stdout appartient au terminal alternatif de
ratatui et au JSON-RPC d'ACP.

✅ **Correct** — champs structures, pas d'interpolation :

```rust
#[tracing::instrument(skip(content), fields(bytes = content.len()))]
async fn admit(&mut self, session: SessionId, path: &Path, content: &str) -> Verdict {
    let verdict = self.evaluate(session, path, content);
    tracing::info!(verdict = verdict.label(), seq = %self.seq, "ecriture admise");
    verdict
}
```

❌ **Contre-exemple** :

```rust
println!("admis {} pour {}", path.display(), session);   // ecrase l'affichage du TUI
tracing::info!("verdict {:?} pour {}", verdict, session); // ni filtrable ni requetable
```

`skip` sur les contenus de fichiers, toujours : un `content` de 40 ko dans un log
est inutilisable, et il peut contenir des secrets.

## 4. Documentation obligatoire sur tout item public

`missing_docs = "warn"` plus `-D warnings` en CI : un item public sans doc echoue le
build. Ca inclut les **champs** de structures publiques et les variantes d'enums.

La doc dit *pourquoi*, pas *quoi*. `/// L'identifiant de la session.` sur un champ
nomme `session_id` ne sert a personne.

✅ **Correct** :

```rust
/// Le numero de sequence d'une ecriture admise.
///
/// **Local au projet, jamais global.** Un compteur global serait un point de
/// contention entre projets qui, par construction, ne peuvent pas entrer en
/// collision.
pub struct Seq(u64);
```

## 5. Structure des modules

- Un module par concept, declare dans `lib.rs`, re-exporte a plat en bas de `lib.rs`.
  L'appelant ecrit `trame_core::Verdict`, pas `trame_core::verdict::Verdict`.
- **Pas de `mod.rs`.** Un module avec enfants s'ecrit `foo.rs` plus `foo/bar.rs`.
- Les `use` sont groupes en trois blocs separes par une ligne vide, dans cet ordre :
  `std`, crates externes, `crate`/`super`. `rustfmt` ne le fait pas tout seul, c'est
  a la main.
- Les tests unitaires vivent dans un `mod tests` en bas du fichier qu'ils testent.
  Les tests d'integration vont dans `tests/`.

## 6. Nommage

- **Domaine en francais dans les commentaires et la documentation ; identifiants en
  anglais.** Les noms de tests, en revanche, sont en francais et descriptifs :
  `fn stale_read_est_admis_et_notifie()` se lit comme une specification.
- Vocabulaire impose : `ChangeRequest`, **jamais** `PullRequest` (ADR 0011).
  `Verdict`, pas `ConflictResult`. `Admit`, pas `CheckWrite`.
- Newtypes systematiques sur les identifiants. Un `SessionId` ne doit jamais etre
  interchangeable avec un `ProjectId`, meme si les deux portent un UUID.
- `Handle` en suffixe pour la poignee clonable d'un acteur : `RegistryHandle`.
- `label()` pour la representation stable persistee en base. Ne jamais changer une
  valeur de `label()` sans migration : le journal est append-only.

## Avant de valider

```sh
just lint    # fmt --check + clippy -D warnings. Exactement ce que la CI verifie.
just test
```
