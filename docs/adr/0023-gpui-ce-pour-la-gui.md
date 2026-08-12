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

## L'échappatoire, et pourquoi elle rend le pari acceptable

`gpui-ce` est un **drop-in** de `gpui`. Comme nous l'importons sous le nom `gpui`, en sortir
est un **changement d'une ligne** dans un seul `Cargo.toml` :

```toml
# On quitte le fork pour l'amont, sans toucher une ligne de code applicatif :
gpui = { git = "https://github.com/zed-industries/zed", branch = "main" }
```

Si un jour une dépendance **transitive** réclame `gpui-ce` — ce n'est pas le cas aujourd'hui —
la forme est un `[patch]` à la racine du workspace :

```toml
[patch.crates-io]
gpui-ce = { git = "https://github.com/zed-industries/zed", package = "gpui", branch = "main" }
```

**Cette échappatoire n'est pas mesurée.** Elle repose sur la propriété annoncée par le projet
— fork drop-in — et sur le fait que notre code n'utilise que la surface publique commune. La
vérifier demanderait de construire l'amont depuis git, ce que la sonde n'a pas fait. À
vérifier avant d'en dépendre en urgence, pas le jour de l'urgence.

Un fait relevé au passage, qui la rend plus crédible : un crate `gpui` **0.2.2** existe
maintenant sur crates.io, dont les métadonnées déclarent
`repository = https://github.com/zed-industries/zed`. Si c'est bien l'amont publié, la sortie
devient une version au lieu d'une branche git. Non vérifié — les métadonnées d'un crate sont
déclaratives.

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

**Non mesuré** : l'échappatoire vers l'amont ; le comportement sur une machine sans le cache
de compilation ; le temps de démarrage réel avec compilation des shaders au lancement
(`runtime_shaders` déplace ce coût du build vers le lancement — à chiffrer avant de le
considérer négligeable) ; la signature et la notarisation d'une app gpui.

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
- **Pas de canari automatisé pour l'instant, et c'est un manque assumé** : contrairement au
  retrait de `Write`/`Edit` par l'adaptateur ACP, qui est invisible et fatal, une rupture
  `gpui` est visible. Si la GUI devient testable sans écran, un canari de rendu deviendra
  possible — et souhaitable.

## Ce qui invaliderait cette décision

- Une rupture d'API amont qui coûte plus cher que réécrire l'interface ailleurs. Le seuil est
  explicite : **l'interface fait moins de 1 500 lignes**, une migration qui dépasse ce coût
  n'a plus d'argument.
- Le temps de démarrage avec `runtime_shaders` s'avère visible pour l'utilisateur, **et**
  Xcode complet reste refusé. Il faudrait alors trancher entre exiger Xcode et changer de
  framework.
- `gpui-ce` cesse d'être maintenu **et** l'amont n'est pas consommable. C'est le scénario que
  l'échappatoire couvre — d'où l'intérêt de la vérifier avant d'en avoir besoin.
