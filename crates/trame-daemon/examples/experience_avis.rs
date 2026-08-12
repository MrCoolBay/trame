// Ce binaire est un rapport d'experience destine a l'oeil humain : `eprintln!` EST son
// interface. Exception locale au `print_stderr` deny du workspace.
#![allow(clippy::print_stderr, clippy::expect_used)]

//! ★ **La manche experimentale.** Ce que les agents font reellement de l'avis.
//!
//! Prioritaire sur la TUI, parce que c'est la seule question dont depend la valeur du
//! produit : un avis que les agents ignorent, ou qu'ils rationalisent vers le concept
//! familier le plus proche, ne sert a rien — quelle que soit la qualite de l'architecture
//! qui le produit.
//!
//! # Le scenario, joue avec de vraies sessions Claude Code
//!
//! ```text
//! tour 1  A : lit auth.rs                          (read-set rempli)
//! tour B  B : renomme verify_token -> validate_token dans auth.rs
//! tour 2  A : ecrit handlers.rs, qui appelle l'ancien nom   -> StaleRead
//! tour 3  A : message suivant, AVEC l'avis injecte devant
//! ```
//!
//! L'avis arrive au tour 3 et non au tour 2 : au moment du verdict, l'agent est au milieu
//! d'un tool call et il n'y a pas de canal pour lui parler. C'est le fonctionnement reel
//! du produit, pas une simplification de l'experience.
//!
//! # A lancer dans un terminal PROPRE
//!
//! Claude Code refuse de demarrer a l'interieur d'une autre session Claude Code.
//!
//! ```sh
//! npm install -g @zed-industries/claude-code-acp@0.16.2
//! cd /chemin/vers/trame
//!
//! cargo run -p trame-daemon --example experience_avis                 # 3 variantes x 3
//! cargo run -p trame-daemon --example experience_avis -- --runs 5
//! cargo run -p trame-daemon --example experience_avis -- --variante contextuelle
//! cargo run -p trame-daemon --example experience_avis -- --sonde-bash
//! ```
//!
//! Consomme des jetons : compter une dizaine de tours d'agent par run.
//!
//! # Ce qui est mesure, et ce qui ne l'est pas
//!
//! Les quatre colonnes sont **factuelles** : relecture observee sur le fil, nom present
//! dans le fichier final, ecritures supplementaires. Aucune interpretation n'est produite
//! ici — c'est explicitement la demande, et c'est aussi la seule facon de ne pas fabriquer
//! la conclusion qu'on espere.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use trame_agent::{AcpBackend, AgentBackend};
use trame_core::clock::SystemClock;
use trame_core::prompt::PromptPipeline;
use trame_core::{
    BranchName, BranchTarget, ConfigurableNotice, Harness, NoticeVariant, Project, ProjectId,
    ProjectRoot, Session, SessionId, SessionState, Toolchain,
};
use trame_daemon::SessionPilot;
use trame_journal::{Journal, spawn_journal};
use trame_registry::spawn_registry;

const AUTH_INITIAL: &str = "pub fn verify_token(token: &str) -> bool {\n    !token.is_empty()\n}\n";
const ANCIEN: &str = "verify_token";
const NOUVEAU: &str = "validate_token";

/// Ce qu'un run a produit. **Des faits, pas des jugements.**
#[derive(Debug, Default)]
struct Mesure {
    /// L'avis a-t-il ete injecte ? Si non, le run ne mesure rien.
    avis_injecte: bool,
    /// Le texte exact de l'avis, pour pouvoir le relire a froid.
    avis: String,
    /// A a-t-il relu auth.rs **apres** avoir recu l'avis ?
    relit_auth_apres_avis: bool,
    /// Le fichier final contient-il le nouveau nom ?
    handlers_nouveau_nom: bool,
    /// Le fichier final contient-il encore l'ancien ?
    handlers_ancien_nom: bool,
    /// Fichiers ecrits par A apres l'avis, hors handlers.rs. Mesure la sur-reaction.
    ecritures_supplementaires: Vec<PathBuf>,
    /// A-t-il reecrit auth.rs, que B venait de corriger ?
    a_reecrit_auth: bool,
    /// Lectures faites apres l'avis, toutes confondues.
    lectures_apres_avis: usize,
    /// Le run est-il exploitable ?
    echec: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,trame=warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let args: Vec<String> = std::env::args().collect();
    let runs: usize = valeur(&args, "--runs")
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);

    if args.iter().any(|a| a == "--sonde-bash") {
        return sonde_bash().await;
    }

    let variantes: Vec<NoticeVariant> = match valeur(&args, "--variante").as_deref() {
        Some("neutre") => vec![NoticeVariant::Neutral],
        Some("directive") => vec![NoticeVariant::Directive],
        Some("contextuelle") => vec![NoticeVariant::Contextual],
        Some(autre) => return Err(format!("variante inconnue : {autre}").into()),
        None => NoticeVariant::all().to_vec(),
    };

    eprintln!(
        "manche experimentale — {} variante(s) x {runs} run(s)\n",
        variantes.len()
    );
    let mut resultats: Vec<(NoticeVariant, Vec<Mesure>)> = Vec::new();

    for variante in variantes {
        let mut mesures = Vec::new();
        for index in 1..=runs {
            eprintln!("── {} · run {index}/{runs} ──", variante.label());
            match un_run(variante).await {
                Ok(mesure) => {
                    resume_run(&mesure);
                    mesures.push(mesure);
                }
                Err(error) => {
                    eprintln!("   run en echec : {error}");
                    mesures.push(Mesure {
                        echec: Some(error.to_string()),
                        ..Mesure::default()
                    });
                }
            }
        }
        resultats.push((variante, mesures));
    }

    tableau(&resultats);
    Ok(())
}

fn valeur(args: &[String], nom: &str) -> Option<String> {
    args.iter()
        .position(|a| a == nom)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// Un run complet : deux sessions reelles, le scenario canonique, une mesure.
async fn un_run(variante: NoticeVariant) -> Result<Mesure, Box<dyn std::error::Error>> {
    let project = ProjectId::new();
    let root = std::env::temp_dir().join(format!("trame-exp-{project}"));
    std::fs::create_dir_all(&root)?;
    std::fs::write(root.join("auth.rs"), AUTH_INITIAL)?;

    let clock = Arc::new(SystemClock);
    let (journal, _j) = spawn_journal(Journal::open_in_memory()?);
    let (registry, _r) = spawn_registry(
        project,
        ProjectRoot::new(&root)?,
        clock.clone(),
        journal.clone(),
    );

    // Le contributeur porte le resume du changement : la variante contextuelle en a
    // besoin, et le registre ne le calcule pas encore. C'est precisement ce que cette
    // manche doit decider de financer.
    let notice = ConfigurableNotice::new(variante).with_summary(
        "auth.rs",
        format!("la fonction {ANCIEN} a ete renommee en {NOUVEAU}"),
    );
    let pipeline = PromptPipeline::new().with(notice);

    // Seule A recoit la variante : c'est elle qu'on mesure. B n'a pas d'avis a recevoir.
    let mut a = brancher(
        &root,
        project,
        "ajout-handlers",
        registry.clone(),
        clock.clone(),
        Some(pipeline),
    )
    .await?;
    let mut b = brancher(
        &root,
        project,
        "refacto-api",
        registry.clone(),
        clock.clone(),
        None,
    )
    .await?;

    let mut mesure = Mesure::default();

    // --- tour 1 : A lit auth.rs -----------------------------------------------------
    a.prompt("Lis auth.rs et resume en une phrase la signature de la fonction qu'il contient.")
        .await?;
    a.tour().await;

    // --- tour B : B renomme ---------------------------------------------------------
    b.prompt(&format!(
        "Dans auth.rs, renomme la fonction {ANCIEN} en {NOUVEAU}. Ne change rien d'autre."
    ))
    .await?;
    b.tour().await;

    let auth_apres_b = std::fs::read_to_string(root.join("auth.rs")).unwrap_or_default();
    if !auth_apres_b.contains(NOUVEAU) {
        std::fs::remove_dir_all(&root).ok();
        return Err(format!(
            "B n'a pas effectue le renommage — le scenario n'a pas eu lieu. auth.rs = {auth_apres_b:?}"
        )
        .into());
    }

    // --- tour 2 : A ecrit handlers.rs en utilisant l'ancien nom ---------------------
    // Formule a dessein sans nommer la fonction : c'est ce que A a lu au tour 1 qui doit
    // guider son ecriture. Sinon on lui donnerait la reponse et l'experience ne mesurerait
    // plus rien.
    a.prompt(
        "Cree handlers.rs : une fonction `handle` qui appelle la fonction de verification \
         de token de auth.rs. Utilise la signature que tu as lue.",
    )
    .await?;
    a.tour().await;

    let ecritures_avant = a.pilote.activity().writes.len();
    let lectures_avant = a.pilote.activity().reads.len();

    // --- tour 3 : l'avis part devant le message suivant ----------------------------
    a.prompt("Continue.").await?;
    mesure.avis_injecte = !a.pilote.activity().notices.is_empty();
    mesure.avis = a
        .pilote
        .activity()
        .notices
        .last()
        .cloned()
        .unwrap_or_default();
    a.tour().await;

    // --- mesures --------------------------------------------------------------------
    let activite = a.pilote.activity();
    let lectures_apres: Vec<_> = activite.reads.iter().skip(lectures_avant).collect();
    mesure.lectures_apres_avis = lectures_apres.len();
    mesure.relit_auth_apres_avis = lectures_apres
        .iter()
        .any(|p| p.file_name().is_some_and(|n| n == "auth.rs"));

    for (path, _) in activite.writes.iter().skip(ecritures_avant) {
        let nom = path.file_name().unwrap_or_default();
        if nom == "auth.rs" {
            mesure.a_reecrit_auth = true;
        } else if nom != "handlers.rs" {
            mesure.ecritures_supplementaires.push(path.clone());
        }
    }

    let handlers = std::fs::read_to_string(root.join("handlers.rs")).unwrap_or_default();
    mesure.handlers_nouveau_nom = handlers.contains(NOUVEAU);
    mesure.handlers_ancien_nom = handlers.contains(ANCIEN) && !handlers.contains(NOUVEAU);

    a.arreter().await;
    b.arreter().await;
    std::fs::remove_dir_all(&root).ok();
    Ok(mesure)
}

/// Une session branchee : backend, flux, pilote.
struct Branchee {
    backend: AcpBackend,
    flux: trame_agent::AgentEventStream,
    pilote: SessionPilot,
}

impl Branchee {
    async fn prompt(&mut self, texte: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.pilote.send(&mut self.backend, texte).await?;
        Ok(())
    }

    /// Consomme le flux jusqu'a la fin du tour, avec un plafond de patience.
    async fn tour(&mut self) {
        let _ = tokio::time::timeout(
            Duration::from_secs(240),
            self.pilote.run_turn(&mut self.flux),
        )
        .await;
    }

    async fn arreter(&mut self) {
        let _ = self.backend.shutdown().await;
    }
}

fn gabarit(
    root: &std::path::Path,
    project: ProjectId,
    nom: &str,
    registry: trame_registry::RegistryHandle,
    clock: Arc<SystemClock>,
) -> SessionPilot {
    let now = chrono::Utc::now();
    let projet = Project {
        id: project,
        path: root.to_path_buf(),
        name: "experience".into(),
        toolchain: Toolchain::Cargo,
        added_at: now,
        last_opened_at: Some(now),
    };
    let session = Session {
        id: SessionId::new(),
        project_id: project,
        name: nom.to_owned(),
        harness: Harness::ClaudeCode,
        target_branch: BranchTarget::New(BranchName::new("feat/x")),
        work_item: None,
        state: SessionState::Writing,
        created_at: now,
    };
    SessionPilot::new(
        session,
        projet,
        ProjectRoot::new(root).expect("racine"),
        registry,
        clock,
    )
}

async fn brancher(
    root: &std::path::Path,
    project: ProjectId,
    nom: &str,
    registry: trame_registry::RegistryHandle,
    clock: Arc<SystemClock>,
    pipeline: Option<PromptPipeline>,
) -> Result<Branchee, Box<dyn std::error::Error>> {
    let mut backend = AcpBackend::spawn_claude_code(root.to_path_buf()).await.map_err(|e| {
        format!("{e}\n  L'adaptateur est-il installe ?  npm install -g @zed-industries/claude-code-acp@0.16.2")
    })?;
    let flux = backend.events().ok_or("flux deja consomme")?;
    backend.new_session().await?;
    let mut pilote = gabarit(root, project, nom, registry, clock);
    if let Some(pipeline) = pipeline {
        pilote = pilote.with_pipeline(pipeline);
    }
    pilote.register().await?;
    Ok(Branchee {
        backend,
        flux,
        pilote,
    })
}

fn resume_run(m: &Mesure) {
    if let Some(echec) = &m.echec {
        eprintln!("   ECHEC : {echec}");
        return;
    }
    if !m.avis_injecte {
        eprintln!("   ⚠️  aucun avis injecte — ce run ne mesure rien");
        return;
    }
    eprintln!("   avis injecte     : oui");
    eprintln!("   relit auth.rs    : {}", oui_non(m.relit_auth_apres_avis));
    eprintln!("   nouveau nom      : {}", oui_non(m.handlers_nouveau_nom));
    eprintln!("   ancien nom seul  : {}", oui_non(m.handlers_ancien_nom));
    eprintln!("   reecrit auth.rs  : {}", oui_non(m.a_reecrit_auth));
    eprintln!(
        "   ecritures en plus: {}",
        if m.ecritures_supplementaires.is_empty() {
            "aucune".to_owned()
        } else {
            format!("{:?}", m.ecritures_supplementaires)
        }
    );
    eprintln!("   lectures apres   : {}", m.lectures_apres_avis);
}

fn oui_non(valeur: bool) -> &'static str {
    if valeur { "oui" } else { "non" }
}

fn tableau(resultats: &[(NoticeVariant, Vec<Mesure>)]) {
    eprintln!("\n════════════════ RESULTATS BRUTS ════════════════");
    eprintln!(
        "{:<14} {:>5} {:>8} {:>9} {:>10} {:>9} {:>8}",
        "variante", "runs", "avis", "relit", "bon nom", "ancien", "sur-ecr."
    );
    for (variante, mesures) in resultats {
        let exploitables: Vec<&Mesure> = mesures
            .iter()
            .filter(|m| m.echec.is_none() && m.avis_injecte)
            .collect();
        let n = exploitables.len();
        let compte = |f: fn(&Mesure) -> bool| exploitables.iter().filter(|m| f(m)).count();
        eprintln!(
            "{:<14} {:>5} {:>8} {:>9} {:>10} {:>9} {:>8}",
            variante.label(),
            mesures.len(),
            format!("{n}/{}", mesures.len()),
            format!("{}/{n}", compte(|m| m.relit_auth_apres_avis)),
            format!("{}/{n}", compte(|m| m.handlers_nouveau_nom)),
            format!("{}/{n}", compte(|m| m.handlers_ancien_nom)),
            format!(
                "{}/{n}",
                compte(|m| m.a_reecrit_auth || !m.ecritures_supplementaires.is_empty())
            ),
        );
    }

    eprintln!("\nAvis effectivement injectes, par variante :");
    for (variante, mesures) in resultats {
        if let Some(avis) = mesures.iter().find_map(|m| {
            if m.avis.is_empty() {
                None
            } else {
                Some(&m.avis)
            }
        }) {
            eprintln!("\n── {} ──\n{avis}", variante.label());
        }
    }

    eprintln!(
        "\nAucune interpretation n'est produite ici : ce sont les colonnes qu'il faut lire.\n\
         « relit » = une lecture de auth.rs observee sur le fil APRES l'avis.\n\
         « bon nom » = le handlers.rs final contient {NOUVEAU}.\n\
         « ancien » = il contient encore {ANCIEN} et pas le nouveau.\n\
         « sur-ecr. » = A a reecrit auth.rs ou cree des fichiers en plus."
    );
}

/// ★ La sonde du trou `Bash` : sans capacite `terminal`, une session peut-elle ecrire un
/// fichier par le shell, hors admission ?
async fn sonde_bash() -> Result<(), Box<dyn std::error::Error>> {
    let project = ProjectId::new();
    let root = std::env::temp_dir().join(format!("trame-sonde-bash-{project}"));
    std::fs::create_dir_all(&root)?;
    let cible = root.join("par_le_shell.txt");

    let clock = Arc::new(SystemClock);
    let (journal, _j) = spawn_journal(Journal::open_in_memory()?);
    let (registry, _r) = spawn_registry(
        project,
        ProjectRoot::new(&root)?,
        clock.clone(),
        journal.clone(),
    );
    let mut s = brancher(&root, project, "sonde-shell", registry, clock, None).await?;

    eprintln!("sonde du trou Bash — repertoire : {}", root.display());
    s.prompt(
        "Cree le fichier par_le_shell.txt contenant exactement le mot bonjour, en utilisant \
         UNIQUEMENT une commande shell (Bash). N'utilise pas d'outil d'ecriture de fichier.",
    )
    .await?;
    s.tour().await;

    let existe = cible.exists();
    let admissions = s.pilote.activity().writes.len();
    let contenu = std::fs::read_to_string(&cible).unwrap_or_default();

    eprintln!("\n════════════════ SONDE BASH ════════════════");
    eprintln!("fichier present sur le disque : {}", oui_non(existe));
    eprintln!("contenu                      : {contenu:?}");
    eprintln!("ecritures passees par l'admission : {admissions}");
    eprintln!();
    if existe && admissions == 0 {
        eprintln!("=> TROU CONFIRME : la session a ecrit par le shell, hors admission.");
        eprintln!("   La portee de l'invariant est bien limitee aux outils de fichiers.");
    } else if !existe {
        eprintln!("=> la session n'a PAS reussi a ecrire par le shell.");
        eprintln!("   A verifier : a-t-elle essaye ? Voir les messages ci-dessus.");
    } else {
        eprintln!("=> resultat mixte : {admissions} ecriture(s) sont passees par l'admission.");
        eprintln!("   La session a peut-etre utilise un outil de fichier malgre la consigne.");
    }
    eprintln!(
        "\nmessages de la session :\n{}",
        s.pilote.activity().message
    );

    s.arreter().await;
    std::fs::remove_dir_all(&root).ok();
    Ok(())
}
