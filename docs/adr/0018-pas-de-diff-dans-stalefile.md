# 0018 — `StaleFile` ne portera pas de resume du changement

- **Statut** : Acceptee. Decision de fond inchangee — pas de resume, pas de diff a l'admission.
  **Forme du texte livre tranchee le 2026-08-13** : deux lines, pas trois.
- **Date** : 2026-08-12, revise le 2026-08-13

> ⚠️ **A lire avant les tableaux ci-dessous.** Les mesures du 2026-08-12 (`5/5`) et le premier
> rejeu du 2026-08-13 (`3/3`) portent sur `ConfigurableNotice::Neutral`, **pas** sur
> `StaleReadNotice` — le contributeur que le produit utilise. Les deux textes differaient d'une
> line. Mesure directe de la production dans cet etat : **`3/6`**. La line a ete retiree, le
> texte livre **est** desormais le texte neutre, et le rejeu donne **`6/6`**.
>
> Chronologie et reserves dans
> « [Le texte livre n'avait jamais ete mesure](#le-texte-livre-navait-jamais-ete-mesure) » puis
> « [Decision : le texte livre perd sa troisieme line](#decision--le-texte-livre-perd-sa-troisieme-line) ».
> **Six runs ne donnent aucune puissance statistique** : ce qui est solide est la direction, pas
> l'amplitude.

## Contexte

L'hypothese la plus intuitive du projet etait qu'un agent ne suivrait un avis de lecture
perimee que s'il comprenait **de quoi il parle** : « `auth.rs` a change » serait trop
opaque, et il faudrait dire « la fonction `verify_token` a ete renommee en
`validate_token` ».

Cette hypothese avait un prix precis, et il n'etait pas petit :

- `StaleFile` doit porter un resume du changement, donc le type public change.
- Le registre doit **calculer un diff a l'admission**, donc de l'I/O et du calcul sur le
  chemin chaud, la ou tout le reste a ete concu pour repondre en microsecondes.
- Il faut conserver le contenu d'avant, ou le relire, ou le reconstruire depuis le journal.
- Il faut resumer un diff en langage naturel — une heuristique par langage, donc une
  surface qui ne cesse plus de grossir.

Le log de la phase 2 renforcait l'hypothese : les agents y avaient rationalise un message
opaque (« refusee par un hook de verification ») vers le concept familier le plus proche
plutot que d'enqueter.

**On a mesure avant de payer.**

## La mesure

Trois formulations de l'avis, cinq runs chacune, avec de vraies sessions Claude Code sur le
scenario canonique : A lit `auth.rs`, B renomme `verify_token` en `validate_token`, A ecrit
`handlers.rs` avec l'ancien nom, puis A recoit l'avis devant son message suivant.

| variante | runs | avis injecte | relit `auth.rs` | bon nom | ancien nom seul | sur-ecriture |
|---|---|---|---|---|---|---|
| **neutre** | 5 | 5/5 | **5/5** | **5/5** | 0/5 | 0/5 |
| directive | 5 | 5/5 | 5/5 | 5/5 | 0/5 | 0/5 |
| contextuelle | 5 | 5/5 | 5/5 | 5/5 | 0/5 | 0/5 |

Les trois variantes :

- **neutre** — les faits seuls : le fichier, l'auteur, le delai. Aucun ordre.
- **directive** — les memes faits, plus une instruction explicite de relecture.
- **contextuelle** — les faits, plus le resume du changement (simule pour la mesure).

## Decision

**`StaleFile` ne portera pas de resume du changement, et le registre ne calcule aucun diff
a l'admission.** Le texte de la variante **neutre** devient la forme canonique de l'avis.

La variante neutre obtient **5/5 sur les trois colonnes qui comptent** : l'agent relit le
fichier perime, et le fichier final porte le bon nom. Elle fait aussi bien que les deux
autres tout en etant la moins chere et la moins intrusive — elle n'ordonne rien.

L'hypothese qui justifiait la depense est donc **refutee** : dire *ce qui* a change n'ajoute
rien a dire *qu'il faut relire*. Nommer le fichier, l'auteur et le delai suffit a declencher
l'enquete que l'agent mene ensuite tout seul.

## Consequences

- Le chemin chaud de l'admission reste un hash blake3 et une comparaison. Pas de diff, pas
  de lecture de l'ancien contenu, pas d'heuristique par langage.
- `ConfigurableNotice` et `NoticeVariant` sont conserves mais marques **experimentaux, non
  retenus**. La simulation du resume ne sert plus qu'a documenter ce qui a ete mesure — elle
  n'est plus un point d'extension du produit.
- `StaleReadNotice` reste le contributeur du produit. **Son texte n'est pas celui de la variante
  neutre** : il ajoute une line de relecture, et cette divergence — affirmee ici par erreur
  pendant deux campagnes — est mesuree et traitee dans
  « [Le texte livre n'avait jamais ete mesure](#le-texte-livre-navait-jamais-ete-mesure) ».

### Pourquoi on n'ajoute pas le diff « au cas ou »

C'est la tentation qu'il faut nommer pour y resister : le resume ne *couterait pas si cher*,
il pourrait *servir plus tard*, et on l'a deja simule.

Trois raisons de ne pas le faire.

1. **Une mesure qui ne change rien est un resultat, pas une invitation a hedger.** Ajouter
   le diff apres avoir montre qu'il n'apporte rien, c'est dire que la mesure ne comptait pas.
   Autant ne pas l'avoir faite.

   **Cet argument a une portee precise, et il faut la nommer** : il ne vaut que **tant que
   les conditions de mesure tiennent**. Il interdit d'ajouter le resume *sans nouvelle
   donnee*, par prudence ou par gout du completude. Il n'interdit pas de rouvrir la question
   quand les conditions changent.

   Concretement, si l'une des quatre limites de la dette de validation ci-dessous saute —
   session longue, changement subtil, `Grep` rouvert, plusieurs fichiers perimes a la fois —
   alors **le resultat ne s'applique plus a la situation observee**, et la question se rouvre
   legitimement. Ce n'est pas revenir sur cette decision : c'est **mesurer a nouveau**, dans
   des conditions ou l'ancienne mesure ne dit rien.

   La difference entre les deux est operationnelle. « Ajoutons le resume au cas ou » se
   refuse. « Cette limite a saute, rejouons la manche » s'accepte, et le resultat de ce
   rejeu remplacera cet ADR ou le confirmera.
2. **Le cout n'est pas dans l'ajout, il est dans le maintien.** Un champ dans un type public
   se propage : le journal le persiste, l'interface l'affiche, les tests le couvrent, et
   quelqu'un finit par en dependre. Un diff a l'admission met de l'I/O sur le seul chemin
   dont la latence compte, et une heuristique de resume ne se supprime plus jamais.
3. **La v0.1 a une variable a itererer, et ce n'est pas celle-la.** Le vrai risque produit
   reste le taux de faux positifs. Un avis plus riche ne le reduit pas — il rend chaque faux
   positif plus long a lire.

Si un cas reel montre un jour que l'agent a besoin du contenu du changement, il aura des
donnees derriere lui. Le pari inverse n'en a aucune.

## La dette de validation

**Ce resultat est solide sur ce qu'il mesure, et ce qu'il mesure est etroit.** Le
`15/15` doit se lire comme le signe d'un **test qui ne discrimine plus**, pas comme la preuve
d'un message optimal.

Quatre limites, a nommer plutot qu'a oublier :

- **Le scenario est court.** Trois tours pour la session mesuree. Un agent qui vient de lire
  `auth.rs` deux tours plus tot a encore le fichier en tete ; l'avis tombe dans un contexte
  ou il est facile a suivre.
- **`Grep`, `Glob` et `Bash` etaient fermes**, pour forcer la lecture par le chemin ACP
  (ADR 0016). Un agent qui dispose de tous ses outils n'explore pas de la meme facon.
- **Peu de contexte accumule.** Pas de longue conversation, pas de plan en cours, pas de
  travail concurrent a abandonner. C'est precisement dans ces situations qu'un avis se fait
  ignorer ou sur-interpreter.
- **Un seul harness, un seul modele, un seul renommage.** Le changement mesure est le plus
  lisible qui existe : un identifiant renomme. Un changement de semantique sans changement
  de signature serait beaucoup plus dur a rattraper, et l'avis neutre n'aurait peut-etre
  plus le meme effet.

**A rejouer sur un cas realiste** : session longue, outils complets, changement moins
evident, et un plan deja engage du cote de la session avertie. Si l'avis neutre tient dans
ces conditions, la decision est confirmee ; sinon c'est la que le resume trouvera sa
justification — avec des donnees.

**Chacune de ces quatre limites est un declencheur de rejeu a part entiere.** Il n'est pas
necessaire qu'elles sautent toutes : une seule suffit pour que le `15/15` cesse de dire
quelque chose sur la situation qu'on observe. Le tableau ci-dessus mesure un scenario, pas
le produit.

### Rejeu du 2026-08-13 — outils ouverts. Une limite sur quatre est levee.

Le declencheur explicite a ete tire : la manche rejouee avec `Grep`, `Glob` et `Bash`
**ouverts** (`--open-tools`), variante neutre, trois runs.

| variante | runs | avis | relit | bon nom | ancien | sur-ecr. |
|---|---|---|---|---|---|---|
| neutre, **outils ouverts** | 3 | 3/3 | 3/3 | 3/3 | 0/3 | 0/3 |

Colonnes brutes, aucune interpretation — meme lecture que le tableau precedent.

**Un resultat que la manche ne cherchait pas** : le read-set s'est peuple les trois fois. Le
harnais avorte le run si `auth.rs` n'est pas entre dans le read-set apres le tour 1 ; les
trois runs sont alles au bout, donc l'agent a lu par le chemin ACP **alors que `Grep` etait
disponible**.

C'est le **meme signal que sur `Bash`** ([ADR 0026](0026-politique-bash-un-seul-motif.md)), et
il se formule pareil : **une capacite n'est pas une propension.** L'agent *peut* lire par
`Grep`, il ne le *choisit* pas quand son outil de lecture est la. Ce qui ne diminue pas le trou —
une capacite suffit a le creuser — mais change son ordre de grandeur attendu, et c'est
exactement ce que le mode ombre va chiffrer ([ADR 0027](0027-trou-lecture-ouvert-et-mesure-en-ombre.md)).

**Avec la limite deja identifiee** : le tour 1 nomme l'outil dans son enonce. Ce signal-la est
donc une indication, pas une mesure du choix spontane.

**Ce que ce rejeu ne leve pas**, et il faut le dire aussi precisement :

- Le tour 1 **nomme l'outil** dans son enonce (« lis auth.rs avec l'outil de lecture de
  fichier »). Le choix spontane d'outil pour la lecture n'est donc **pas** mesure ici — seule
  la disponibilite des autres outils a change.
- Le scenario reste **court**, le contexte accumule **faible**, et le changement mesure reste
  **le plus lisible qui existe** — un identifiant renomme.

**Trois des quatre limites tiennent donc toujours**, et la decision reste ce qu'elle etait :
pas de resume dans `StaleFile`. La variante contextuelle n'a pas ete rejouee, faute d'espace
pour demontrer quoi que ce soit — a `3/3` sur toutes les colonnes, l'avis neutre ne laisse
aucune marge ou un resume pourrait faire mieux. Depenser des runs pour le confirmer aurait
mesure le plafond, pas la variante.

### Rejeu du 2026-08-13 — avis en anglais. La mesure tient.

La conversion du depot en anglais a change **le texte meme dont cet ADR mesure l'effet**. Une
mesure faite sur une chaine qui n'existe plus ne dit rien du produit, donc elle a ete refaite.

Conditions identiques a la mesure d'origine — variante neutre, outils fermes, trois runs —
avec une precision qui compte : **le harnais entier est passe en anglais**, pas seulement
l'avis. Les trois prompts envoyes a A et a B etaient en francais ; les traduire faisait partie
de la conversion, et ne traduire que l'avis aurait mesure un melange qui n'aurait jamais
existe dans le depot.

| variante | runs | avis | relit | bon nom | ancien | sur-ecr. |
|---|---|---|---|---|---|---|
| neutre, **avis et prompts en anglais** | 3 | 3/3 | 3/3 | 3/3 | 0/3 | 0/3 |

Colonnes brutes, aucune interpretation. L'avis injecte, verbatim :

```
[Trame] auth.rs was changed by session "refactor-api"
        after you read it (a few seconds ago).
```

**Identique a la mesure francaise, colonne par colonne.** C'etait l'attendu — les trois
variantes plafonnaient deja — et c'est precisement pour ca qu'il fallait le verifier plutot
que de le supposer : un plafond ne dit pas qu'une variable n'a pas d'effet, il dit que le
dispositif ne peut pas le voir. Le `3/3` **confirme la traduction, il ne revalide pas la
decision** : celle-ci repose toujours sur les trois limites ci-dessus, inchangees.

**Un ecart que ce rejeu a rendu visible, et qui n'a rien a voir avec la traduction.** En
relisant le texte injecte a cote du texte de production, ils ne sont pas les memes :

| | 3e line |
|---|---|
| `ConfigurableNotice::Neutral` — **ce que la manche mesure** | aucune |
| `StaleReadNotice` — **ce que le produit envoie** | `Re-read it before continuing if your work depends on it.` |

La forme livree est donc un hybride : le constat de la neutre, plus une line directive. L'ecart
preexiste a la traduction — la version francaise avait exactement la meme structure, et le test
`only_the_neutral_variant_orders_nothing` epingle l'absence de cette line dans la variante neutre. Il
n'est donc **pas** une regression, mais **le `5/5` puis le `3/3` de cet ADR ne portent pas sur
la chaine que le produit envoie.** Ce constat a declenche la mesure de la section suivante.

### Le texte livre n'avait jamais ete mesure

Le harnais mesure desormais `StaleReadNotice` directement, par une variante `production` qui est
le **defaut** de la manche. `ConfigurableNotice` reste un **dispositif de comparaison contre la
production**, jamais un substitut.

Trois textes, meme jour, memes conditions — outils fermes, scenario canonique, aucun autre
changement entre les batteries :

| texte injecte | runs | avis | relit | bon nom | ancien seul | sur-ecr. |
|---|---|---|---|---|---|---|
| **`StaleReadNotice` — ce que le produit envoie** | **6** | 6/6 | **3/6** | **3/6** | **3/6** | 0/6 |
| `ConfigurableNotice::Neutral` | 3 | 3/3 | 3/3 | 3/3 | 0/3 | 0/3 |
| `ConfigurableNotice::Directive` | 3 | 3/3 | 3/3 | 3/3 | 0/3 | 0/3 |

Colonnes brutes, aucune interpretation. Les trois textes, verbatim :

```
production   [Trame] auth.rs was changed by session "refactor-api"
                     after you read it (a few seconds ago).
                     Re-read it before continuing if your work depends on it.

neutre       [Trame] auth.rs was changed by session "refactor-api"
                     after you read it (a few seconds ago).

directive    [Trame] auth.rs was changed by session "refactor-api"
                     after you read it (a few seconds ago).
                     Re-read auth.rs before continuing, and fix whatever depends on it.
```

**Le texte livre est le seul des trois qui echoue.** Les six runs de production se separent en
deux moities nettes : trois fois l'agent relit `auth.rs` et ecrit le bon nom, trois fois il ne
relit pas du tout — `reads after = 0` — et `handlers.rs` part avec `verify_token`. C'est
exactement le mode d'echec que Trame existe pour attraper.

La comparaison est propre : la production et la directive ne different que par leur troisieme
line, et la neutre n'en a pas. **Une seule variable separe `3/6` de `3/3`.**

**Ce que la mesure etablit** : le `15/15` puis le `3/3` decrivaient une chaine que le produit
n'envoie pas, et la chaine qu'il envoie fait moitie moins bien que les deux formes mesurees.
Six runs suffisent a ecarter la variance comme explication unique — trois echecs d'un cote, zero
sur six de l'autre — dans une manche qui n'avait jamais rien mis en defaut jusqu'ici. Le
dispositif s'est mis a discriminer le jour ou on lui a donne le bon texte a mesurer.

**Ce qu'elle n'etablit pas, et il faut le dire aussi precisement.** La troisieme line de
production differe de la directive sur **deux points a la fois**, donc confondus :

1. **`it` au lieu du nom du file.** Un pronom dont le referent est ambigu — deux choses
   viennent d'etre nommees, le file et la session.
2. **`if your work depends on it`, une conditionnelle.** Elle donne a l'agent une licence
   explicite de ne rien faire, la ou la directive presuppose la dependance et instruit.

Le fait que la neutre — **aucun ordre du tout** — fasse `3/3` pese en faveur du second : ce
n'est pas l'absence d'instruction qui coute, c'est cette instruction-la. Mais ce raisonnement est
une hypothese formee **apres** avoir vu les donnees, et il ne se lit pas dans le tableau. Le
discriminer demande de varier un point a la fois : `Re-read auth.rs before continuing if your
work depends on it.` isole le pronom, `Re-read it before continuing.` isole la conditionnelle.

**Ce que la decision de fond devient.** Rien ici ne touche au resume : aucune des trois formes
ne porte de diff, et l'echec de la production ne s'explique pas par un manque de contexte — la
neutre en dit **moins** et reussit. **`StaleFile` ne portera toujours pas de resume, et le
registre ne calculera toujours aucun diff a l'admission.** Ce qui etait rouvert etait la
**formulation de la troisieme line**, pas la structure de la donnee — et c'est tranche dans la
section suivante.

### Decision : le texte livre perd sa troisieme line

`StaleReadNotice` est desormais **exactement** le texte neutre. La line
`Re-read it before continuing if your work depends on it.` est retiree.

**Trois raisons, dans l'ordre de leur poids.**

1. **La neutre fait `3/3` sans aucune instruction.** Le signal factuel suffit : l'agent sait
   quoi en faire. Rien ne justifie de payer une line qui n'ajoute pas de fait.
2. **`if your work depends on it` donne une licence explicite de ne rien faire.** Le mecanisme
   se formule en une phrase : **un agent qui recoit un fait agit ; un agent qui recoit un fait
   plus la permission de l'ignorer l'ignore une fois sur deux.** C'est litteralement ce que
   montrent les six runs — trois relectures, trois abandons.
   Le pronom `it` est probablement secondaire, et **on n'a pas besoin de trancher entre les
   deux** : la neutre n'a ni l'un ni l'autre, et elle marche.
3. **Cet ADR affirmait deja que la neutre etait la forme canonique.** Le produit ne la
   respectait pas. **On aligne le code sur la decision ecrite, on ne prend pas une nouvelle
   decision.**

#### Rejeu apres alignement

Memes conditions — outils fermes, scenario canonique, six runs :

| texte injecte | runs | avis | relit | bon nom | ancien seul | sur-ecr. |
|---|---|---|---|---|---|---|
| `StaleReadNotice`, **3 lines** (avant) | 6 | 6/6 | 3/6 | 3/6 | 3/6 | 0/6 |
| `StaleReadNotice`, **2 lines** (apres) | **6** | 6/6 | **6/6** | **6/6** | **0/6** | 0/6 |

Colonnes brutes. Le texte expedie aujourd'hui :

```
[Trame] auth.rs was changed by session "refactor-api"
        after you read it (a few seconds ago).
```

#### ⚠️ La reserve, a lire avec le tableau

**Six runs ne donnent aucune puissance statistique.** Ce qui est solide, c'est **la direction** :
une variable, une coupure nette, un mecanisme d'echec identifie. **L'amplitude reste faible.**

Personne ne devrait citer « 3/6 contre 6/6 » comme un effet mesure. Ce qui est etabli est
qualitatif : cette line-la coutait des runs, et la retirer n'en coute aucun.

#### Dette : les deux points restent confondus

`it` contre le nom du file, et la conditionnelle contre l'instruction seche. La decision ne
depend pas de leur separation — la neutre elimine les deux — mais la question reste bonne, et
elle informerait toute instruction qu'on voudrait ajouter plus tard.

Deux formes suffisent a discriminer, un point a la fois :

```
Re-read auth.rs before continuing if your work depends on it.   -> isole le pronom
Re-read it before continuing.                                   -> isole la conditionnelle
```

**Ne bloque rien.** A traiter quand le harnais sera plus rapide : six runs coutent aujourd'hui
une dizaine de minutes et des jetons, ce qui rend une matrice a quatre cellules disproportionnee
devant son enjeu.

#### Le test qui a fait son travail, et ce qu'il devient

`no_variant_is_the_production_notice` affirmait qu'**aucune** variante n'egalait la production.
Il a echoue au run suivant l'alignement, **en portant son propre mode d'emploi** : rejouer la
mesure, mettre l'ADR a jour, puis lever l'assertion. Les trois ont eu lieu.

Il est remplace par `production_is_exactly_the_neutral_text`, qui epingle la **relation voulue** :
egalite exacte avec la neutre, difference maintenue avec les deux autres. C'est plus fort que la
forme precedente — elle detectait un accident, celle-ci epingle une decision — et son controle
negatif est fait : remettre une line d'un cote ou de l'autre la fait tomber.
`the_shipped_notice_states_facts_and_orders_nothing` porte la propriete que la mesure a achetee.

### Le septieme cas du motif

Celui-ci est plus vicieux que les six de `AGENTS.md`, parce que le dispositif de mesure
fonctionnait parfaitement — il mesurait juste **autre chose que le produit**, et son plafond
`15/15` rendait la substitution indetectable.

> **Un harnais de mesure doit consommer le composant de production, pas un jumeau.** Si la
> mesure passe par un type dedie a l'experience, ce type doit etre construit comme une
> **comparaison contre la production**, et un test doit constater qu'ils diffèrent.

Trois proprietes du piege, toutes presentes ici :

- **Les deux textes se ressemblaient**, assez pour qu'une relecture les confonde.
- **Le plafond masquait tout.** A `15/15`, aucun ecart ne pouvait se manifester.
- **Aucun test ne les comparait.** Chaque texte etait epingle contre lui-meme, jamais contre
  l'autre — c'est le meme angle mort que le compteur global du cinquieme cas.

Et ce qui l'a trouve n'est ni un test ni un run : c'est **une relecture cote a cote**, faite en
verifiant tout autre chose. La traduction du depot a impose de lire les deux chaines dans la
meme heure.

## Alternatives ecartees

- **Ajouter le resume quand meme.** Voir ci-dessus : ce serait annuler la valeur de la
  mesure.
- **Retenir la variante directive.** Elle fait aussi bien, sans faire mieux, et elle donne un
  ordre. Un outil qui ordonne a l'agent de relire perd le droit de se tromper : chaque faux
  positif devient une instruction inutile plutot qu'une information ignorable.
- **Rendre la variante configurable par l'utilisateur.** Trois textes a maintenir pour un
  reglage dont la mesure dit qu'il ne change rien. Les variantes existaient pour etre
  mesurees, pas pour etre livrees.

> **Cette alternative a change de statut le 2026-08-13.** « Elle fait aussi bien, sans faire
> mieux » etait vrai contre la variante neutre, et reste vrai contre elle : `3/3` de chaque cote.
> Mais la directive fait **mieux que le texte livre** — `3/3` contre `3/6` — donc elle redevient
> candidate. L'argument contre elle est intact et il faut le peser tel quel : un outil qui ordonne
> perd le droit de se tromper, et l'invariant 8 est le risque produit numero un.

## Ce qui invaliderait cette decision

Un rejeu ou l'avis neutre echouerait la ou la contextuelle reussirait.

Ce rejeu devient **legitime des qu'une** des quatre limites de la dette de validation saute :
session longue, changement subtil, `Grep`/`Glob`/`Bash` rouverts, ou plusieurs fichiers
perimes simultanement. Le declencheur n'est donc pas « quelqu'un pense que le resume serait
mieux » — ca reste refuse — mais « les conditions ont change, donc la mesure ne couvre plus
le cas ».

C'est une invalidation par **changement de perimetre**, pas par changement d'avis. Et elle
est attendue : la premiere reouverture probable est le jour ou `Grep` sera rouvert dans le
produit, ce que la sonde `PreToolUse` cherche justement a rendre possible.
