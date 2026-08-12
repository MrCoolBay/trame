# 0023 — `gpui-ce` pour la GUI, épinglé, avec son échappatoire

- **Statut** : Acceptée — **la sonde a passé son critère de sortie**
  ([sonde 4](../sondes/2026-08-12-gpui-ce.md))
- **Date** : 2026-08-12
- **Repose sur** l'[ADR 0022](0022-decoupage-daemon-gui.md) : c'est parce que la GUI est
  jetable que ce pari est acceptable.

## Contexte

La phase 4 est l'application desktop. Le choix se fait entre deux familles :

- **native Rust** — `gpui`, le framework de Zed, dans son fork communautaire `gpui-ce`
  publié sur crates.io. Un seul langage, un seul binaire, pas de WebView.
- **web embarqué** — Tauri v2 + Vue. Écosystème mûr, deux langages, une WebView.

`gpui` n'était pas publié sur crates.io ; `gpui-ce` l'est, en 0.3.3. C'est un framework
**pré-1.0**, qui suit l'amont Zed et hérite donc de ses ruptures.

## Décision

**La GUI est écrite avec `gpui-ce`, épinglé, importé sous le nom `gpui`.**

```toml
gpui = { package = "gpui-ce", version = "=0.3.3", default-features = false, features = [
    "font-kit",
    "runtime_shaders",   # ★ voir plus bas : c'est ce qui evite d'installer Xcode
] }
gpui_platform = { git = "https://github.com/gpui-ce/gpui-ce", default-features = false, features = [
    "font-kit",
    "runtime_shaders",
] }
```

Trois éléments de cette déclaration ne sont pas cosmétiques.

**L'alias `package = "gpui-ce"` importé sous le nom `gpui`** — c'est lui qui fait
l'échappatoire (section suivante). Tout notre code écrit `gpui::`, sans savoir quelle
implémentation le fournit.

**`=0.3.3`, épinglage exact.** Même raisonnement que l'adaptateur ACP
([ADR 0017](0017-adaptateur-acp-epingle.md)) : une dépendance dont on hérite les ruptures
d'un amont qu'on ne contrôle pas ne se met à jour que délibérément.

**`runtime_shaders` sur les deux crates.** Sans ce drapeau, le `build.rs` appelle
`xcrun metal`, qui n'existe qu'avec **Xcode complet** — pas avec les Command Line Tools. Avec
lui, les shaders sont assemblés à la compilation et compilés par Metal **au démarrage**. Le
coût d'installation passe de ~10 Go et un compte Apple à **deux drapeaux**. Mesuré : sans le
drapeau sur `gpui_platform`, le mur revient, parce que `gpui_macos` a son propre `build.rs`.

## L'échappatoire — **testée**, et c'est une version, pas une branche git

Elle était annoncée ; elle est maintenant constatée. Une échappatoire non testée est une
intention, pas une échappatoire.

**`gpui` sur crates.io est l'amont Zed publié.** Vérifié par les propriétaires du crate, pas
par ses métadonnées déclaratives :

```
crates.io/api/v1/crates/gpui/owners
  → github:zed-industries:crates-io (team), maxbrunsfeld, mikayla-maki, MrSubidubi
  → description : « Zed's GPU-accelerated UI framework »
gpui-ce : owner philocalyst — un fork communautaire, cohérent avec « CE »
```

**La bascule compile et tourne, code applicatif inchangé.** La sonde a été rebâtie contre
l'amont sans toucher une ligne de `main.rs` :

```toml
# Le fork :
gpui = { package = "gpui-ce", version = "=0.3.3", default-features = false, features = [...] }
gpui_platform = { git = "https://github.com/gpui-ce/gpui-ce", ... }

# L'amont, mesuré :
gpui = { version = "=0.2.2", default-features = false, features = ["font-kit", "runtime_shaders"] }
# et gpui_platform DISPARAIT : l'amont embarque sa couche plateforme.
```

Résultat : compilation sans erreur, fenêtre ouverte, **87 observations, offset −1342 px** —
identique au fork. La propriété drop-in tient sur la surface que nous utilisons : `div`, texte,
`rgb`, `ScrollHandle`, `Context::spawn`, `cx.notify()`.

**Coût réel de la sortie : deux lignes**, pas une — changer la déclaration de `gpui` et
supprimer celle de `gpui_platform`. Aucun `[patch]` nécessaire, puisque nous importons sous le
nom `gpui`. La forme `[patch]` reste utile si un jour une dépendance **transitive** réclame
`gpui-ce`, ce qui n'est pas le cas :

```toml
[patch.crates-io]
gpui-ce = { git = "https://github.com/zed-industries/zed", package = "gpui" }
```

Ce qui reste non mesuré, et qu'il faut dire : la bascule a été testée sur **une** version de
l'amont, à un instant donné, sur la surface d'API d'une sonde de 230 lignes. Une GUI complète
en utilisera davantage.

### Alors pourquoi le fork, et pas l'amont directement ?

La question devient légitime maintenant que l'amont est consommable en version. Trois raisons,
et la troisième est la vraie :

1. `gpui-ce` est en **0.3.3** contre 0.2.2 : il suit l'amont de plus près et publie plus
   souvent, ce qui est le point d'un fork community edition.
2. Il découpe la couche plateforme (`gpui_platform`, `gpui_macos`), ce qui rend les features
   par plateforme explicites — d'où notre `default-features = false`.
3. **Le choix est désormais réversible à coût connu**, dans les deux sens. C'est ce qui rend
   la question secondaire : si le fork décroche, on prend l'amont ; si l'amont accélère, on le
   prend aussi. Aucune des deux décisions n'est chère.

## `gpui-component` : écarté, et la raison n'est pas l'utilité

La question posée était : apporte-t-il quelque chose, ou est-ce du poids en trop ? Elle est
tranchée **avant** d'être arbitrable, par la compatibilité.

`gpui-component` 0.5.1 dépend de `gpui` **0.2.2**, un crate **distinct** de `gpui-ce` 0.3.3.
Les deux ensemble mettent **deux frameworks différents** dans le graphe : deux types `App`,
deux types `Window`, sans conversion possible. Mesuré — la compilation échoue, et elle
échouerait même en réglant les shaders.

Donc : pas utilisable avec notre épinglage. La question de l'utilité ne se pose pas, et si
elle se posait, la sonde a montré que le périmètre v0.1 tient avec `div`, du texte, une
couleur et un `ScrollHandle`.

## Licence

`gpui-ce` est sous **Apache-2.0**, compatible avec notre `MIT OR Apache-2.0`
([ADR 0013](0013-licence-open-source-mit-apache.md)) : c'est exactement la branche Apache de
notre double licence. Rien à ajouter au `NOTICE`, rien à vendoriser.

## Ce qui a été mesuré, et ce qui ne l'a pas été

La règle du projet — voir tourner avant de considérer acquis — s'applique ici, la sonde 4 la
respecte, et il faut dire les deux côtés.

**Vu tourner** : une fenêtre s'ouvre sans Xcode ; 87 `Observation` produites par le **vrai**
registre traversent un `tokio::sync::mpsc::Receiver` et alimentent la vue ; un `StaleRead`
s'affiche en jaune avec son marqueur `▲`, une écriture hors-bande en magenta ; la liste
**défile** et suit sa queue, offset mesuré à `-1342 px`. Capture d'écran relue, pas déduite.

**Le démarrage avec `runtime_shaders` est chiffré**, et il ne pose pas d'arbitrage. Mesure du
démarrage du processus au premier rendu effectif, trois lancements de chaque, cache de shaders
Metal supprimé entre chaque mesure à froid :

| | médiane | pire cas observé |
|---|---|---|
| **à froid**, cache Metal vidé | **360 ms** | 1 435 ms — le tout premier lancement |
| **à chaud** | **115 ms** | 119 ms |

Loin du seuil de deux à trois secondes au-delà duquel il aurait fallu arbitrer. **Xcode complet
reste donc optionnel**, y compris au moment de la notarisation. Le pire cas de 1 435 ms est le
premier lancement après purge : il cumule le cache de shaders vide et les pages du binaire
froides, et il ne se reproduit pas.

Le cache Metal vit dans `$TMPDIR/../C/com.apple.metal` (~450 Ko) et se reconstruit seul.

**Non mesuré** : la signature et la notarisation d'une app gpui ; le comportement de la molette
de défilement (voir le rapport de sonde, §6) ; le démarrage sur une machine sans le cache de
compilation **Rust** — mais celui-là relève du build, pas du lancement.

## Alternatives écartées

- **Tauri v2 + Vue.** C'était la porte de sortie prévue si la sonde échouait ; elle n'a pas
  échoué. Écosystème plus mûr, mais deux langages, une WebView, un pont à sérialiser pour
  faire passer chaque `Observation`, et un thème système à réimplémenter. **Reste la sortie
  si `gpui-ce` déçoit** — et l'ADR 0022 garantit que ça coûte le prix de l'interface seule.
- **Tauri + Nuxt.** Écarté explicitement : le routing et le SSR ne servent à rien sur une
  application mono-fenêtre. On paierait un framework de site pour afficher deux panneaux.
- **`gpui` de l'amont Zed directement, en dépendance git.** C'est l'échappatoire, pas le
  choix par défaut : dépendre d'une branche git d'un dépôt de 500 Mo pour un binaire de
  bureau, c'est accepter qu'un `cargo update` casse le build un mardi matin. Le fork publié
  donne une version, donc un point fixe.
- **`egui`, `iced`, `slint`.** Plus stables, et un rendu qui ne ressemble pas à macOS. Le
  produit est une app macOS assumée ([ADR 0001](0001-macos-uniquement.md)) ; l'apparence
  n'est pas un détail quand l'outil doit rester ouvert toute la journée.
- **Attendre la 1.0 de `gpui`.** Attendre une stabilité qui n'a pas de date, pour un
  composant que l'ADR 0022 rend jetable.

## Dépendance à surveiller

Au même titre que l'adaptateur ACP, et pour la même raison : **un comportement tiers dont
dépend le produit, sur une version qu'on n'a pas choisie de subir.**

- **Épinglage exact**, jamais de plage. Une montée est un acte, pas un effet de bord.
- **Ce qui casse en silence** ici n'est pas la compilation — une rupture d'API `gpui` casse le
  build, bruyamment, ce qui est le bon cas. Le risque silencieux est ailleurs :
  `runtime_shaders` déplace la compilation des shaders au **lancement**. Une régression de ce
  chemin ne se verrait pas en CI si la CI ne lance pas la fenêtre.
- **Pas de canari sur l'API, et c'est justifié** : contrairement au retrait de `Write`/`Edit`
  par l'adaptateur ACP, qui est invisible et fatal, une rupture `gpui` casse le build.
- **Mais un test de fumée sur les shaders, parce que ce risque-là est silencieux.**
  `just fumee` lance `trame-gui --smoke` : la fenêtre s'ouvre, on attend qu'une **image ait
  réellement été produite**, puis sortie 0. Une compilation verte ne prouve rien sur un chemin
  déplacé au lancement.

  Mesuré : sortie 0 avec `FUMEE_OK` en 2 679 ms cache Metal vide, 214 ms à chaud. Et
  **contrôle négatif fait** — en empêchant volontairement la vue de signaler son rendu, le test
  sort 1 avec `FUMEE_ECHEC` après 10 s. Un test de fumée qui sort 0 sans avoir rien vu ne garde
  rien.

  **Trou nommé** : il exige une session graphique Aqua. Un runner macOS en mode service, sans
  utilisateur connecté, ne peut pas joindre le WindowServer — le job `gui:macos` échouera
  plutôt que de passer à vide, ce qui est le bon sens de l'échec, mais ça reste un job qu'on ne
  peut pas exécuter aujourd'hui.
- **La GUI est exclue des jobs Linux de la CI**, et ce n'est pas un oubli : gpui n'a de couche
  plateforme sur Linux que derrière les features `x11` ou `wayland`, que nous n'activons pas
  puisque Trame ne cible que macOS. Constaté en lisant `platform.rs`, **pas compilé** — aucune
  cible Linux n'est installée sur la machine de développement. Conséquence : `trame-gui` n'est
  couverte que par un job macOS manuel.

## Ce qui invaliderait cette décision

- Une rupture d'API amont qui coûte plus cher que réécrire l'interface ailleurs. Le seuil est
  explicite : **l'interface fait moins de 1 500 lignes**, une migration qui dépasse ce coût
  n'a plus d'argument.
- Le temps de démarrage avec `runtime_shaders` s'avère visible pour l'utilisateur, **et**
  Xcode complet reste refusé. Il faudrait alors trancher entre exiger Xcode et changer de
  framework.
- `gpui-ce` cesse d'être maintenu **et** l'amont n'est pas consommable. C'est le scénario que
  l'échappatoire couvre — d'où l'intérêt de la vérifier avant d'en avoir besoin.
