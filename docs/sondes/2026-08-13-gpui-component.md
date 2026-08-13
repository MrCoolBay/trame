# Sonde 6 — `gpui-component` 0.5.1 comme dependance de fond

- **Date** : 2026-08-13
- **Question** : peut-on batir la GUI de la phase 5 sur cette bibliotheque, plutot que
  d'ecrire les primitives ou de porter celles de Zed ?
- **Etat** : **concluante, adoption decidee** ([ADR 0028](../adr/0028-adoption-de-gpui-component.md)).
  Deux gestes restent a verifier a la main : selection a la souris sur plusieurs lignes, et
  curseur au clic dans un bloc.
- **Verdict** : `auto_grow(5, 20)` affiche un collage de plusieurs paragraphes, retour a la
  ligne souple, accents et backticks conserves, et le champ grandit. `multi_line(true)`
  **employe seul**, meme collage, rend une seule ligne — parce que `plain_text()` initialise
  `rows: 1`. Le correctif est `rows(n)`, **un builder public** que la premiere version de ce
  rapport avait manque : voir le dixieme cas dans `AGENTS.md`. La sonde garde les **trois**
  champs empiles, c'est une demonstration reproductible plutot qu'une description.

## Pourquoi une evaluation avant une sonde

58 000 lignes sur 185 fichiers, ce n'est pas une bibliotheque de composants, c'est une
dependance de fond. « Est-ce que ca marche » n'est donc pas le critere principal : un
composant parfait mais inadaptable ou abandonne est un piege, pas une economie.

Trois criteres ont ete etablis **avant** d'executer quoi que ce soit. Si l'un des trois
avait ete redhibitoire, la sonde d'execution n'aurait servi a rien.

## Critere 1 — maintenu. Bon, et la premisse de depart etait fausse

| | |
|---|---|
| dernier push | 2026-08-13, le jour de la sonde |
| activite | ~18 commits/semaine, **52 semaines consecutives sans trou** |
| contributeurs | 100+ ; `huacnlee` 1 229 commits, `madcodelife` 390, longue traine reelle |
| etoiles / forks | 12 695 / 764 |
| issues | 98 ouvertes, des fermetures **le jour meme** |
| licence | `LICENSE-APACHE` verifie dans le depot. Le `NOASSERTION` de GitHub est un artefact de son detecteur, pas un probleme de licence |

**Le test qui comptait — comment ont-ils suivi les montees de gpui ?**

| gpui publie | gpui-component |
|---|---|
| 0.2.0 — 2025-10-09 | 0.2.0, **le meme jour** |
| 0.2.1 — 2025-10-14 | |
| 0.2.2 — 2025-10-22 | 0.3.0 — 2025-10-24, **deux jours** |

Des jours, pas des mois. Et le fait qui retourne la question : **gpui n'a rien publie
depuis 0.2.2, il y a dix mois.** Les ruptures frequentes du pre-1.0 sont celles du gpui
**interne** de Zed, pas du crate publie sur lequel nous sommes epingles. Le pire cas
redoute — nous bloques sur une vieille gpui pendant qu'ils rattrapent — n'a pas de matiere :
il n'y a rien a rattraper.

**La reserve, reelle.** Derniere version publiee le 2026-02-05, six mois, alors qu'ils
committent tous les jours. Ils developpent sur `main` sans couper de release, et `main`
depend de gpui **par git sur le depot de Zed**. Prendre `main` rouvrirait donc l'ADR 0023,
qui a choisi une version contre une branche. **On ne rouvre pas une decision pour de la
fraicheur** : la sonde porte sur 0.5.1, la version crates.io.

## Critere 2 — maintenable en interne. **Negatif**, et c'est assume

**61 modules, un seul derriere une feature** (`webview`). Les features existantes —
`decimal`, `tree-sitter-languages`, `inspector` — gerent des dependances optionnelles, pas
les composants. **On ne peut pas ne prendre que `input` et `virtual_list` : tout vient en
bloc.**

`gpui_component::init(cx)` est obligatoire et initialise en cascade le theme,
`global_state`, `dock`, `sheet`, `select`, `input`, `list`… Le theme est un `Global` gpui.

Nuance en leur faveur : `Root` n'est requis que pour les overlays — `Sheet`, `Dialog`,
`Notification` — pas pour rendre un composant. Et sur `main` un decoupage en workspace a
commence (`crates/base`, `crates/ui`, `crates/macros`, `crates/assets`), non publie.

**Donc l'echappatoire « forker deux composants » n'existe pas sur 0.5.1.** Ce qui existe,
c'est forker les 58 000 lignes — exactement le portage continu qu'on venait d'ecarter pour
Zed.

Ce critere est negatif et la decision passe quand meme, pour une raison qui merite d'etre
ecrite : **l'echappatoire par fork n'etait pas la bonne strategie.** Ce qui la remplace est
le critere 3.

## Critere 3 — adaptable. **Positif, et c'est ce qui decide**

```rust
impl Styled for Button { fn style(&mut self) -> &mut StyleRefinement { &mut self.style } }
impl Styled for Input  { … }
impl Styled for Label  { … }
```

Les trois exposent **le trait `Styled` de gpui**, et leur `render` finit par
`.refine_style(&self.style)` : **le style de l'appelant raffine par-dessus leur preset**, il
ne se fait pas ecraser.

Composition : `#[derive(IntoElement)]` + `impl RenderOnce` sur chacun, donc
`div().child(Button::new(…))` fonctionne dans un arbre gpui nu, sans imposer leur modele au
reste.

Variantes : `ButtonVariant::Custom(ButtonCustomVariant)` existe a cote des six presets. Ce
n'est pas un ensemble ferme.

> **On habille la bibliotheque au lieu d'etre habille par elle.** C'est une echappatoire
> differente du fork, et meilleure : elle ne coute aucun code a maintenir.

**Ce point reste a confirmer sur ecran** : tout ci-dessus vient de la lecture de la source.
La sonde met deux boutons cote a cote, l'un nu et l'autre avec nos methodes enchainees,
precisement pour que l'ecart se voie ou ne se voie pas.

## Ce que la sonde a mesure

### Le build passe, et il coute

| | |
|---|---|
| compilation du crate | **290 s** en debug, **314 s** en release, a froid |
| binaire de sonde | 78 Mo en debug, **13 Mo** en release |
| disque | `target/` est passe a **25 Go** et a rempli le volume en cours de sonde |

Le crate compile sans un patch contre notre gpui 0.2.2 epingle. **Aucune adaptation n'a ete
necessaire.**

### ★ Le cout cache, et il n'etait pas dans l'inventaire

`gpui-component` demande `gpui = "0.2.2"` **sans specifier de features**, donc les defauts
de gpui : `font-kit`, `wayland`, `x11`, `windows-manifest`. **Cargo unit les features, il ne
les soustrait pas.** L'adopter allume donc, sur une application macOS :

```
wayland  x11  blade-graphics  blade-macros  blade-util  cosmic-text  xkbcommon
x11rb  x11-clipboard  xim  wayland-client  wayland-protocols  ashpd  …
```

C'est-a-dire le backend graphique Linux et la pile de clients X11/Wayland — **exactement ce
que `default-features = false` avait ete pose pour eviter** (ADR 0023).

Mesure a l'appui : notre `trame-gui` demande deux features de gpui, `font-kit` et
`runtime_shaders`. Avec `gpui-component` dans le graphe, la liste passe a une vingtaine.

**Contenu pour la duree de la sonde** : `gpui-component` est une **dev-dependency**. Sous
le resolver v3, `cargo build -p trame-gui` reste libre de l'unification — verifie, le
binaire livre voit toujours `font-kit` + `runtime_shaders` seulement. **L'adoption
supprimerait ce confinement**, et il n'y a pas de moyen de la refuser cote appelant : on ne
peut pas retirer une feature qu'une dependance transitive demande.

### Le demarrage

`gpui_component::init(cx)` est chronometre separement du premier frame.

| profil | `init(cx)` | premier frame |
|---|---|---|
| debug | 4,5 ms | 1 160 ms |
| release, a froid | 1,3 ms | 749 ms |
| release, a chaud | 0,4 ms | 97 ms |
| release, a chaud | 0,3 ms | 93 ms |

**`init(cx)` est negligeable** — moins de 1,5 ms. Le cout de demarrage n'est pas la.

Contre la reference de 360 ms a froid et 115 ms a chaud : **a chaud, rien ne se degrade**
(93-97 ms contre 115). **A froid, c'est environ le double** (749 contre 360), ce qui est
coherent avec un binaire plus gros et davantage de frameworks lies, meme non utilises.

**Reserve sur cette comparaison** : elle n'est pas parfaitement controlee. La sonde rend
1 000 lignes et un champ de saisie, la reference rendait un arbre plus simple, et le profil
de build de la reference n'est pas documente. Le chiffre solide est `init(cx)`, mesure
directement ; le reste est une indication.

### La cohabitation avec notre canal

Notre `Receiver<Observation>` est attendu depuis l'executor de gpui, exactement comme
`trame-gui` le fait, avec un producteur en tache de fond qui emet 200 observations. Le
compteur monte a l'ecran. **Rien ne se casse** — c'est le point technique central (ADR 0022)
et il tient.

## Ce qui reste a faire, et par qui

**La manipulation a la main.** `just probe-component` ouvre la fenetre. L'ordre de
manipulation est dans la recette et dans la banniere du binaire :

1. **le champ multi-ligne** — dix lignes, selection a la souris, clic pour placer le
   curseur, ligne longue qui doit passer a la ligne, `cmd-C` / `cmd-V`, caractere accentue
   ou saisie IME. **C'est le point qui decide.**
2. la liste de 1 000 lignes, a la molette
3. les deux boutons : si l'ecart ne se voit pas, `.refine_style()` ne fait pas ce que la
   lecture annoncait
4. le compteur `observed`, qui bouge tout seul

**Regle de sortie** : si le champ multi-ligne n'est pas correct a l'usage, on le dit et on
reparle du perimetre. Pas d'acharnement.

## Trois erreurs de methode commises pendant cette sonde

Elles valent d'etre notees parce que ce sont **trois instances de la meme famille** que le
motif de `AGENTS.md`, commises dans la meme heure.

1. **Une boucle d'attente qui n'attendait pas.** `read -t 1 </dev/null` rend la main
   immediatement sur EOF, donc la boucle tournait ses 25 iterations en un instant et lisait
   un fichier vide. Elle a rapporte « rien » pour un processus qui n'avait pas encore
   demarre.
2. **Un observable trop large.** Corrigee, la boucle attendait `grep -q FIRST_FRAME_MS` —
   qui matche la **banniere** disant « Startup is printed as FIRST_FRAME_MS below ». Elle
   sortait donc sur les instructions, pas sur la mesure. Corrige en `^FIRST_FRAME_MS [0-9]`.
3. **Un `_ => panic!()` mort.** Dans le test du canal de commande, cf. le neuvieme cas du
   motif dans `AGENTS.md` : clippy l'a attrape.

Les deux premieres sont la meme regle que le cinquieme cas du tableau : **choisir
l'observable le plus etroit qui exprime la propriete.** Une attente qui n'attend pas et un
motif qui matche sa propre documentation produisent tous deux une sortie plausible, donc
aucune verification.

## Dette a surveiller

- **`Input` / `Textarea` / `Editor` viennent d'etre separes** sur `main` (PR #2691,
  2026-08-13). L'API de 0.5.1 n'est donc pas la forme finale, et une future montee de
  version demandera une adaptation cote appelant. A relire au moment ou une 0.6 sortira.
- **Les features forcees.** Si l'adoption est validee, `wayland` et `x11` seront allumes sur
  une application macOS. Deux sorties possibles : une PR chez eux pour rendre les features
  de gpui configurables, ou l'accepter et le documenter dans l'ADR. **Ne pas le laisser
  implicite.**
- **Le disque.** Le build complet a porte `target/` a **21 Go** et rempli le volume a 99 %.
  Correctif dans `[profile.dev]` : `target/debug/deps` tombe de 17 Go a 1,7 Go. `just clean`
  garde `target/release` ou vit le binaire de sonde ; `just clean-deep` vide tout, cache cargo
  compris — 4,9 Go aujourd'hui. **Ca se reproduira**, d'ou les recettes.
