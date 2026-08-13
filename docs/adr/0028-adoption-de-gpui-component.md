# 0028 — On adopte `gpui-component` 0.5.1

- **Statut** : Acceptee
- **Date** : 2026-08-13
- **Mesures** : [sonde 6](../sondes/2026-08-13-gpui-component.md)
- **Prolonge** l'[ADR 0023](0023-gpui-amont-pour-la-gui.md), qui epingle gpui a l'amont Zed
  en `=0.2.2`

## Le verrou, et pourquoi il decidait de tout

La phase 5 fait de la GUI une application : saisie, fil de conversation, navigation. Une
seule primitive commandait la decision — **le champ de saisie multi-ligne**. Estime a 5-10
jours a ecrire, et surtout : c'est ce qu'un utilisateur remarque dans la premiere seconde si
c'est approximatif.

Les autres primitives ne valaient pas la dependance. Un bouton, un panneau, une liste — on
les ecrit. **58 000 lignes et l'unification de features ne s'achetent pas pour un bouton.**

Le verrou est leve : `auto_grow(5, 20)` a affiche un collage de plusieurs paragraphes, avec
retour a la ligne souple, accents et backticks conserves, et le champ a grandi. Constate a la
main, pas deduit.

## ★ La regle qui sort de cette sonde

> **Un champ multi-ligne demande deux appels, jamais un seul.** Soit
> `auto_grow(min, max)`, soit `multi_line(true).rows(n)`. **`multi_line(true)` employe seul
> rend une ligne**, quel que soit le contenu.

C'est une regle du projet et pas une anecdote, parce que l'appel evident est le mauvais et que
son echec est silencieux : le champ accepte les `\n`, affiche une barre de defilement, et ne
montre qu'une ligne.

Le mecanisme :

| | |
|---|---|
| `InputMode::plain_text()` initialise | `rows: 1` |
| `multi_line(bool)` modifie | **le booleen seulement** — jamais `rows` |
| `is_multi_line()` rend alors | `true` |
| le layout calcule | `max_rows().min(rows())` → **1** |
| **`InputState::rows(n)`** | **builder public**, state.rs:495 — c'est lui qui corrige |

La doc sur `InputState::multi_line` annonce « Default rows is 2 », et `plain_text()` met 1.
**C'est un defaut de valeur par defaut, pas une capacite manquante** — la doc officielle du
projet montre exactement `multi_line(true).rows(10)`.

### ⚠️ Ce que cet ADR affirmait a tort dans sa premiere version

La premiere redaction disait que `multi_line` etait **irreparable depuis l'exterieur du
crate**, en s'appuyant sur `InputMode::set_rows` qui est `pub(super)`. **C'etait faux** :
`InputState::rows(n)` est public, dans le fichier que je venais de lire.

L'erreur vient de la methode, pas de la bibliotheque : la source avait ete interrogee par une
liste de noms **devines**, et la documentation publique du projet n'avait pas ete consultee.
Une issue en amont batie sur cette conclusion a ete redigee puis **retiree avant publication**.

C'est le **dixieme cas** du motif de `AGENTS.md`, et d'une famille nouvelle : conclure a
l'absence de ce qu'on n'a pas cherche. La regle qui en sort : **pour conclure a l'absence
d'une capacite, enumerer la surface** — et **confronter toute conclusion sur une API tierce a
sa documentation publique**.

## Ce qu'on accepte les yeux ouverts

### 1. Non modulaire — et l'echappatoire n'est pas le fork

61 modules, **un seul** derriere une feature (`webview`). On ne peut pas ne prendre que
`input` et `virtual_list` : tout vient en bloc. `gpui_component::init(cx)` est obligatoire et
initialise en cascade le theme, `dock`, `sheet`, `select`, `input`, `list`.

Donc « forker deux composants » n'existe pas. Ce qui existe, c'est forker 58 000 lignes —
exactement le portage continu qu'on venait d'ecarter pour les crates de Zed. **Une dependance
maintenue vaut mieux qu'un fork silencieux.**

**L'echappatoire est ailleurs, et elle est meilleure**, verifiee sur ecran :

- `Button`, `Input`, `Label` implementent **`gpui::Styled`**
- leur `render` finit par `.refine_style(&self.style)` — **notre style raffine par-dessus
  leur preset**, il ne se fait pas ecraser
- tous sont `IntoElement` + `RenderOnce`, donc `div().child(Button::new(…))` fonctionne dans
  un arbre gpui nu
- `ButtonVariant::Custom(ButtonCustomVariant)` existe a cote des six presets

> **On habille la bibliotheque au lieu d'etre habille par elle.** Le bouton violet de la
> sonde le montre sur piece : c'est leur `Button`, restyle par nos methodes tailwind-like.

Cette echappatoire ne coute **aucun code a maintenir**, contrairement a un fork.

### 2. Features unifiees — 20 en trop, et ce que ca ne casse pas

`gpui-component` demande `gpui = "0.2.2"` **sans specifier de features**, donc les defauts :
`font-kit`, `wayland`, `x11`, `windows-manifest`. **Cargo unit les features, il ne les
soustrait pas** — on ne peut pas refuser ce qu'une dependance transitive reclame.

Consequence sur une application macOS : `wayland`, `x11`, `blade-graphics`, `cosmic-text`,
`xkbcommon`, `x11rb`, `ashpd` et la pile de clients Wayland. Notre `trame-gui` demandait deux
features de gpui ; le graphe en porte maintenant une vingtaine. C'est precisement ce que
`default-features = false` avait ete pose pour eviter (ADR 0023).

**Ce que ca coute** : du temps de compilation — 290 s a froid pour le crate — et du poids de
binaire.

**Ce que ca ne coute pas, et c'est le point** :

- **`runtime_shaders` survit.** Il est demande directement par `trame-gui`, l'unification ne
  le touche pas.
- **`macos-blade` n'est pas allume** — zero occurrence dans le graphe. C'est le seul flag qui
  aurait deplace le rendu macOS de Metal vers blade. Le chemin de rendu est **inchange**.

> **La preuve n'est pas la liste de features, c'est la fenetre qui s'ouvre avec le bouton
> violet.** Metal et les shaders compiles au lancement fonctionnent avec `gpui-component` dans
> le graphe, constate a l'ecran.

### 3. Le volume disque, parce que ca va se reproduire

Le build complet a porte `target/` a **21 Go** et rempli le volume a 99 % en cours de sonde.
Le diagnostic et le correctif sont dans `[profile.dev]` de `Cargo.toml` : `line-tables-only`
plus `debug = false` sur les dependances font tomber `target/debug/deps` de 17 Go a 1,7 Go.

`just clean` garde `target/release`, ou vit le binaire de sonde. `just clean-deep` vide tout,
cache cargo compris.

### 4. Dette a surveiller

- **`Input` / `Textarea` / `Editor` viennent d'etre separes** sur `main` (PR #2691, le jour de
  la sonde). **L'API de 0.5.1 n'est pas la forme finale** : une montee de version demandera
  une adaptation cote appelant. A relire quand une 0.6 sortira.
- **La version publiee a six mois de retard sur `main`.** Derniere release le 2026-02-05, et
  ils committent tous les jours. On reste sur crates.io **conformement a l'ADR 0023**, qui a
  choisi une version contre une branche git — prendre `main` tirerait gpui depuis le git de
  Zed. **A revoir s'ils coupent une release**, pas avant.

## Pourquoi 0.5.1 et pas `main`

`main` porte le decoupage `Input`/`Textarea`/`Editor` et les corrections du jour. Il depend
aussi de `gpui = { version = "0.2.2", git = "https://github.com/zed-industries/zed" }`.

**On ne rouvre pas une decision pour de la fraicheur.** L'ADR 0023 a choisi une version
publiee contre une branche, apres mesure, et rien dans cette sonde ne l'invalide : le champ
multi-ligne fonctionne en 0.5.1.

## Consequences

- `gpui-component` passe de **dev-dependency de sonde** a dependance normale de `trame-gui`
  au moment du cablage de la phase 5. Tant que ca reste une dev-dependency, le resolver v3
  garde le binaire livre libre de l'unification — **l'adoption supprime ce confinement**, et
  il faut le savoir plutot que le decouvrir.
- Le rendu markdown est disponible (`src/text/`), **ce qui change la forme du modele de fil**
  de la phase 5.2 : un fil qui rend du markdown ne se structure pas comme un fil de texte
  brut.
- `virtual_list` remplace notre defilement maison pour le fil et le flux d'observations.
- Le theme de la bibliotheque devient la source des tokens, ce qui **absorbe le chantier
  « tokens de theme partages dans trame-view »** — a rearbitrer : la TUI n'a pas de theme
  gpui, donc `trame-view` garde probablement ses propres tokens et la GUI les mappe.

## Alternatives ecartees

- **Ecrire notre champ multi-ligne.** 5-10 jours, et l'IME, la selection a la souris, le
  placement du curseur au clic et le retour a la ligne souple sont exactement ce qui coute
  ces jours-la — et ce qu'un utilisateur remarque si c'est approximatif. Ecrire un champ
  mediocre coute une semaine et se voit.
- **Porter les crates `ui` et `editor` de Zed.** Sous licence permissive, et le niveau de
  finition est atteignable. Mais ce sont des fichiers couples a leur theme, leurs icones et
  leurs traits internes : c'est du **portage continu sans amont qui le maintienne**. Une
  dependance maintenue vaut mieux qu'un fork silencieux.
- **`main` plutot que 0.5.1.** Voir ci-dessus : ca rouvre l'ADR 0023 pour de la fraicheur.
- **Ne prendre que quelques modules en les recopiant.** Impossible proprement : la
  non-modularite du critere 2 fait que `input` tire le theme, `global_state` et le reste.

## Ce qui invaliderait cette decision

- **Le projet s'arrete.** Observable : plus de commits pendant plusieurs mois, ou une archive
  du depot. La reponse serait alors le fork complet, avec son cout assume — pas une surprise.
- **Une montee de version casse l'API du champ au point que l'adaptation coute plus que
  l'ecrire.** Le decoupage `Input`/`Textarea`/`Editor` est le premier candidat, et c'est
  pourquoi il est en dette ci-dessus.
- **Les features unifiees finissent par changer le chemin de rendu.** Aujourd'hui `macos-blade`
  est eteint. S'il s'allumait par une dependance transitive, la sonde de fumee des shaders
  (`just smoke`) est ce qui le dirait.
- **Un besoin de restylage que `.refine_style()` ne couvre pas.** Ce serait la fin de
  l'echappatoire du critere 3, et donc de l'argument central de cet ADR.
