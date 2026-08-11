//! **Couture non speculative.** Le pipeline de composition du prompt.
//!
//! C'est par ce mecanisme que l'avis de lecture perimee est injecte dans le
//! contexte de l'agent. Sans lui, [`crate::Verdict::StaleRead`] serait une ligne
//! de journal que personne ne lit — et le produit n'aurait plus de raison
//! d'exister. La v0.1 en a donc besoin, contrairement aux deux autres coutures.
//!
//! Le modele est une liste ordonnee de contributeurs. Chacun regarde le contexte
//! de la session et decide s'il a quelque chose a dire.

use std::path::Path;

use chrono::TimeDelta;

use crate::clock::Timestamp;
use crate::project::Project;
use crate::session::Session;
use crate::verdict::Verdict;

/// Ce que voit un contributeur au moment de composer.
///
/// **Pas `#[non_exhaustive]`** : c'est le daemon, dans un autre crate, qui construit
/// ce contexte a chaque admission. Le marquer non exhaustif le rendrait
/// inconstructible hors de `trame-core` — ajouter un champ ici est donc un changement
/// cassant assume, et c'est le bon compromis pour une structure de passage.
#[derive(Debug, Clone, Copy)]
pub struct SessionContext<'a> {
    /// La session concernee.
    pub session: &'a Session,
    /// Son projet.
    pub project: &'a Project,
    /// L'instant courant, fourni par l'horloge injectee. Un contributeur ne lit
    /// jamais l'heure lui-meme, sinon il devient intestable.
    pub now: Timestamp,
    /// Le verdict rendu par la derniere admission, s'il y en a eu une.
    pub last_verdict: Option<&'a Verdict>,
    /// Le fichier que la session s'apprete a ecrire.
    pub pending_write: Option<&'a Path>,
}

/// Un morceau de prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptFragment {
    /// Qui l'a produit. Journalise, pour qu'on puisse repondre a « pourquoi
    /// l'agent a-t-il vu ca ».
    pub source: &'static str,
    /// L'ordre d'assemblage. Plus petit passe en premier.
    pub priority: u8,
    /// Le texte injecte.
    pub body: String,
}

/// Un contributeur au prompt.
///
/// Pas de `async` : composer un prompt est un calcul sur des donnees deja en
/// memoire. Un contributeur qui aurait besoin d'une I/O est un contributeur mal
/// place — c'est a l'appelant d'avoir deja recupere ce qu'il faut dans le
/// [`SessionContext`].
pub trait PromptContributor: Send + Sync {
    /// Le nom du contributeur, pour le journal.
    fn name(&self) -> &'static str;

    /// Sa contribution, ou `None` s'il n'a rien a dire.
    ///
    /// **`None` est le cas normal.** 95 % du trafic est propre et doit passer
    /// sans un mot.
    fn contribute(&self, ctx: &SessionContext<'_>) -> Option<PromptFragment>;
}

/// Assemble les contributions dans l'ordre.
#[derive(Default)]
pub struct PromptPipeline {
    contributors: Vec<Box<dyn PromptContributor>>,
}

impl PromptPipeline {
    /// Un pipeline vide.
    #[must_use]
    pub fn new() -> Self {
        Self {
            contributors: Vec::new(),
        }
    }

    /// Ajoute un contributeur en fin de chaine.
    #[must_use]
    pub fn with(mut self, contributor: impl PromptContributor + 'static) -> Self {
        self.contributors.push(Box::new(contributor));
        self
    }

    /// Les fragments a injecter, tries par priorite. Vide si personne n'a rien
    /// a dire.
    #[must_use]
    pub fn compose(&self, ctx: &SessionContext<'_>) -> Vec<PromptFragment> {
        let mut fragments: Vec<_> = self
            .contributors
            .iter()
            .filter_map(|c| c.contribute(ctx))
            .collect();
        fragments.sort_by_key(|fragment| fragment.priority);
        fragments
    }

    /// Le texte final, fragments joints par une ligne vide. `None` si rien a dire.
    #[must_use]
    pub fn render(&self, ctx: &SessionContext<'_>) -> Option<String> {
        let bodies: Vec<_> = self
            .compose(ctx)
            .into_iter()
            .map(|fragment| fragment.body)
            .collect();
        if bodies.is_empty() {
            None
        } else {
            Some(bodies.join("\n\n"))
        }
    }
}

impl std::fmt::Debug for PromptPipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PromptPipeline")
            .field(
                "contributors",
                &self
                    .contributors
                    .iter()
                    .map(|c| c.name())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// Le contributeur qui informe l'agent qu'une de ses lectures est perimee.
///
/// # C'est le format du message qui compte
///
/// Le code autour est de la plomberie ; **ce texte est la variable a iterer**.
/// Il doit rester neutre, factuel, actionnable : pas d'ordre, pas d'alarme. Un
/// agent a qui on crie dessus ne se comporte pas mieux, et un utilisateur a qui
/// on crie dessus desactive la fonctionnalite.
#[derive(Debug, Clone, Copy, Default)]
pub struct StaleReadNotice;

impl PromptContributor for StaleReadNotice {
    fn name(&self) -> &'static str {
        "stale_read_notice"
    }

    fn contribute(&self, ctx: &SessionContext<'_>) -> Option<PromptFragment> {
        let Some(Verdict::StaleRead { stale }) = ctx.last_verdict else {
            return None;
        };
        if stale.is_empty() {
            return None;
        }

        let mut body = String::new();
        for file in stale {
            let ago = humanize(ctx.now - file.read_at);
            body.push_str(&format!(
                "[Trame] {} a ete modifie par la session « {} »\n        \
                 apres que tu l'aies lu (il y a {}).\n        \
                 Relis-le avant de continuer si ton travail en depend.\n",
                file.path.display(),
                file.last_writer_name,
                ago,
            ));
        }

        Some(PromptFragment {
            source: self.name(),
            priority: 10,
            body: body.trim_end().to_owned(),
        })
    }
}

/// Une duree, en francais, arrondie a l'unite utile.
///
/// « il y a 2 min » est actionnable. « il y a 127,4 s » ne l'est pas.
fn humanize(delta: TimeDelta) -> String {
    let seconds = delta.num_seconds().max(0);
    match seconds {
        0..=44 => "quelques secondes".to_owned(),
        45..=5399 => {
            let minutes = (seconds + 30) / 60;
            format!("{} min", minutes.max(1))
        }
        _ => {
            let hours = (seconds + 1800) / 3600;
            format!("{hours} h")
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::{TimeDelta, Utc};

    use super::*;
    use crate::ids::{ProjectId, Seq, SessionId};
    use crate::project::Toolchain;
    use crate::session::{BranchTarget, Harness, SessionState};
    use crate::{BranchName, StaleFile};

    fn fixture() -> (Project, Session) {
        let now = Utc::now();
        let project = Project {
            id: ProjectId::new(),
            path: PathBuf::from("/Users/x/dev/portailfcd"),
            name: "portailfcd".into(),
            toolchain: Toolchain::Cargo,
            added_at: now,
            last_opened_at: Some(now),
        };
        let session = Session {
            id: SessionId::new(),
            project_id: project.id,
            name: "ajout-handlers".into(),
            harness: Harness::ClaudeCode,
            target_branch: BranchTarget::New(BranchName::new("feat/handlers")),
            work_item: None,
            state: SessionState::Writing,
            created_at: now,
        };
        (project, session)
    }

    #[test]
    fn rien_a_dire_quand_c_est_propre() {
        let (project, session) = fixture();
        let ctx = SessionContext {
            session: &session,
            project: &project,
            now: Utc::now(),
            last_verdict: Some(&Verdict::Clean),
            pending_write: None,
        };
        let pipeline = PromptPipeline::new().with(StaleReadNotice);
        assert_eq!(
            pipeline.render(&ctx),
            None,
            "un verdict propre doit etre silencieux"
        );
    }

    /// Fabrique un fichier perime, lu il y a `read_ago`.
    fn stale_file(path: &str, writer_name: &str, read_ago: TimeDelta, now: Timestamp) -> StaleFile {
        StaleFile {
            path: PathBuf::from(path),
            last_writer: SessionId::new(),
            last_writer_name: writer_name.to_owned(),
            read_at: now - read_ago,
            written_at: now - TimeDelta::seconds(30),
            seq: Seq::FIRST,
        }
    }

    // Les deux tests qui suivent verifient la **structure** de l'avis — que les trois
    // faits actionnables y sont — et jamais sa prose.
    //
    // Le texte du message est la variable qu'on iterera le plus : c'est lui qui decide
    // si l'agent relit et s'adapte. Un test qui epinglerait les mots ferait echouer la
    // suite a chaque ajustement de formulation, donc decouragerait precisement les
    // ajustements qu'on veut encourager. Le delai est compare a la sortie de
    // [`humanize`] plutot qu'a un litteral, pour que changer le format des durees ne
    // casse pas ces tests non plus — c'est le role de `les_delais_sont_lisibles`.

    #[test]
    fn l_avis_porte_le_fichier_la_session_et_le_delai() {
        let (project, session) = fixture();
        let now = Utc::now();
        let read_ago = TimeDelta::minutes(2);
        let verdict = Verdict::StaleRead {
            stale: vec![stale_file("auth.rs", "refacto-api", read_ago, now)],
        };
        let ctx = SessionContext {
            session: &session,
            project: &project,
            now,
            last_verdict: Some(&verdict),
            pending_write: Some(Path::new("handlers.rs")),
        };

        let fragments = PromptPipeline::new().with(StaleReadNotice).compose(&ctx);
        assert_eq!(
            fragments.len(),
            1,
            "un seul contributeur a quelque chose a dire"
        );

        let fragment = &fragments[0];
        assert_eq!(
            fragment.source, "stale_read_notice",
            "le fragment doit etre attribuable"
        );

        let body = &fragment.body;
        assert!(
            body.contains("auth.rs"),
            "le chemin du fichier perime doit apparaitre : {body}"
        );
        assert!(
            body.contains("refacto-api"),
            "le nom de la session qui a ecrit doit apparaitre : {body}"
        );
        assert!(
            body.contains(&humanize(read_ago)),
            "le delai ecoule doit apparaitre : {body}"
        );
    }

    #[test]
    fn chaque_fichier_perime_est_mentionne() {
        let (project, session) = fixture();
        let now = Utc::now();
        let verdict = Verdict::StaleRead {
            stale: vec![
                stale_file("auth.rs", "refacto-api", TimeDelta::minutes(2), now),
                stale_file("db/pool.rs", "migration-sqlx", TimeDelta::hours(1), now),
            ],
        };
        let ctx = SessionContext {
            session: &session,
            project: &project,
            now,
            last_verdict: Some(&verdict),
            pending_write: Some(Path::new("handlers.rs")),
        };

        let body = PromptPipeline::new()
            .with(StaleReadNotice)
            .render(&ctx)
            .unwrap();
        for expected in ["auth.rs", "refacto-api", "db/pool.rs", "migration-sqlx"] {
            assert!(
                body.contains(expected),
                "{expected} doit apparaitre dans l'avis : {body}"
            );
        }
    }

    // Ici, en revanche, epingler la sortie est legitime : `humanize` est une fonction
    // pure dont le contrat *est* la forme rendue. C'est ce test qui porte le format des
    // durees, ce qui permet aux deux precedents de ne pas s'en occuper.
    #[test]
    fn les_delais_sont_lisibles() {
        assert_eq!(humanize(TimeDelta::seconds(3)), "quelques secondes");
        assert_eq!(humanize(TimeDelta::seconds(120)), "2 min");
        assert_eq!(humanize(TimeDelta::minutes(89)), "89 min");
        assert_eq!(humanize(TimeDelta::hours(3)), "3 h");
        assert_eq!(humanize(TimeDelta::seconds(-5)), "quelques secondes");
    }
}
