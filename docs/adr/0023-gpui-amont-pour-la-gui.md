# 0023 — `gpui` de l'amont Zed pour la GUI, épinglé, avec `gpui-ce` en échappatoire

- **Statut** : Acceptée
- **Date** : 2026-08-12
- **Repose sur** l'[ADR 0022](0022-decoupage-daemon-gui.md) : c'est parce que la GUI est
  jetable que ce pari est acceptable.

> **Cette décision a été inversée le jour même de sa rédaction.** La première version retenait
> le fork `gpui-ce`, parce que l'amont n'était consommable que par une branche git et que sa
> parité n'était pas établie. Les deux prémisses étaient fausses, et ce sont **nos propres
> mesures** qui l'ont montré. L'historique est conservé plus bas plutôt que réécrit : une
> décision renversée par la mesure est le fonctionnement normal, pas un accident à masquer.

## Contexte

La phase 4 est l'application desktop. Le choix se fait entre deux familles :

- **native Rust** — `gpui`, le framework de Zed. Un seul langage, un seul binaire, pas de
  WebView.
- **web embarqué** — Tauri v2 + Vue. Écosystème mûr, deux langages, une WebView.

La sonde a tranché la première famille ([sonde 4](../sondes/2026-08-12-gpui-ce.md)) : une
fenêtre s'ouvre sans Xcode, un `tokio::sync::mpsc::Receiver` alimente une vue **sans
passerelle ni sérialisation**, et la liste défile. C'est ce dernier point qui décide : le même
`Receiver<Observation>` que consomme la TUI alimente la GUI.

Restait à choisir **quelle** distribution de `gpui`.

## Décision

**La GUI est écrite avec `gpui` de l'amont Zed, épinglé exactement.**

```toml
gpui = { version = "=0.2.2", default-features = false, features = [
    "font-kit",
    "runtime_shaders",   # ★ voir plus bas : c'est ce qui evite d'installer Xcode
] }
```

Une seule ligne. Pas de `gpui_platform` : **l'amont embarque sa couche plateforme**.

Trois éléments de cette déclaration ne sont pas cosmétiques.

**`=0.2.2`, épinglage exact.** Même raisonnement que l'adaptateur ACP
([ADR 0017](0017-adaptateur-acp-epingle.md)) : une dépendance pré-1.0 dont on hérite les
ruptures ne monte que délibérément.

**`default-features = false`.** Les defaults tirent `wayland` et `x11`, inutiles sur une cible
macOS uniquement ([ADR 0001](0001-macos-uniquement.md)).

**`runtime_shaders`.** Sans ce drapeau, le `build.rs` appelle `xcrun metal`, qui n'existe
qu'avec **Xcode complet** — pas avec les Command Line Tools. Avec lui, les shaders sont
assemblés à la compilation et compilés par Metal **au démarrage**. Le coût d'installation
passe de ~10 Go et un compte Apple à **un drapeau**.

## Pourquoi l'amont et pas le fork — trois mesures, pas trois opinions

La première version de cet ADR retenait `gpui-ce`, un fork communautaire publié en 0.3.3, pour
deux raisons qui semblaient solides : l'amont n'était atteignable que par une branche git, et
la propriété drop-in du fork n'était qu'annoncée. Vérifier ces deux points les a démolis.

**1. `gpui` sur crates.io *est* l'amont publié.** Établi par les **propriétaires du crate**, et
non par le champ `repository`, qui est déclaratif et que n'importe qui peut renseigner :

```
crates.io/api/v1/crates/gpui/owners
  → github:zed-industries:crates-io (team), maxbrunsfeld, mikayla-maki, MrSubidubi
  → description : « Zed's GPU-accelerated UI framework »

crates.io/api/v1/crates/gpui-ce/owners
  → philocalyst — un fork communautaire, cohérent avec « CE »
```

Donc **une version, pas une branche git.** L'argument qui portait le fork — « dépendre d'une
branche d'un dépôt de 500 Mo, c'est accepter qu'un `cargo update` casse le build un mardi
matin » — n'a plus d'objet. Un `=0.2.2` est un point fixe.

**2. La parité d'API est constatée.** La sonde a été rebâtie contre l'amont **sans toucher une
ligne de `main.rs`** : compilation sans erreur, fenêtre ouverte, 87 observations, offset
−1342 px. Identique au fork, sur la surface que nous utilisons — `div`, texte, `rgb`,
`ScrollHandle`, `Context::spawn`, `cx.notify()`.

**3. L'amont embarque sa couche plateforme.** Le fork la découpe en `gpui_platform` /
`gpui_macos`, ce que la première version de cet ADR comptait comme un avantage — features par
plateforme explicites. En pratique c'est une ligne de plus, un `build.rs` de plus à connaître,
et **un piège** : activer `runtime_shaders` sur `gpui` seul ne suffisait pas, le mur Xcode se
déplaçait sur `gpui_macos`. Une dépendance de moins est une surface de moins.

### Le raisonnement, en une phrase

À surface d'API équivalente, on prend la dépendance la moins risquée : l'amont, maintenu par
l'équipe qui écrit le framework, plutôt qu'un fork de quelques personnes qui le suit.

Et **on le fait maintenant, tant que l'interface fait moins de 1 500 lignes.** Le coût de la
bascule ne fera que croître ; à ce volume il est nul, ce qui est la seule fenêtre où ce genre
de décision se prend sans négocier.

## `gpui-ce` reste l'échappatoire — dans l'autre sens

Le mécanisme est le même, et il joue **dans les deux sens** : notre code écrit `gpui::` sans
savoir qui le fournit. Revenir au fork est un changement de deux lignes :

```toml
gpui = { package = "gpui-ce", version = "=0.3.3", default-features = false, features = ["font-kit", "runtime_shaders"] }
gpui_platform = { git = "https://github.com/gpui-ce/gpui-ce", default-features = false, features = ["font-kit", "runtime_shaders"] }
```

**Et cette échappatoire-là est déjà testée** : c'est la base sur laquelle la GUI a été écrite
avant la bascule. Elle est disponible le jour où l'amont casse, refuse quelque chose, ou tarde
à publier — le fork suit l'amont de plus près et publie plus souvent, ce qui est le point d'une
*community edition*.

Si un jour une dépendance **transitive** réclame `gpui-ce`, la forme est un `[patch]` à la
racine du workspace :

```toml
[patch.crates-io]
gpui-ce = { git = "https://github.com/gpui-ce/gpui-ce" }
```

### La réserve, conservée telle quelle

**La parité a été testée sur une version, à un instant, sur la surface d'API d'une sonde de
230 lignes.** Une GUI complète en utilisera davantage. Ce qui est établi est que la bascule est
possible et bon marché, pas qu'elle le restera sans y regarder.

## `gpui-component` : écarté, et la raison n'est pas l'utilité

`gpui-component` 0.5.1 dépend de `gpui` **0.2.2** — la même version que la nôtre depuis la
bascule, ce qui rouvre la question de compatibilité. Mais l'arbitrage ne change pas : le
périmètre v0.1 tient avec `div`, du texte, `rgb` et un `ScrollHandle` — 70 lignes de thème et
280 de vue. Une bibliothèque de composants ajouterait un **second amont à suivre** pour des
boutons dont on n'a pas besoin.

À rouvrir si le périmètre s'élargit — champs de saisie, menus, listes virtualisées — pas
avant. Note pour ce jour-là : sous le fork, elle était **inutilisable** (deux crates `gpui`
distincts dans le graphe, mesuré) ; sous l'amont, elle est simplement inutile.

## Licence

`gpui` est sous **Apache-2.0**, compatible avec notre `MIT OR Apache-2.0`
([ADR 0013](0013-licence-open-source-mit-apache.md)) : c'est exactement la branche Apache de
notre double licence. Rien à ajouter au `NOTICE`, rien à vendoriser. Idem pour `gpui-ce`, ce qui
rend l'échappatoire neutre côté licence.

## Ce qui a été mesuré, et ce qui ne l'a pas été

La règle du projet — voir tourner avant de considérer acquis — s'applique ici, et il faut dire
les deux côtés.

**Vu tourner, sur l'amont** : la fenêtre s'ouvre sans Xcode ; les observations du **vrai**
registre traversent un `Receiver` tokio et alimentent la vue ; un `StaleRead` s'affiche en jaune
avec son marqueur `▲`, une écriture hors-bande en magenta ; la liste défile et suit sa queue.
Captures d'écran relues, pas déduites.

**Le démarrage avec `runtime_shaders` est chiffré**, et il ne pose pas d'arbitrage. Mesure du
démarrage du processus au premier rendu effectif, trois lancements de chaque, cache de shaders
Metal supprimé entre chaque mesure à froid :

| | médiane | pire cas observé |
|---|---|---|
| **à froid**, cache Metal vidé | **360 ms** | 1 435 ms — le tout premier lancement |
| **à chaud** | **115 ms** | 119 ms |

Loin du seuil de deux à trois secondes au-delà duquel il aurait fallu arbitrer. **Xcode complet
reste donc optionnel**, y compris au moment de la notarisation. Le pire cas cumule cache de
shaders vide et pages du binaire froides, et ne se reproduit pas.

Le cache Metal vit dans `$TMPDIR/../C/com.apple.metal` (~450 Ko) et se reconstruit seul.

**Non mesuré** : la signature et la notarisation d'une app gpui ; le comportement de la molette
de défilement — seul le contrôle **programmatique** de l'offset est prouvé, et c'est ce dont un
flux vivant a besoin, mais pas de quoi conclure pour une application.

## Alternatives écartées

- **Le fork `gpui-ce`.** C'était la décision d'il y a quelques heures, renversée ci-dessus. Elle
  reste l'échappatoire, testée.
- **Tauri v2 + Vue.** C'était la porte de sortie prévue si la sonde échouait ; elle n'a pas
  échoué. Écosystème plus mûr, mais deux langages, une WebView, un pont à sérialiser pour faire
  passer chaque `Observation`, et un thème système à réimplémenter. **Reste la sortie si `gpui`
  déçoit** — et l'ADR 0022 garantit que ça coûte le prix de l'interface seule.
- **Tauri + Nuxt.** Écarté explicitement : le routing et le SSR ne servent à rien sur une
  application mono-fenêtre. On paierait un framework de site pour afficher deux panneaux.
- **`gpui` en dépendance git sur `main`.** Utile pour tester un correctif amont avant sa
  publication, pas comme choix par défaut : une branche n'est pas un point fixe.
- **`egui`, `iced`, `slint`.** Plus stables, et un rendu qui ne ressemble pas à macOS. Le
  produit est une app macOS assumée ; l'apparence n'est pas un détail quand l'outil doit rester
  ouvert toute la journée.
- **Attendre la 1.0 de `gpui`.** Attendre une stabilité qui n'a pas de date, pour un composant
  que l'ADR 0022 rend jetable.

## Dépendance à surveiller

Au même titre que l'adaptateur ACP, et pour la même raison : **un comportement tiers dont dépend
le produit, sur une version qu'on n'a pas choisie de subir.**

- **Épinglage exact**, jamais de plage. Une montée est un acte, pas un effet de bord.
- **Pas de canari sur l'API, et c'est justifié** : contrairement au retrait de `Write`/`Edit`
  par l'adaptateur ACP, qui est invisible et fatal, une rupture `gpui` casse le build.
- **Mais un test de fumée sur les shaders, parce que ce risque-là est silencieux.**
  `just smoke` lance `trame-gui --smoke` : la fenêtre s'ouvre, on attend qu'une **image ait
  réellement été produite**, puis sortie 0. Une compilation verte ne prouve rien sur un chemin
  déplacé au lancement.

  Mesuré **sur l'amont** : sortie 0 avec `SMOKE_OK`. Et **contrôle négatif refait sur cette
  base** — en empêchant volontairement la vue de signaler son rendu, le test sort 1 avec
  `FUMEE_ECHEC` après 10 s. Un contrôle négatif fait sur l'ancienne base ne dit rien de la
  nouvelle.

  **Le trou nommé est fermé, par la mesure.** Il exigeait une session graphique Aqua, et on ne
  savait pas si un runner macOS GitHub en avait une. Vérifié le 2026-08-13 en lançant le job une
  fois :

  ```
  utilisateur      : runner
  session launchd  : Aqua          ← la réponse
  console          : runner
  xcode-select -p  : /Applications/Xcode_26.6.app/Contents/Developer
  xcrun -f metal   : .../Metal.xctoolchain/usr/bin/metal
  ```

  `just smoke` rend `SMOKE_OK: an image was produced`. Le job est donc passé sur le chemin
  critique de la CI. Un runner macOS en **mode service**, sans utilisateur connecté, resterait
  incapable de joindre le WindowServer — mais ce n'est pas la configuration de GitHub.
- **`metal` est présent en CI, et ça précise le périmètre de `runtime_shaders` sans le remettre
  en cause.** Le runner a Xcode 26.6 complet, donc le drapeau n'y sert à rien : la CI aurait pu
  compiler les shaders au build. **Le drapeau sert sur une machine de développement sans Xcode
  complet**, qui est le cas qui motive cette décision — et c'est le cas de la machine sur
  laquelle Trame s'écrit. Une mesure qui déplace la frontière d'un choix n'est pas une mesure
  qui l'annule ; celle-ci dit *où* le drapeau est utile, pas qu'il est inutile.
- **La GUI est exclue des jobs Linux de la CI**, et ce n'est pas un oubli : gpui n'a de couche
  plateforme sur Linux que derrière les features `x11` ou `wayland`, que nous n'activons pas
  puisque Trame ne cible que macOS. Constaté en lisant `platform.rs`, **pas compilé** — aucune
  cible Linux n'est installée sur la machine de développement. Conséquence : `trame-gui` n'est
  couverte que par le job macOS.

## Ce qui invaliderait cette décision

- Une rupture d'API amont qui coûte plus cher que réécrire l'interface ailleurs. Le seuil est
  explicite : **l'interface fait moins de 1 500 lignes**, une migration qui dépasse ce coût n'a
  plus d'argument.
- L'amont cesse de publier sur crates.io, ou tarde au point que le fork prend une avance
  fonctionnelle utile. C'est le scénario que l'échappatoire couvre, et elle est testée.
- Le temps de démarrage avec `runtime_shaders` s'avère visible pour l'utilisateur, **et** Xcode
  complet reste refusé. Il faudrait alors trancher entre exiger Xcode et changer de framework.
