# 0011 — GitLab self-hosted comme premiere cible de forge

- **Statut** : Acceptee
- **Date** : 2026-08-11

## Contexte

La quasi-totalite des outils pour developpeurs traite GitHub comme la forge par
defaut et GitLab comme un portage tardif. Le symptome est toujours le meme : le
vocabulaire du code dit « pull request », et l'URL de l'instance est un parametre
optionnel ajoute apres coup pour faire plaisir aux instances privees.

Le contexte de Trame est l'inverse : GitLab **self-hosted**. Une instance privee est
le cas normal, pas l'exception, et l'angle produit — auditabilite, souverainete — vise
exactement les organisations qui heberge leur forge.

## Decision

GitLab self-hosted est la premiere cible. Deux consequences concretes, appliquees
des la phase 0 :

1. **`base_url` est un champ de premiere classe.** `Forge::base_url()` est une
   methode du trait, pas une option de configuration. Une implementation qui
   supposerait un domaine en dur ne compile pas contre ce trait.
2. **Nommage neutre : `ChangeRequest`, jamais `PullRequest`.** « Pull request » est un
   terme GitHub ; GitLab dit « merge request ». Le code utilise un troisieme terme,
   neutre, qui ne privilegie ni l'un ni l'autre. De meme `CrId`, `ReviewThread`,
   `open_change_request`.

## Consequences

- Le trait `Forge` est ecrit sans hypothese sur l'hebergeur. GitHub sera une
  implementation parmi d'autres, ni privilegiee ni penalisee.
- Le modele de review de GitLab — des **threads** de discussion, resolvables,
  eventuellement ancres sur une ligne — est ce que le trait expose (`review_threads`,
  `reply`). C'est plus riche que le modele de commentaires plats, et le sur-ensemble
  se projette correctement sur GitHub.
- La boucle de review devient une source de travail : chaque thread non resolu peut
  devenir un `WorkItem`, donc une session. C'est ce que `BranchTarget::Existing`
  (ADR 0005 pour la couture, `trame-core` pour le type) rend possible sans refactor.
- La CI du projet est `.gitlab-ci.yml`, pas GitHub Actions. Coherence, et ca evite de
  decouvrir a la premiere MR que l'outillage suppose l'inverse de la cible.
- Cout reel : un renommage mental. `ChangeRequest` demande une seconde d'adaptation a
  qui vient de GitHub, et rend le code correct pour tout le monde.

## Alternatives ecartees

- **GitHub d'abord, GitLab ensuite.** Le chemin habituel, et celui qui produit le
  `base_url` bricole en v0.3. Le cout du retrofit n'est pas dans le champ lui-meme,
  il est dans le vocabulaire qui s'est repandu partout entre-temps.
- **Une abstraction de forge sans implementation, decidee plus tard.** C'est ce que
  fait la phase 0 pour le trait, mais le choix de la premiere cible doit etre pris
  maintenant : c'est lui qui determine le vocabulaire, et le vocabulaire est ce qui
  coute cher a changer.
- **Supporter les deux d'entree.** Deux implementations a maintenir avant d'avoir un
  seul utilisateur.

## Ce qui invaliderait cette decision

Rien. Le nommage neutre et `base_url` en champ de premiere classe sont corrects quelle
que soit la forge qui arrive en premier dans les faits.
