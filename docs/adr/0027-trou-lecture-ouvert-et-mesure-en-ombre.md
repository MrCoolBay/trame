# 0027 — Le trou lecture reste ouvert, et se mesure en mode ombre

- **Statut** : Acceptée — **décision de ne pas fermer**, avec le protocole qui la rouvrira
- **Date** : 2026-08-13
- **Prolonge** l'[ADR 0021](0021-pas-d-analyse-de-la-sortie-de-grep.md) et l'invariant 2

## Le trou, précisément

Les fichiers rapportés par `Grep` et `Glob` arrivent bien au registre — la plomberie existe,
elle relit chaque fichier pour l'empreinter (invariant 10) et les deux formes de chemins sont
absorbées. Mais `ReadKind::GrepHit.is_substantial()` rend `false`, donc **ils n'entrent pas dans
le read-set** et aucun `StaleRead` ne peut se déclencher dessus.

```
read-set observe : []
```

**Une seule ligne fermerait le trou.** Cet ADR dit pourquoi on ne la change pas encore.

## Pourquoi basculer serait un pari, pas une décision

La manche expérimentale mesure des **taux de succès** : l'avis est-il injecté, l'agent relit-il,
le fichier final est-il correct. Elle ne mesure **jamais** le taux de **faux positifs**.

Or c'est précisément la variable du **risque produit numéro un** :

> **Invariant 8 — silencieux quand c'est propre.** ~95 % du trafic doit passer sans un mot. Un
> outil qui crie au loup est désactivé en une semaine, avant tout risque technique.

Et l'ordre de grandeur du risque est réel : un `grep -r` sur un vrai codebase rapporte des
dizaines à des centaines de fichiers. Chacun entrerait dans le read-set, et **toute** écriture
d'une autre session sur **l'un** d'eux produirait un avis. C'est exactement ce que le filtre
`ReadKind` existe pour prévenir.

Fermer le trou sans cette donnée, ce serait **parier sur l'invariant 8** — troquer un trou connu
contre un bruit inconnu.

## Le booléen force un faux choix

`is_substantial()` rend un `bool`, ce qui n'offre que deux positions : aucune couverture, ou
tout le bruit. Mais les lectures ne sont pas de même nature :

| Ce que rend le `Grep` | Ce que c'est |
|---|---|
| 3 fichiers | une **lecture ciblée** — l'agent cherchait quelque chose de précis |
| 300 fichiers | une **exploration** — il ratissait |

Traiter les deux pareil est une erreur de modèle, dans un sens ou dans l'autre.

## Décision

**Trois choses, dont une seule touche le code de production.**

### 1. On ne bascule pas. Le trou reste ouvert et nommé

Il est écrit dans le README, dans le concept, dans l'invariant 2 et dans l'interface. Un trou
nommé vaut mieux qu'un trou ignoré — et un trou nommé vaut mieux qu'un bruit non mesuré.

### 2. Mode ombre : on compte ce qu'on aurait dit, et on ne dit rien

Les hits `Grep` entrent dans un **read-set parallèle** qui ne participe à aucun verdict. À
chaque admission, le registre compte les avis que ces lectures **auraient** produits.

**La condition de validité, et c'est le test le plus important du dispositif** : l'ombre ne
change aucun verdict. Une mesure qui modifie ce qu'elle mesure ne mesure rien. Le test rejoue le
même scénario avec et sans lectures d'ombre et compare les verdicts ; le contrôle négatif a été
fait — en faisant l'ombre alimenter le read-set réel, trois tests tombent.

Deux raffinements qui évitent une mesure fausse :

- **Un avis que le vrai verdict a déjà dit n'est pas compté** comme potentiel. Sinon le compteur
  doublerait l'existant et surestimerait le bruit ajouté — la mesure serait pessimiste sans
  qu'on le sache.
- **Le TTL s'applique comme au réel.** Une lecture d'ombre expirée ne compte pas.

Le compteur est affiché dans la TUI et la GUI, **distinctement des avis réels** : couleur
propre, libellé « N potentiels (ombre) », et une ligne de flux qui dit *auraient été émis*,
jamais *ont été émis*. Une interface qui les confondrait annoncerait une couverture qui
n'existe pas.

### 3. Le seuil est préparé, pas choisi

On enregistre, pour chaque avis potentiel, **la taille du résultat du `Grep` d'où il vient**.
`StatsOmbre::avis_potentiels_si_seuil(n)` répond alors pour **n'importe quel** `n`, après coup :

```rust
stats.avis_potentiels_si_seuil(1)    // 0  — aucune recherche a 1 fichier
stats.avis_potentiels_si_seuil(2)    // 1  — la recherche ciblee
stats.avis_potentiels_si_seuil(50)   // 2  — ciblee + moyenne
stats.avis_potentiels_si_seuil(300)  // 3  — tout, exploration incluse
```

**`n` n'a aucune valeur par défaut, et n'en aura pas avant la mesure.** C'est le paramètre de
l'expérience, pas un réglage du produit. Enregistrer la distribution plutôt qu'appliquer un
seuil évite de rejouer la mesure pour chaque hypothèse — et évite surtout de choisir `n` à
l'intuition puis de chercher les données qui le confirment.

## Le protocole de mesure

Ce qui manque n'est pas du code, c'est de **l'usage réel**. Le compteur s'accumule sur le
travail quotidien, et la lecture se fait sur trois chiffres :

1. **`lectures_ombre`** — le dénominateur. Sans lui, « douze avis potentiels » ne veut rien dire.
2. **`avis_potentiels`** — ce que la bascule complète aurait ajouté.
3. **`par_taille`** — la distribution, qui dit *où* couper.

La question à laquelle répondre : **quelle proportion de ces avis potentiels aurait été
pertinente ?** Elle ne se lit pas dans le compteur seul — il faut regarder les cas. D'où
l'affichage : le flux nomme le cumul au moment où il bouge, ce qui permet de rapprocher un avis
potentiel du travail en cours et de juger, cas par cas, s'il aurait aidé ou agacé.

**Ce que le compteur ne dira pas** : si l'agent aurait *tenu compte* de l'avis. Le mode ombre
mesure le volume, pas l'utilité. C'est la même limite que la manche de l'ADR 0018, dans l'autre
sens.

## Conséquences

- **Le trou lecture reste le dernier problème structurel ouvert**, et il est maintenant
  instrumenté. C'est un progrès de nature différente de sa fermeture, et il ne faut pas les
  confondre dans un rapport d'avancement.
- Le coût du mode ombre est celui de la relecture, mesuré : **0,06 ms par fichier**, 18,5 ms pour
  300. Rien qui pèse.
- `ReadKind::GrepHit` reste non substantiel, et **un test épingle cet état** avec le
  raisonnement dans sa doc. Il échouera le jour où quelqu'un basculera le drapeau : il forcera à
  relire plutôt qu'à découvrir le bruit en production.
- Un `Bash` de lecture (`cat`, `head`) n'est couvert ni par le réel, ni par l'ombre. Nommé.

## Alternatives écartées

- **Basculer `GrepHit` en substantiel maintenant.** C'est le pari décrit plus haut. Il pourrait
  être gagnant ; on n'en sait rien, et l'enjeu est le risque produit n° 1.
- **Choisir un seuil plausible tout de suite** — 5, 10, 20 fichiers. Ça a l'air prudent et c'est
  la même erreur en plus petit : un chiffre inventé, qu'on défendrait ensuite par habitude. La
  distribution coûte le même code et donne la réponse.
- **Journaliser les avis potentiels** dans SQLite avec les vrais. Le journal est append-only et
  fait autorité sur ce qui *a eu lieu* ; y écrire des hypothétiques le rendrait ambigu, même avec
  une colonne pour les distinguer. Les compteurs vivent en mémoire, dans le registre.
- **Un troisième `ReadKind`** — `SearchHit` distinct de `GrepHit` — avec sa propre
  substantialité. Ça déplace le booléen sans le supprimer, et il faudrait quand même mesurer.

## Ce qui invaliderait cette décision

- **La mesure montre un faible taux d'avis potentiels par lecture d'ombre**, concentré sur des
  recherches ciblées. Alors on ferme le trou, avec ou sans seuil selon la distribution — et
  c'est une décision, plus un pari.
- **La mesure montre l'inverse** : un volume d'avis potentiels dominé par des explorations
  larges. Le trou reste ouvert, et l'ADR 0021 se confirme : le mode `content` n'était pas le
  problème principal.
- **Un `StaleRead` manqué et coûteux, attribuable à une lecture par `Grep`**, observé en usage
  réel. Ce serait la donnée symétrique : le prix du trou, mesuré, à mettre en face du prix du
  bruit.
