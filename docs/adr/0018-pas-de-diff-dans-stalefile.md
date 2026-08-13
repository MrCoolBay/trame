# 0018 — `StaleFile` ne portera pas de resume du changement

- **Statut** : Acceptee
- **Date** : 2026-08-12

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
- `StaleReadNotice` reste le contributeur du produit, avec le texte neutre.

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
**ouverts** (`--outils-ouverts`), variante neutre, trois runs.

| variante | runs | avis | relit | bon nom | ancien | sur-ecr. |
|---|---|---|---|---|---|---|
| neutre, **outils ouverts** | 3 | 3/3 | 3/3 | 3/3 | 0/3 | 0/3 |

Colonnes brutes, aucune interpretation — meme lecture que le tableau precedent.

**Un resultat que la manche ne cherchait pas** : le read-set s'est peuple les trois fois. Le
harnais avorte le run si `auth.rs` n'est pas entre dans le read-set apres le tour 1 ; les
trois runs sont alles au bout, donc l'agent a lu par le chemin ACP **alors que `Grep` etait
disponible**. Meme signal que sur `Bash` (ADR 0026) : l'agent prefere ses outils de fichiers
dedies.

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

## Alternatives ecartees

- **Ajouter le resume quand meme.** Voir ci-dessus : ce serait annuler la valeur de la
  mesure.
- **Retenir la variante directive.** Elle fait aussi bien, sans faire mieux, et elle donne un
  ordre. Un outil qui ordonne a l'agent de relire perd le droit de se tromper : chaque faux
  positif devient une instruction inutile plutot qu'une information ignorable.
- **Rendre la variante configurable par l'utilisateur.** Trois textes a maintenir pour un
  reglage dont la mesure dit qu'il ne change rien. Les variantes existaient pour etre
  mesurees, pas pour etre livrees.

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
