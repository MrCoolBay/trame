---
name: concurrency-testing
description: Tester du code concurrent de facon deterministe dans Trame — injection d'horloge, ordonnancement controle, aucun sleep dans les tests, controle negatif obligatoire sur tout dispositif de mesure. A lire avant d'ecrire un test qui touche a un acteur, au temps ou a plusieurs sessions, et avant d'ecrire une sonde ou un canari.
---

# Tester la concurrence — Trame

## La regle

> **Aucun `sleep` dans un test. Jamais.**

Un `sleep` dans un test signifie l'une de deux choses : soit le test attend un temps
reel, et il est lent ; soit il attend un ordonnancement, et il est instable. Les deux
sont inacceptables, et le second est le pire : un test qui echoue une fois sur trente
est un test qu'on finit par ignorer, et une suite qu'on ignore ne protege plus rien.

Le registre est precisement le composant ou ce risque est maximal — ses verdicts
dependent de l'ordre des evenements et de l'ecoulement du temps.

## Technique 1 — Injecter l'horloge

`trame_core::Clock` existe pour ca. Le registre prend une decision qui depend du
temps : une entree du read-set expire au bout de dix minutes. Le tester en vrai
demanderait un test de dix minutes.

✅ **Correct** — le temps n'avance que sur ordre, le test est instantane :

```rust
use std::sync::Arc;
use chrono::TimeDelta;
use trame_core::clock::{Clock, ManualClock};

#[tokio::test]
async fn une_lecture_expiree_ne_declenche_plus_d_avis() {
    let clock = Arc::new(ManualClock::new());
    let (registry, _join) = spawn_registry(project, clock.clone());

    registry.record_read(session_a, "auth.rs").await.unwrap();
    registry.admit(session_b, "auth.rs", "// modifie").await.unwrap();

    // Juste avant l'expiration : l'avis est encore pertinent.
    clock.advance(TimeDelta::minutes(9));
    let verdict = registry.admit(session_a, "handlers.rs", "// ...").await.unwrap();
    assert_eq!(verdict.level(), 1, "a 9 min, la lecture compte encore");

    // Apres : le contexte de l'agent a tourne, on se tait.
    clock.advance(TimeDelta::minutes(2));
    registry.record_read(session_a, "auth.rs").await.unwrap();
    registry.admit(session_b, "auth.rs", "// encore").await.unwrap();
    clock.advance(TimeDelta::minutes(11));
    let verdict = registry.admit(session_a, "other.rs", "// ...").await.unwrap();
    assert_eq!(verdict, Verdict::Clean, "au-dela de 10 min, plus d'avis");
}
```

❌ **Contre-exemple** — non seulement lent, mais faux : il teste l'horloge systeme.

```rust
#[tokio::test]
async fn une_lecture_expiree_ne_declenche_plus_d_avis() {
    registry.record_read(session_a, "auth.rs").await.unwrap();
    tokio::time::sleep(Duration::from_secs(601)).await;   // dix minutes de CI
    // ...
}
```

`ManualClock` vit derriere la feature `test-support` de `trame-core`. Elle s'active
en dev-dependency, jamais en dependance de production.

## Technique 2 — Ordonner par les messages, pas par le temps

Un acteur traite ses messages un par un. Un `await` sur le `oneshot` de reponse est
donc une **barriere de synchronisation exacte** : quand `admit(...).await` rend la
main, le message est traite et l'etat de l'acteur inclut son effet. Il n'y a rien a
attendre de plus.

C'est ce qui rend le scenario canonique du produit testable en cinq lignes
deterministes, **sans le moindre agent** :

```rust
#[tokio::test]
async fn stale_read_sans_collision_d_ecriture() {
    let clock = Arc::new(ManualClock::new());
    let (registry, _join) = spawn_registry(project, clock.clone());

    // 1. A lit auth.rs
    registry.record_read(session_a, "auth.rs").await.unwrap();

    // 2. B ecrit auth.rs  -> Clean : personne d'autre n'a lu ce que B ecrase
    let verdict_b = registry.admit(session_b, "auth.rs", "fn validate_token()").await.unwrap();
    assert_eq!(verdict_b, Verdict::Clean);

    // 3. A ecrit handlers.rs -> StaleRead, alors qu'il n'y a AUCUNE collision
    //    d'ecriture : deux fichiers differents. Un verrou par fichier ne verrait rien.
    let verdict_a = registry.admit(session_a, "handlers.rs", "verify_token()").await.unwrap();
    let Verdict::StaleRead { stale } = verdict_a else {
        panic!("attendu StaleRead, obtenu {verdict_a:?}");
    };
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].path, PathBuf::from("auth.rs"));
    assert_eq!(stale[0].last_writer, session_b);
}
```

Ce test est **la raison d'etre du produit**. S'il casse, ce n'est pas le test qui a
un probleme.

## Technique 3 — `start_paused` pour le temps de tokio

Quand c'est le temps de tokio lui-meme qui est en jeu — un `interval`, un `timeout` —
et pas l'horloge metier :

```rust
#[tokio::test(start_paused = true)]
async fn un_timeout_d_admission_est_signale() {
    // Le temps virtuel de tokio avance instantanement jusqu'au prochain reveil.
    tokio::time::advance(Duration::from_secs(30)).await;
    // ...
}
```

Deux horloges, deux usages. `ManualClock` pour les decisions metier, `start_paused`
pour les primitives temporelles de tokio. Ne pas les melanger dans un meme test :
on ne sait plus laquelle on teste.

## Technique 4 — La concurrence reelle, quand elle est le sujet

Pour verifier que N sessions concurrentes produisent bien un ordre total coherent,
on lance vraiment en parallele, mais on assertionne sur des **invariants**, pas sur un
ordre precis.

```rust
#[tokio::test]
async fn les_sequences_sont_uniques_et_contigues_sous_charge() {
    let (registry, _join) = spawn_registry(project, Arc::new(SystemClock));

    let mut set = tokio::task::JoinSet::new();
    for i in 0..50 {
        let registry = registry.clone();
        set.spawn(async move {
            registry.admit(sessions[i % 3], &format!("f{i}.rs"), "x").await
        });
    }
    while set.join_next().await.is_some() {}

    let snapshot = registry.snapshot().await.unwrap();
    let mut seqs: Vec<u64> = snapshot.writes.iter().map(|w| w.seq.get()).collect();
    seqs.sort_unstable();
    seqs.dedup();
    assert_eq!(seqs.len(), 50, "aucun numero de sequence ne doit etre reutilise");
    assert_eq!(seqs.first().copied(), Some(1));
    assert_eq!(seqs.last().copied(), Some(50), "et aucun trou");
}
```

❌ **Contre-exemple** — assertionner un ordre que rien ne garantit :

```rust
assert_eq!(snapshot.writes[0].session, session_a);  // depend de l'ordonnanceur
```

## ★ Un test qui simule un tiers ne verifie pas le tiers

**La regle**, et elle est nee de trois bugs consecutifs :

> **Tout test qui simule un tiers doit etre double d'un test qui interroge le vrai tiers.**

Le faux agent en memoire est indispensable — il rend le transport deterministe, sans
sous-process, sans authentification, sans jeton consomme. Mais il a un defaut structurel :
**c'est nous qui ecrivons ce que le tiers repond.** Un test qui fabrique la reponse qu'il
attend verifie sa propre fiction, et il reste vert pendant que le produit est casse.

### Les trois bugs, et ce qu'ils ont en commun

| Bug | Ce que le test simule | Ce que le tiers fait vraiment |
|---|---|---|
| Fin de tour jamais detectee | le test emettait un `sessionUpdate` « end_of_turn » | **cette notification n'existe pas** — la fin de tour est la reponse a `session/prompt` |
| Appels d'outil invisibles | le test n'emettait que des `tool_call` | l'adaptateur emet parfois **uniquement** des `tool_call_update` |
| Lectures jamais enregistrees | le test faisait toujours lire par `fs/read_text_file` | l'agent lit par `Grep` ou `Bash`, qui **echappent** a l'interception |

Le premier est le plus instructif : le test **fabriquait la notification qu'il attendait**.
Il est reste vert pendant toute la phase 2 et a valide un chemin qui n'existait pas. Le
blocage n'est apparu qu'a la premiere manche experimentale, quand un vrai agent a du
enchainer deux tours.

Les trois ont ete trouves **hors des tests**, en interrogeant le tiers.

### La technique qui les a trouves

Orienter `CLAUDE_CODE_EXECUTABLE` vers un faux `claude` qui n'ecrit que son `argv`, puis
faire la **vraie negociation** contre le **vrai adaptateur**. On lit alors la ligne de
commande que l'adaptateur *aurait* donnee au vrai binaire.

```sh
#!/bin/sh
for a in "$@"; do echo "$a" >> "$CAPTURE"; done
exit 0
```

Trois proprietes qui en font une technique et pas une bricole :

- **Aucune authentification, aucun jeton consomme.** Le modele n'est jamais sollicite.
- **Le garde-fou anti-imbrication ne s'applique pas** : le vrai `claude` n'est jamais lance,
  donc ca tourne meme depuis une session Claude Code, et en CI.
- **Ca observe le tiers, pas notre simulation de lui.** C'est tout l'interet.

C'est ce qui a permis de refuser la migration ACP en mesurant, plutot qu'en lisant du code
avec optimisme :

```
0.16.2 : --disallowedTools AskUserQuestion,Read,Write,Edit   -> interception possible
0.66.0 : --disallowedTools AskUserQuestion                   -> interception perdue
```

### Ce que ca donne comme discipline

- Chaque comportement tiers dont depend un invariant a **un canari** :
  `crates/trame-agent/tests/canari_interception.rs`. Il echoue bruyamment, et un second
  test verifie qu'il **sait** echouer — un canari incapable d'echouer ne garde rien.
- Quand un test simule un protocole, son commentaire dit **quelle observation du vrai tiers**
  justifie la forme simulee. Sans cette trace, la simulation derive sans que personne le voie.
- Un test qui n'a rien verifie **le dit fort** plutot que de passer au vert : le canari
  affiche un avertissement quand l'adaptateur est absent.

## ★ Tout dispositif de mesure porte un controle negatif

**La regle** :

> **Un test, un canari ou une sonde doit prouver qu'il sait echouer, avant qu'on croie
> a son succes.** Sans ce controle, on ne mesure pas le sujet : on mesure la capacite du
> dispositif a produire une trace plausible.

C'est la meme regle que la section precedente, vue par l'autre bout. Simuler un tiers fait
croire qu'on l'a observe ; un dispositif sans controle negatif fait croire qu'on a mesure.

### Deux occurrences, un seul mode d'echec

| Dispositif | Ce qu'il semblait montrer | Ce qui se passait vraiment |
|---|---|---|
| Test de fin de tour (phase 2) | le flux emet bien `Done` a la fin d'un tour | le test **emettait lui-meme** la notification qu'il attendait — et cette notification n'existe pas |
| Hook de sonde (sonde 3) | `PostToolUse` se declenche apres un appel refuse | le hook **n'observait rien et ne refusait rien** — le tour a produit une trace credible quand meme |

Le hook de la sonde 3 etait ecrit ainsi :

```sh
/usr/bin/python3 - <<'PY'      # le heredoc EST le stdin de python
raw = sys.stdin.read()         # donc ceci lit EOF, jamais le payload du hook
```

Le programme arrivant par stdin, `sys.stdin.read()` rendait une chaine vide. Le hook n'ecrivait
aucune capture et ne rendait aucune decision — mais le tour s'est deroule normalement, avec un
`Grep` « sans resultat » qu'il etait tentant de lire comme « refuse ». La conclusion aurait ete
**l'inverse de la verite**, et rien dans la trace ne le signalait.

Les deux dispositifs ne mesuraient rien. Les deux avaient l'air de fonctionner. **C'est la
signature du mode d'echec : la sortie est plausible, donc elle ne declenche aucune verification.**

### Le controle negatif en pratique

Avant de tirer une conclusion d'un dispositif, le faire echouer volontairement :

```sh
# Sonde : le hook decide-t-il vraiment ? On l'interroge a vide, hors session.
echo '{"tool_name":"Grep","tool_input":{"pattern":"base_url"}}' | ./hook.sh   # -> doit decider
echo '{"tool_name":"Grep","tool_input":{"pattern":"autre"}}'    | ./hook.sh   # -> doit se taire
```

```rust
// Canari : un test verifie que le canari echoue quand la condition disparait.
// Un canari incapable d'echouer ne garde rien.
#[test]
fn le_canari_sait_echouer() { /* ... */ }
```

Et **une invariante de comptage**, quand le dispositif produit des traces : le nombre de
captures attendu se calcule *avant* de lire les captures. « `pre.jsonl` devrait contenir une
ligne par appel d'outil » a suffi a trouver le bug du heredoc ; sans ce calcul prealable, un
fichier vide se lit comme « il ne s'est rien passe ».

### Corollaire de redaction

Un rapport de sonde mentionne **explicitement** quel controle negatif a ete fait. Un rapport qui
n'en mentionne aucun n'est pas une mesure, c'est une impression — et il sera relu dans six mois
comme s'il etait une mesure.

## Regles de detail

- **Un `JoinHandle` d'acteur se garde** (`let (h, _join) = ...`). Le laisser tomber
  peut arreter l'acteur au milieu du test.
- **Pas de `#[tokio::test(flavor = "multi_thread")]` par defaut.** Le runtime
  monothread est deterministe ; le multi-thread ne sert que si le test *est* sur le
  parallelisme reel.
- **Un test par comportement**, nomme en francais comme une specification :
  `stale_read_sans_collision_d_ecriture` dit ce qui est garanti.
- **Le message d'assertion explique l'invariant**, pas la valeur :
  `assert!(x, "95 % du trafic doit passer sans un mot")`.
- **Tester par le handle, pas par l'etat interne.** Si un test a besoin de lire l'etat
  prive de l'acteur, il manque un message `Snapshot`.
- `unwrap()` est **autorise dans les tests** (`allow-unwrap-in-tests` dans
  `clippy.toml`) : c'est la facon la plus lisible d'echouer.
- Si un test devient instable, on ne le relance pas en boucle et on ne le marque pas
  `#[ignore]` : on trouve le `sleep` ou l'assertion d'ordre qui s'y cache.
