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

> **Deux chemins, et ils ne coutent pas le meme nombre d'appels.**
>
> - **`auto_grow(min, max)`** — **un appel**, autonome. `rows` pilote la hauteur.
> - **`multi_line(true).rows(n)` + une hauteur sur l'element** — **trois appels**, dont le
>   troisieme sur `Input`, pas sur `InputState`.
>
> **`multi_line(true)` employe seul rend une ligne**, quel que soit le contenu.

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
| **`InputState::rows(n)`** | **builder public**, state.rs:495 |
| mais sur ce chemin, `rows(n)` | **n'agit pas sur la hauteur rendue** — voir ci-dessous |

La doc sur `InputState::multi_line` annonce « Default rows is 2 », et `plain_text()` met 1.
**C'est un defaut de valeur par defaut, pas une capacite manquante** — la doc officielle du
projet montre exactement `multi_line(true).rows(10)`.

### Pourquoi trois appels et pas deux : ou la hauteur est vraiment decidee

`element.rs` en 0.5.1 :

```rust
if state.mode.is_multi_line() {
    style.size.height = relative(1.).into();          // 100 % du parent
    if state.mode.is_auto_grow() {
        let rows = state.mode.max_rows().min(state.mode.rows());
        style.min_size.height = (rows * line_height).into();   // rows PILOTE la hauteur
    } else {
        style.min_size.height = line_height.into();            // UNE ligne, quel que soit rows()
    }
}
```

Donc sur le chemin `PlainText`, **`rows(n)` est inerte pour la mise en page** : la hauteur vaut
100 % du parent avec un plancher d'une ligne. C'est precisement pour ca que la doc officielle
appaire `.rows(10)` avec `Input::new(&state).h(px(320.))` — **la hauteur explicite est ce qui
dimensionne le champ**, et elle se pose sur l'element parce que `Input` implemente `Styled`.

`auto_grow` n'a besoin de rien de tout ca, ce qui en fait le chemin court et correct. La sonde
garde les deux, plus un **controle** — meme etat sans hauteur explicite — pour que l'inertie de
`rows(n)` se constate au lieu d'etre deduite du source.

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

## ★ Pourquoi 0.5.1, et pourquoi ce n'est pas « c'est ce qu'on a »

Le cadrage initial etait faux. **Le goulot n'est pas `gpui-component`, c'est la publication de
`gpui`** :

| | |
|---|---|
| versions de `gpui` publiees depuis 2022 | **7** |
| derniere | **0.2.2**, 2025-10-22 — huit mois |
| Zed est passe en **1.0** | avril 2026, **sans rien publier depuis** |
| ce que recommande leur README | **le git, pour tout** |

Donc `gpui-component` 0.5.1 n'a pas six mois de retard par negligence : **il a l'age du dernier
`gpui` publie.** Le chemin crates.io est un sous-produit, pas le chemin supporte en amont.

Trois arguments, dans cet ordre.

### 1. C'est une paire coherente par construction

0.5.1 a ete publiee **contre** 0.2.2. C'est un appairage **teste et reproductible** : deux
versions figees, un `Cargo.lock` qui donne le meme resultat dans six mois.

`gpui-component` `main` contre le `HEAD` de Zed change **tous les jours**, et n'importe quelle
autre combinaison — 0.5.1 contre un gpui git, ou `main` contre 0.2.2 — **n'est testee par
personne.** Le choix n'est donc pas entre « vieux » et « recent », il est entre **une paire
que quelqu'un a fait tourner** et une combinaison inedite.

### 2. La peremption est un plancher, pas un risque

0.2.2 est du **source fige**. Il ne pourrit pas : il compile aujourd'hui comme dans deux ans.
Le seul cout d'une version ancienne est de **manquer des fonctionnalites**, et tout ce dont la
phase 5 a besoin est la — champ multi-ligne, `virtual_list`, theme, markdown — **verifie a
l'ecran**, pas suppose.

C'est une distinction qui compte pour un pre-1.0 : la peur habituelle est « la dependance va
casser ». Ici elle ne peut pas casser, elle peut seulement plafonner.

### 3. Le vrai cout est le decalage, et il faut l'ecrire noir sur blanc

Leur developpement se fait **contre le git de Zed**. Donc 0.5.1 va s'eloigner de leur `main`,
et le jour ou on remonte un bug, **la reponse sera « corrige sur `main` »** — sur un `main`
qu'on ne peut pas consommer sans tirer gpui depuis le git de Zed, ce que l'ADR 0023 refuse.

**C'est acceptable pour une v1** : le champ fonctionne, le restylage fonctionne, et un
correctif qu'on ne peut pas recevoir est un correctif dont on n'a pas besoin aujourd'hui. Mais
c'est le cout reel de cette decision, et il n'est pas nul.

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

## ★ Les deux declencheurs de reexamen

Deux, pas un, et le second est le vrai signal.

### A. `gpui-base` publie avec une vraie version

Leur README distingue desormais deux niveaux : **`gpui-component`** pour des controles finis
prets a expedier, et **`gpui-base`** pour une application qui veut posseder le code de ses
composants, sa mise en page, son style et ses animations tout en reutilisant les comportements
d'interaction difficiles.

**C'est exactement notre cas** — on veut notre design system, pas ecrire un champ multi-ligne
avec IME et selection.

Etat au 2026-08-13, mesure :

| | |
|---|---|
| `gpui-base` sur crates.io | **0.1.0**, 2026-08-11, **zero dependance** — une reservation de nom |
| la vraie brique, sur `main` | version `0.5.2`, **`publish = false`** |
| modularite | **57 modules, zero gate de feature** — meme monolithe |
| coutures | `component_traits.rs` (`Selectable`, `Disableable`, `Collapsible`), `styled.rs`, `state_style.rs`, `theme_tokens.rs` |

Donc **il n'est pas consommable aujourd'hui** : git seulement, et son manifeste herite
`gpui.workspace = true`, ce qui tire gpui depuis le git de Zed. Le prendre rouvrirait
l'ADR 0023 pour un crate de deux jours que personne n'a exerce.

**Le declencheur est observable** : une version reelle sur crates.io, sans `publish = false`.
On rejoue alors les trois criteres — et il repondrait mieux que `gpui-component` a ce qu'on
veut, ce qui en fait le declencheur le plus probable des deux a moyen terme.

### B. Zed publie un `gpui` 0.3 — **le vrai signal**

Quand `gpui` 0.3 sort, `gpui-component` suit **en jours** : prouve deux fois, 0.2.0 le meme
jour et 0.2.2 en deux jours. La paire coherente se reconstitue d'elle-meme un cran plus haut,
et c'est le moment ou monter coute presque rien.

A relire alors : la note de l'[ADR 0023](0023-gpui-amont-pour-la-gui.md) sur `gpui_platform`.
Il est decoupe sur le `HEAD` de Zed, donc **une 0.3 le reintroduira probablement** — ce n'est
pas une regression, c'est le rattrapage d'une difference entre 0.2.2 et `HEAD`.

## Le filet : le scenario « bloques pour toujours sur 0.2.2 » n'existe pas

A noter, **et a ne pas prendre** :

- **`gpui-unofficial`** (iamnbutler) republie automatiquement `gpui` sur les tags de release de
  Zed. Une GitHub Action verifie **toutes les six heures**, les versions vont jusqu'a mai 2026,
  toute la famille de crates est couverte, Apache-2.0, et le projet se declare explicitement
  **non affilie**.
- **`gpui-ce`**, le fork, publie aussi — c'est l'echappatoire deja testee de l'ADR 0023, et la
  base sur laquelle la GUI a ete ecrite avant la bascule.

**Deux ponts independants vers un gpui recent publie.** Ce qui veut dire que le risque redoute
— « le crate publie ne bouge plus, on est enfermes » — **n'a pas de matiere** : si l'attente
devient couteuse, il existe deux sorties, chacune deja fonctionnelle.

On ne les prend pas : ce sont des republications tierces, et l'ADR 0023 a choisi l'amont apres
mesure. Mais leur existence est ce qui rend le point 2 ci-dessus — la peremption est un
plancher — vrai plutot que rassurant.

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
- **Le decalage devient bloquant** : un bug qui nous gene, corrige sur `main`, sans
  contournement de notre cote. C'est le cout du point 3, et il se mesure au premier cas reel.
