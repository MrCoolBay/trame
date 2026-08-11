# 0007 — Concurrence optimiste avec validation du read-set

- **Statut** : Acceptee
- **Date** : 2026-08-11

## Contexte

C'est l'ADR central. Tout le reste du projet existe pour le servir.

Puisqu'il n'y a pas d'isolation (ADR 0002), plusieurs agents ecrivent dans le meme
arbre. La question est : quel modele de controle de concurrence ?

Le **locking pessimiste** est inadapte pour trois raisons independantes, chacune
suffisante :

1. Les agents tiennent leur transaction pendant des minutes. Un verrou detenu
   plusieurs minutes affame les autres.
2. Ils ne declarent pas leur intention a l'avance. On ne sait pas quoi verrouiller
   avant qu'ils ecrivent.
3. Bloquer un tool call en vol declenche des timeouts cote harness — donc un echec
   de session, pas une attente.

Surtout, le mode d'echec le plus frequent a trois agents **ne produit aucune
collision d'ecriture**, donc aucun verrou ne le verrait :

```
1. Agent A lit auth.rs, memorise la signature de verify_token()
2. Agent B ecrit auth.rs, renomme verify_token() -> validate_token()
3. Agent A ecrit handlers.rs, appelle verify_token()

Deux fichiers differents. Un verrou par fichier ne voit rien. L'arbre est casse.
```

## Decision

Le modele des bases de donnees : **controle de concurrence optimiste avec
validation du read-set**.

Chaque session tient un read-set — les fichiers qu'elle a lus, avec leur empreinte
et l'instant de lecture. Au moment ou elle demande a ecrire, le registre verifie si
un fichier de son read-set a change depuis. Si oui, on ne sait pas *si* ca casse,
mais on sait que **la session raisonne sur un monde qui n'existe plus**. C'est le
seul invariant qui compte.

Quatre verdicts, pas un booleen, parce que la bonne reponse a une collision n'est
pas toujours « non » :

| Niveau | Situation | Reponse |
|---|---|---|
| **0 — Clean** | Aucun recouvrement | Admis, silencieux. ~95 % du trafic. |
| **1 — StaleRead** | Intersection sur le read-set uniquement | **Admis, et on informe l'agent.** Il relit et s'adapte tout seul dans la grande majorite des cas. |
| **2 — DisjointWrite** | Meme fichier, regions disjointes | Admis. Deux fonctions differentes d'un fichier de 2000 lignes, c'est legitime. |
| **3 — Overlap** | Regions qui se recouvrent | Bloque, on demande a l'humain via le mecanisme de permission ACP existant. |

Le niveau 1 est celui qui compte : **la bonne reponse n'est pas de bloquer, c'est
d'informer**. Ce n'est possible que parce qu'ACP donne un canal structure vers
l'agent (ADR 0005), et l'avis est injecte par `PromptContributor` — c'est pour ca
que cette couture n'est pas speculative.

## Consequences

- **Rien n'est bloque en v0.1.** Le registre observe, journalise et informe. Meme
  le niveau 3 passerait, et de toute facon il n'est jamais produit tant que la
  granularite est le fichier entier (ADR 0012). Le blocage se decidera **apres**
  mesure du taux reel de faux positifs sur un mois d'usage propre.
- Le read-set doit etre **filtre** : seules les lectures substantielles, pas les
  hits de grep ni les listings de repertoire. Les agents lisent enormement ; sans
  filtre, le read-set explose et tout devient niveau 1.
- Le read-set doit **decroitre** : dix minutes. Au-dela, le contexte de l'agent a
  tourne de toute facon, et l'avis serait du bruit. C'est le premier cadran a
  tourner si le taux de faux positifs est trop haut.
- Un verdict optimiste peut se tromper dans les deux sens. Faux positif : `auth.rs`
  a change mais sur une partie qui ne concerne pas A — c'est le risque produit
  numero un. Faux negatif : A et B se contredisent semantiquement sans recouvrement
  de lecture — **aucun registre ne peut l'attraper**, le seul filet reel est le
  compilateur et les tests.
- Il faut instrumenter le taux de faux positifs des le depart, sinon la decision de
  la v0.5 se prendra a l'intuition.

## Alternatives ecartees

- **Locking pessimiste par fichier.** Les trois raisons ci-dessus, et il ne voit
  pas le scenario canonique.
- **Detection *a posteriori* par FSEvents uniquement.** Arrive apres l'ecriture,
  donc trop tard pour informer l'agent. Reste utile comme filet pour les ecritures
  hors-bande.
- **Bloquer des le niveau 1.** Un outil qui crie au loup est desactive en une
  semaine. C'est la seule facon sure de tuer le produit.
- **Validation du write-set seulement.** C'est ce que fait tout le monde, et c'est
  exactement ce qui rate le scenario canonique.

## Ce qui invaliderait cette decision

Un taux de faux positifs qui reste inacceptable **apres** avoir tourne les deux
cadrans disponibles — le filtre de read-set et la fenetre de decroissance — et
apres etre passe a une granularite hunk (ADR 0012). Dans ce cas la validation du
read-set serait un mecanisme trop bruyant pour etre expose, et il faudrait le
reduire a un signal passif dans le journal plutot qu'a un avis injecte.
