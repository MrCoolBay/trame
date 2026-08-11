# 0015 — Le canal du registre est borne, a 64

- **Statut** : Acceptee
- **Date** : 2026-08-11

## Contexte

Le registre est un acteur : ses messages arrivent par un `mpsc`. La capacite de ce canal
n'est pas un detail de reglage, c'est **la politique de backpressure du produit**. Trois
comportements possibles quand la file est pleine, et il faut en choisir un
explicitement :

1. **File non bornee** — on n'attend jamais, et une surcharge devient une fuite de
   memoire silencieuse. C'est le choix par defaut de qui ne choisit pas.
2. **Rejeter** le message quand c'est plein. Pour une admission, rejeter signifie soit
   perdre l'ecriture, soit l'autoriser sans verdict. Les deux sont inacceptables : l'un
   casse l'agent, l'autre casse l'invariant.
3. **Attendre** que de la place se libere. L'agent attend. C'est lent, et c'est correct.

## Decision

Canal **borne a 64**, et en saturation on **attend** : `mpsc::Sender::send().await`.
Jamais `unbounded_channel`, jamais `try_send` sur le chemin d'admission.

### Pourquoi 64

Ce n'est pas un chiffre rond pris au hasard. La profondeur *realiste* est de l'ordre de
**cinq** : deux a cinq sessions par projet (cadrage produit), et chaque session n'a au
plus qu'une requete d'ecriture en vol — un agent attend la reponse de son tool call
avant d'en emettre un autre. 64 laisse donc un facteur dix de marge pour les rafales
(plusieurs `RecordRead` groupes, un `Snapshot` de l'interface qui arrive au mauvais
moment), tout en restant assez petit pour que la saturation reste un **signal** plutot
qu'un tampon ou l'on cache un probleme.

### Comportement en saturation, precisement

- L'appelant — la tache ACP qui traite la requete de l'agent — **attend** dans son
  `send().await`. Il ne perd rien et ne double rien.
- L'agent, en face, attend la reponse a son `fs/write_text_file`. C'est exactement le
  comportement souhaitable : mieux vaut un agent qui patiente qu'un agent qui ecrit sans
  verdict.
- Aucun message n'est jamais perdu ni reordonne. L'ordre total, qui est la raison d'etre
  de l'acteur, est preserve.
- **Une saturation durable est un bug, pas un manque de capacite.** A cette echelle, une
  file pleine signifie que l'acteur est bloque sur autre chose : une ecriture disque qui
  pend (ADR 0014), un journal injoignable, un interblocage. La reponse est de
  diagnostiquer, **jamais d'augmenter la capacite** — ce qui ne ferait que retarder et
  masquer le symptome.
- Pour que ce soit diagnosticable, la saturation doit etre **observable** : le registre
  emet un `tracing::warn!` quand la place disponible tombe sous un seuil. Une attente
  silencieuse est un incident qu'on ne comprendra pas.

### Le risque residuel, nomme

Un harness peut avoir un timeout sur ses propres tool calls. Si l'attente depasse ce
timeout, l'agent considere l'ecriture echouee alors qu'elle est en file. Il n'y a pas de
reponse elegante : c'est une raison de plus pour qu'une saturation soit traitee comme un
bug a corriger, et pour ne **jamais** mettre de timeout court cote Trame sur un tour
d'agent — mais bien un timeout sur l'admission elle-meme, qui doit repondre en
millisecondes.

## Consequences

- Le meme raisonnement s'applique au journal, avec une capacite plus grande (256) parce
  que ses messages sont des ajouts sans reponse : ils arrivent en rafales et se traitent
  en dizaines de microsecondes.
- Les deux capacites sont des constantes nommees et documentees dans leur crate, pas des
  litteraux disperses.
- `try_send` reste legitime pour ce qui est **jetable** — une notification d'interface,
  par exemple. Il ne doit jamais apparaitre sur le chemin d'admission.

## Alternatives ecartees

- **Non bornee.** Transforme une surcharge en fuite memoire. Interdit par convention de
  projet, et la raison est ecrite dans le code a chaque `channel()`.
- **Rejeter en saturation.** Perdre une ecriture ou l'admettre sans verdict : les deux
  cassent quelque chose d'essentiel.
- **Une capacite tres grande (10 000).** Rend la saturation invisible et transforme un
  bug de blocage en degradation lente et inexplicable.
- **Un canal par type de message, avec priorites.** `Snapshot` pourrait passer devant
  `Admit`. Ca casse l'ordre total, qui est precisement ce que l'acteur garantit.

## Ce qui invaliderait cette decision

Une saturation observee **alors que l'acteur n'est bloque sur rien** — c'est-a-dire une
charge reelle superieure a cinq requetes concurrentes soutenues par projet. Ce serait le
signe que le cadrage « deux a cinq sessions par projet » a bouge, et c'est le cadrage
qu'il faudrait alors rediscuter, pas la capacite du canal.
