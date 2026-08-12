# 0019 — Héberger **Trame lui-même** sur GitHub

- **Statut** : Acceptée
- **Date** : 2026-08-12
- **Ne remplace pas** l'[ADR 0011](0011-gitlab-self-hosted-en-premier.md), et ne la contredit
  pas. Lire la section « Ce que cet ADR ne dit pas » avant toute autre chose.

## ⚠️ Deux décisions indépendantes qu'il ne faut pas confondre

Cet ADR parle d'**une seule chose** : où vit le dépôt de Trame.

| | Décision | ADR |
|---|---|---|
| **Où vit le code de Trame** | GitHub | **celui-ci** |
| **Quelle forge Trame sait piloter pour les projets qu'il orchestre** | **GitLab self-hosted en première cible, inchangé** | [0011](0011-gitlab-self-hosted-en-premier.md) |

Ce sont deux sujets sans rapport technique. L'un concerne notre hébergement, l'autre concerne
le trait `Forge` et ce qu'il sait faire pour l'utilisateur. **Lire l'un comme l'abandon de
l'autre serait une erreur**, et cet ADR existe autant pour l'empêcher que pour acter le choix.

## Contexte

L'ADR 0011 a été écrite quand Trame était sous FSL-1.1-MIT, à usage personnel. Dans ce cadre,
héberger sur GitLab self-hosted était cohérent : pas de contributeurs attendus, cohérence avec
la forge cible, souveraineté de l'hébergement.

Deux choses ont changé.

La licence est passée en **MIT OR Apache-2.0** ([ADR 0013](0013-licence-open-source-mit-apache.md)),
et avec elle l'objectif : attirer des contributeurs. Or on ne choisit pas un hébergement pour
soi quand on cherche des contributeurs — **on le choisit là où ils sont**. Un projet Rust open
source dont le dépôt vit sur une instance GitLab privée demande à chaque contributeur de créer
un compte sur une forge qu'il ne connaît pas, pour un projet qu'il ne connaît pas encore. C'est
un péage à l'entrée, placé exactement là où il coûte le plus cher.

## Décision

**Le dépôt de Trame sera hébergé sur GitHub.**

L'URL sera fournie par l'utilisateur, qui crée le dépôt. **Aucun remote n'est ajouté par cet
ADR** : `gb-local` pointe aujourd'hui vers le dépôt lui-même
(`/Users/fabienlubin/Projects/trame`), donc **rien n'a jamais quitté la machine** — « pousser »
y est un no-op. La pile de branches reste telle quelle.

**La CI reste sur `.gitlab-ci.yml`** pour l'instant. Le passage à GitHub Actions est un sujet
séparé, à traiter quand le dépôt existera. Garder les deux questions distinctes évite de
transformer un choix d'hébergement en réécriture d'outillage.

## Ce que cet ADR **ne** dit **pas**

À lire comme faisant partie de la décision, pas comme une précaution de style.

**L'ADR 0011 reste entièrement valide, et prioritaire.** Rien de ce qui suit ne change :

- **`base_url` est un champ de première classe** du trait `Forge`, pas une option ajoutée après
  coup. Une instance privée est le cas normal, pas l'exception.
- **`ChangeRequest`, jamais `PullRequest`.** Le vocabulaire du code reste neutre. « Pull
  request » est un terme GitHub ; ce n'est pas parce que notre dépôt y vit que notre code doit
  adopter son vocabulaire.
- **GitLab est la première implémentation du trait `Forge`**, et le modèle de review retenu est
  le sien — des threads résolvables, éventuellement ancrés sur une ligne — parce que c'est le
  sur-ensemble qui se projette correctement sur GitHub.
- **La CI du projet reste `.gitlab-ci.yml`**, ce qui n'est pas qu'une conséquence de l'inertie :
  ça garde l'outillage aligné sur la forge cible.

Autrement dit : **Trame est hébergé sur GitHub et parle GitLab.** Ce n'est pas une
contradiction, c'est la conséquence de deux publics différents — les contributeurs d'un côté,
les utilisateurs de l'autre.

Le jour où quelqu'un lira cet ADR en concluant « GitHub a gagné, on peut renommer
`ChangeRequest` en `PullRequest` » : **non**, et c'est écrit ici pour cette raison.

## Conséquences

- Les contributeurs arrivent sans péage : compte déjà existant, mécanique de fork et de pull
  request déjà connue.
- Le vocabulaire du dépôt et le vocabulaire du code **divergeront**, et c'est assumé. Sur GitHub
  on recevra des *pull requests* ; dans le code on manipulera des `ChangeRequest`. C'est une
  friction de lecture réelle, à documenter dans le futur `CONTRIBUTING.md` plutôt qu'à résoudre
  en cédant sur le nommage.
- La souveraineté de l'hébergement est perdue pour **notre** dépôt. Elle n'a jamais été un
  argument produit — l'angle auditabilité de Trame porte sur ce que l'outil journalise chez
  l'utilisateur, pas sur l'endroit où notre code est stocké.
- `gb-local` reste un remote vers soi-même, donc inutile. À remplacer par le vrai remote quand
  l'URL existera, ou à supprimer.
- Le passage éventuel à GitHub Actions dupliquerait ou remplacerait `.gitlab-ci.yml`. **Non
  tranché.** Une CI GitLab peut piloter un dépôt GitHub par miroir ; le canari ACP et
  `just ci` sont agnostiques.

## Alternatives écartées

- **Rester sur GitLab self-hosted.** Cohérent avec la forge cible, et c'était le bon choix sous
  FSL. Sous MIT/Apache avec l'objectif de contributions, ça place un obstacle à l'entrée pour
  gagner une cohérence qui n'intéresse que nous.
- **GitLab.com plutôt que self-hosted.** Supprime la charge d'hébergement sans supprimer le
  péage : ça reste un compte à créer pour la majorité des contributeurs Rust.
- **Les deux, en miroir.** Deux endroits où les issues et les revues peuvent arriver, donc deux
  endroits à surveiller. Le coût est permanent, le bénéfice symbolique.
- **Attendre d'avoir des contributeurs pour choisir.** L'hébergement est ce qui rend la
  contribution possible ; attendre la conséquence pour décider de la cause.

## Ce qui invaliderait cette décision

Que les contributions ne viennent pas de la communauté Rust généraliste mais d'organisations qui
ont leur propre forge et refusent GitHub — le public visé par l'angle souveraineté. Dans ce cas
le miroir écarté ci-dessus redeviendrait défendable, et **le trait `Forge` n'aurait toujours pas
à changer** : il est déjà neutre, c'est tout l'objet de l'ADR 0011.
