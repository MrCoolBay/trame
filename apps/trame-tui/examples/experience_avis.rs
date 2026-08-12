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
//! cargo run -p trame-tui --example experience_avis                 # 3 variantes x 3
//! cargo run -p trame-tui --example experience_avis -- --runs 5
//! cargo run -p trame-tui --example experience_avis -- --variante contextuelle
//! cargo run -p trame-tui --example experience_avis -- --sonde-bash
//!
//! cargo run -p trame-tui --example experience_avis -- --tui     # ★ regarde en direct
//! ```
//!
//! # Pourquoi cet exemple vit sous `apps/trame-tui`
//!
//! Parce que `--tui` a besoin de l'interface, et que la direction de dependance est un
//! invariant sans exception : `core <- journal <- registry <- {agent, vcs} <- daemon <- tui`.
//! Faire dependre `trame-daemon` de `trame-tui`, meme en dev-dependency, inverserait cette
//! fleche pour la seule commodite de garder le fichier a sa place. Ici, au sommet de la
//! chaine, l'exemple atteint tout ce dont il a besoin sans rien inverser.
//!
//! # Le mode `--tui`
//!
//! Un seul run, une seule variante, et **l'ecran plutot que le tableau** : la lecture de
//! `auth.rs` par A, l'ecriture de B, le `StaleRead` de A marque `▲`, l'avis injecte. Le
//! projet vit dans un repertoire fixe — pendant que ca tourne :
//!
//! ```sh
//! echo '// ajoute a la main' >> /tmp/trame-experience-live/notes.txt
//! ```
//!
//! La ligne apparait en **hors-bande, sans verdict** : le watcher l'a constatee apres coup,
//! personne ne l'a admise, et l'interface ne doit pas laisser croire l'inverse.
//!
//! L'interface reste ouverte apres la fin du run — `q` pour sortir. Le resume s'imprime
//! **apres** la restauration du terminal, et l'arbre produit reste sur le disque.
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
use trame_daemon::{Observer, SessionPilot, Transport, TurnOutcome, observe_channel};
use trame_journal::{Journal, spawn_journal};
use trame_registry::spawn_registry;
use trame_tui::app::App;
use trame_tui::run;

/// La racine du mode `--tui`. **Fixe a dessein.**
///
/// L'interet du mode direct est de pouvoir ecrire dans le projet a la main pendant que ca
/// tourne, pour voir l'ecriture apparaitre en hors-bande. Une racine tiree au hasard
/// obligerait a recopier un chemin lu a l'ecran ; celle-ci se tape de memoire.
const RACINE_DIRECTE: &str = "/tmp/trame-experience-live";

const AUTH_INITIAL: &str = "pub fn verify_token(token: &str) -> bool {\n    !token.is_empty()\n}\n";
const ANCIEN: &str = "verify_token";
const NOUVEAU: &str = "validate_token";

/// ★ Les outils fermes pendant la manche, **en plus** de ceux que l'adaptateur retire.
///
/// Sans ca, la manche mesure du vide. Retirer `Read` ne force pas l'agent a passer par
/// nous : `Grep`, `Glob` et `Bash` restent disponibles, et un agent qui lit par l'un d'eux
/// **n'entre pas dans le read-set**. Sans entree de read-set, aucun `StaleRead` n'est
/// possible : l'avis ne se declenche jamais et rien ne le signale.
///
/// C'est exactement ce qui a bloque la premiere manche : les agents ont travaille — quatre
/// `tool_call_update` en temoignaient — mais aucune lecture n'est remontee au registre.
///
/// Fermer ces outils n'est pas une triche, c'est un **controle experimental** : on veut
/// mesurer l'effet de l'avis, pas le choix d'outil de l'agent. Le trou reste ouvert dans
/// le produit, et il est nomme dans l'ADR 0016.
const OUTILS_FERMES: &[&str] = &[
    "Grep",
    "Glob",
    "Bash",
    "BashOutput",
    "KillShell",
    "Task",
    "WebFetch",
    "WebSearch",
];

/// Duree d'un tour au-dela de laquelle on abandonne, par defaut.
const TIMEOUT_TOUR_PAR_DEFAUT: u64 = 60;

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
    let args: Vec<String> = std::env::args().collect();
    let mode_direct = args.iter().any(|a| a == "--tui");

    // ★ Les logs ne peuvent pas rester sur stderr en mode direct : ils s'ecriraient
    // par-dessus le rendu. Ils vont dans un fichier, plus bavards que d'habitude — c'est
    // par la qu'on diagnostique un run qui n'avance pas, puisque l'ecran, lui, montre le
    // produit et pas la mecanique.
    let journal_logs = PathBuf::from(format!("/tmp/trame-experience-{}.log", std::process::id()));
    let filtre = || {
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            if mode_direct {
                "warn,trame=info".into()
            } else {
                "warn,trame=warn".into()
            }
        })
    };
    match (mode_direct, std::fs::File::create(&journal_logs)) {
        (true, Ok(fichier)) => tracing_subscriber::fmt()
            .with_env_filter(filtre())
            .with_ansi(false)
            .with_writer(Arc::new(fichier))
            .init(),
        // Fichier impossible : on se TAIT plutot que de corrompre l'affichage. Perdre les
        // logs est genant ; rendre l'ecran illisible rend le mode direct inutile.
        (true, Err(_)) => tracing_subscriber::fmt()
            .with_env_filter(filtre())
            .with_writer(std::io::sink)
            .init(),
        (false, _) => tracing_subscriber::fmt()
            .with_env_filter(filtre())
            .with_writer(std::io::stderr)
            .init(),
    }

    let runs: usize = valeur(&args, "--runs")
        .and_then(|v| v.parse().ok())
        .unwrap_or(3);
    let timeout_tour = Duration::from_secs(
        valeur(&args, "--timeout-tour")
            .and_then(|v| v.parse().ok())
            .unwrap_or(TIMEOUT_TOUR_PAR_DEFAUT),
    );
    // Plafond global : une manche qui derape ne doit pas tourner toute la nuit. Par
    // defaut, de quoi tenir tous les tours prevus plus une marge.
    let timeout_global = Duration::from_secs(
        valeur(&args, "--timeout-global")
            .and_then(|v| v.parse().ok())
            .unwrap_or(timeout_tour.as_secs() * 4 * 3 * (runs as u64) + 300),
    );

    if args.iter().any(|a| a == "--sonde-bash") {
        return sonde_bash(timeout_tour).await;
    }

    if mode_direct {
        let variante = match valeur(&args, "--variante").as_deref() {
            Some("directive") => NoticeVariant::Directive,
            Some("contextuelle") => NoticeVariant::Contextual,
            // La forme canonique, tranchee par l'ADR 0018.
            _ => NoticeVariant::Neutral,
        };
        return manche_directe(variante, timeout_tour, &journal_logs).await;
    }

    let variantes: Vec<NoticeVariant> = match valeur(&args, "--variante").as_deref() {
        Some("neutre") => vec![NoticeVariant::Neutral],
        Some("directive") => vec![NoticeVariant::Directive],
        Some("contextuelle") => vec![NoticeVariant::Contextual],
        Some(autre) => return Err(format!("variante inconnue : {autre}").into()),
        None => NoticeVariant::all().to_vec(),
    };

    eprintln!(
        "manche experimentale — {} variante(s) x {runs} run(s)\n\
         timeout par tour : {} s · plafond global : {} s\n\
         outils fermes pendant la manche : {OUTILS_FERMES:?}\n",
        variantes.len(),
        timeout_tour.as_secs(),
        timeout_global.as_secs(),
    );

    let manche = async {
        let mut resultats: Vec<(NoticeVariant, Vec<Mesure>)> = Vec::new();
        for variante in variantes {
            let mut mesures = Vec::new();
            for index in 1..=runs {
                eprintln!("── {} · run {index}/{runs} ──", variante.label());
                match un_run(variante, timeout_tour, None).await {
                    Ok(mesure) => {
                        resume_run(&mesure);
                        mesures.push(mesure);
                    }
                    Err(error) => {
                        // Un run qui echoue est compte non exploitable, il n'arrete pas
                        // la manche : les autres runs ont encore quelque chose a dire.
                        eprintln!("   NON EXPLOITABLE : {error}");
                        mesures.push(Mesure {
                            echec: Some(error.to_string()),
                            ..Mesure::default()
                        });
                    }
                }
            }
            resultats.push((variante, mesures));
        }
        resultats
    };

    match tokio::time::timeout(timeout_global, manche).await {
        Ok(resultats) => tableau(&resultats),
        Err(_) => {
            eprintln!(
                "\n⚠️  PLAFOND GLOBAL ATTEINT ({} s) — manche interrompue, resultats partiels \
                 perdus.\n   Relancer avec --timeout-global plus large, ou --runs plus petit.",
                timeout_global.as_secs()
            );
        }
    }
    Ok(())
}

fn valeur(args: &[String], nom: &str) -> Option<String> {
    args.iter()
        .position(|a| a == nom)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// ★ Le scenario canonique **regarde en direct**, avec deux vraies sessions Claude Code.
///
/// Un seul run, une seule variante : l'objet n'est pas de mesurer mais de **voir**. La
/// lecture de `auth.rs` par A, l'ecriture de B, le `StaleRead` de A marque `▲`, l'avis
/// injecte — et une ecriture faite a la main dans un autre terminal qui apparait en
/// hors-bande, sans verdict.
///
/// # Trois contraintes que le mode direct impose
///
/// 1. **Le terminal avant le projet.** Une interface qui ne peut pas s'afficher ne doit
///    rien avoir touche. Meme regle que le binaire.
/// 2. **Les logs quittent stderr.** `stdout` appartient au terminal alternatif, et `stderr`
///    s'afficherait par-dessus le rendu. Ils vont dans un fichier, dont le chemin est
///    rappele a la sortie.
/// 3. **Le rapport s'imprime apres restauration.** Un tableau ecrit dans le terminal
///    alternatif est un tableau que personne ne lit.
async fn manche_directe(
    variante: NoticeVariant,
    timeout_tour: Duration,
    journal_logs: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let (observer, mut observations) = observe_channel();

    // ★ Le terminal AVANT le projet. Une interface qui ne peut pas s'afficher ne doit rien
    // avoir touche — meme regle que le binaire, et pour la meme raison.
    let mut terminal = ratatui::try_init().map_err(|e| format!("terminal indisponible : {e}"))?;

    // Un depart propre : des fichiers restes d'un run precedent seraient constates par le
    // watcher et affiches comme hors-bande, ce qui serait vrai mais deroutant.
    std::fs::remove_dir_all(RACINE_DIRECTE).ok();
    let socle = match Socle::nouveau(PathBuf::from(RACINE_DIRECTE)) {
        Ok(socle) => socle,
        Err(error) => {
            ratatui::try_restore().ok();
            return Err(error);
        }
    };
    // Le watcher vit ici, pas dans le run : c'est lui qui fera apparaitre l'ecriture faite
    // a la main, y compris apres la fin ou l'echec du run.
    let _watcher = match trame_daemon::spawn_watcher_observed(
        ProjectRoot::new(&socle.root)?,
        socle.registry.clone(),
        Some(observer.clone()),
    ) {
        Ok(watcher) => watcher,
        Err(error) => {
            ratatui::try_restore().ok();
            return Err(error.into());
        }
    };
    let mut etat = App::new(
        format!("{RACINE_DIRECTE}  (ecris dedans depuis un autre terminal)"),
        Arc::new(SystemClock),
    );
    let mut touches = run::spawn_touches();

    // Le run tourne derriere l'affichage. On garde son resultat pour l'imprimer apres.
    // L'erreur est reduite a une chaine : `Box<dyn Error>` n'est pas `Send`, et un run qui
    // echoue n'a de toute facon rien d'autre a transmettre que son motif.
    // Le run tourne pendant que l'affichage vit. `LocalSet` plutot que `spawn` : `Socle`
    // n'est pas `Send` a travers une reference, et rien ici n'a besoin d'un autre thread.
    let resultat = {
        let socle = &socle;
        let observer = &observer;
        async move {
            un_run(variante, timeout_tour, Some((socle, observer)))
                .await
                .map_err(|error| error.to_string())
        }
    };

    // Les deux tournent ensemble, et l'affichage decide de la fin : `q` sort meme si le run
    // n'a pas fini, et la fin du run ne ferme pas l'ecran.
    let mut run_termine = None;
    let rendu = {
        let affichage = run::afficher(&mut terminal, &mut etat, &mut observations, &mut touches);
        tokio::pin!(affichage);
        tokio::pin!(resultat);
        loop {
            tokio::select! {
                fin = &mut affichage => break fin,
                mesure = &mut resultat, if run_termine.is_none() => run_termine = Some(mesure),
            }
        }
    };
    ratatui::try_restore().ok();

    if let Err(error) = rendu {
        eprintln!("rendu interrompu : {error}");
    }
    eprintln!("\nlogs du run : {}", journal_logs.display());
    eprintln!("arbre produit : {RACINE_DIRECTE}  (a supprimer a la main)\n");

    match run_termine {
        Some(Ok(mesure)) => {
            eprintln!("── {} · run direct ──", variante.label());
            resume_run(&mesure);
        }
        Some(Err(motif)) => eprintln!("run NON EXPLOITABLE : {motif}"),
        // `q` avant la fin du run : on ne fabrique pas de resume pour un run inachieve.
        None => eprintln!("run encore en cours a la fermeture — aucun resume."),
    }
    Ok(())
}

/// Un run complet : deux sessions reelles, le scenario canonique, une mesure.
async fn un_run(
    variante: NoticeVariant,
    timeout_tour: Duration,
    direct: Option<(&Socle, &Observer)>,
) -> Result<Mesure, Box<dyn std::error::Error>> {
    // En mode direct le socle appartient a l'appelant, qui le garde vivant apres le run.
    let socle_local;
    let (socle, observer) = match direct {
        Some((socle, observer)) => (socle, Some(observer.clone())),
        None => {
            socle_local = Socle::nouveau(
                std::env::temp_dir().join(format!("trame-exp-{}", ProjectId::new())),
            )?;
            (&socle_local, None)
        }
    };
    let root = socle.root.clone();

    // Le contributeur porte le resume du changement : la variante contextuelle en a
    // besoin, et le registre ne le calcule pas encore. C'est precisement ce que cette
    // manche doit decider de financer.
    let notice = ConfigurableNotice::new(variante).with_summary(
        "auth.rs",
        format!("la fonction {ANCIEN} a ete renommee en {NOUVEAU}"),
    );
    let pipeline = PromptPipeline::new().with(notice);

    // Seule A recoit la variante : c'est elle qu'on mesure. B n'a pas d'avis a recevoir.
    let ctx = Contexte {
        root: root.clone(),
        project: socle.project,
        registry: socle.registry.clone(),
        clock: socle.clock.clone(),
        timeout_tour,
        ferme_les_outils: true,
        observer,
    };
    let mut a = brancher(&ctx, "ajout-handlers", Some(pipeline)).await?;
    let mut b = brancher(&ctx, "refacto-api", None).await?;

    let mut mesure = Mesure::default();

    // --- tour 1 : A lit auth.rs -----------------------------------------------------
    a.prompt(
        "Lis auth.rs avec l'outil de lecture de fichier, et resume en une phrase la \
         signature de la fonction qu'il contient.",
    )
    .await?;
    if a.tour("1-A-lit-auth").await.is_none() {
        return Err("tour 1 expire".into());
    }

    // ★ Verification de la condition reelle du tour 1. La manche ne mesure quelque chose
    // que si la lecture est ENTREE DANS LE READ-SET. Sans ca, aucun StaleRead n'est
    // possible plus loin, et les colonnes seraient remplies de zeros trompeurs.
    let lectures = a.pilote.activity().reads.clone();
    tracing::info!(
        ?lectures,
        "condition du tour 1 : la lecture est-elle enregistree ?"
    );
    if !lectures
        .iter()
        .any(|p| p.file_name().is_some_and(|n| n == "auth.rs"))
    {
        return Err(format!(
            "auth.rs n'est PAS entre dans le read-set (lectures vues : {lectures:?}). \
             L'agent a lu par un outil qui echappe a l'interception, ou n'a pas lu. \
             La manche ne peut rien mesurer dans cet etat."
        )
        .into());
    }
    tracing::info!("condition du tour 1 remplie : auth.rs est dans le read-set");

    // --- tour B : B renomme ---------------------------------------------------------
    b.prompt(&format!(
        "Dans auth.rs, renomme la fonction {ANCIEN} en {NOUVEAU}. Ne change rien d'autre."
    ))
    .await?;
    if b.tour("B-renomme").await.is_none() {
        return Err("tour de B expire".into());
    }

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
    if a.tour("2-A-ecrit-handlers").await.is_none() {
        return Err("tour 2 expire".into());
    }

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
    tracing::info!(
        avis_injecte = mesure.avis_injecte,
        "condition du tour 3 : l'avis a-t-il ete pose devant le prompt ?"
    );
    if a.tour("3-A-recoit-l-avis").await.is_none() {
        return Err("tour 3 expire".into());
    }

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
    // En mode direct on garde le repertoire : l'interface reste ouverte et on veut pouvoir
    // regarder l'arbre produit, et continuer a y ecrire a la main.
    if direct.is_none() {
        std::fs::remove_dir_all(&root).ok();
    }
    Ok(mesure)
}

/// Le projet et ses acteurs. **Il survit au run**, et c'est tout l'objet de ce type.
///
/// En mode direct, le watcher doit continuer a surveiller apres la fin — ou apres l'ECHEC —
/// du run : l'interface reste ouverte, et une ecriture faite a la main a ce moment-la doit
/// encore apparaitre. Quand le socle etait construit dans `un_run`, un `?` sur l'ouverture
/// de session le relachait, le watcher s'arretait, et plus rien n'etait constate. Trouve en
/// regardant l'ecran : la ligne hors-bande n'arrivait jamais.
struct Socle {
    root: PathBuf,
    project: ProjectId,
    registry: trame_registry::RegistryHandle,
    clock: Arc<SystemClock>,
    _acteurs: Vec<tokio::task::JoinHandle<()>>,
}

impl Socle {
    /// Cree le repertoire, la fixture, le journal et le registre.
    ///
    /// La fixture est ecrite **avant** que le watcher demarre, pour la meme raison qu'on
    /// nettoie la racine : `auth.rs` cree dans le dos du registre serait une vraie ecriture
    /// hors-bande, signalee a juste titre — et l'utilisateur verrait un `1 hors-bande` qu'il
    /// n'a pas provoque.
    fn nouveau(root: PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let project = ProjectId::new();
        std::fs::create_dir_all(&root)?;
        std::fs::write(root.join("auth.rs"), AUTH_INITIAL)?;
        let clock = Arc::new(SystemClock);
        let (journal, j) = spawn_journal(Journal::open_in_memory()?);
        let (registry, r) =
            spawn_registry(project, ProjectRoot::new(&root)?, clock.clone(), journal);
        Ok(Self {
            root,
            project,
            registry,
            clock,
            _acteurs: vec![j, r],
        })
    }
}

/// Ce qui est commun a toutes les sessions d'un run.
///
/// Regroupe plutot que passe en sept parametres : sept arguments est le signe d'un
/// regroupement manquant, et `clippy.toml` le dit a six.
struct Contexte {
    root: PathBuf,
    project: ProjectId,
    registry: trame_registry::RegistryHandle,
    clock: Arc<SystemClock>,
    timeout_tour: Duration,
    /// Fermer les outils de lecture alternatifs. Faux uniquement pour la sonde Bash.
    ferme_les_outils: bool,
    /// Le canal vers l'interface, en mode direct. Aucun effet sur ce qui est mesure.
    observer: Option<Observer>,
}

/// Une session branchee : backend, flux, pilote.
struct Branchee {
    nom: &'static str,
    backend: AcpBackend,
    flux: trame_agent::AgentEventStream,
    pilote: SessionPilot,
    timeout_tour: Duration,
}

impl Branchee {
    async fn prompt(&mut self, texte: &str) -> Result<(), Box<dyn std::error::Error>> {
        tracing::info!(session = self.nom, "envoi du prompt");
        self.pilote.send(&mut self.backend, texte).await?;
        Ok(())
    }

    /// Consomme le flux jusqu'a la fin du tour, avec un plafond de patience.
    ///
    /// Un tour expire n'est **pas** un plantage : il rend `None`, et le run sera compte
    /// non exploitable. Une manche qui s'arrete au premier tour lent ne mesure rien.
    async fn tour(&mut self, etape: &str) -> Option<TurnOutcome> {
        tracing::info!(
            session = self.nom,
            etape,
            secondes = self.timeout_tour.as_secs(),
            "debut de tour — attente de la fin de tour (reponse a session/prompt)"
        );
        match tokio::time::timeout(self.timeout_tour, self.pilote.run_turn(&mut self.flux)).await {
            Ok(outcome) => {
                tracing::info!(session = self.nom, etape, ?outcome, "tour termine");
                Some(outcome)
            }
            Err(_) => {
                tracing::warn!(
                    session = self.nom,
                    etape,
                    secondes = self.timeout_tour.as_secs(),
                    "TOUR EXPIRE — run non exploitable"
                );
                None
            }
        }
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
    ctx: &Contexte,
    nom: &'static str,
    pipeline: Option<PromptPipeline>,
) -> Result<Branchee, Box<dyn std::error::Error>> {
    let mut backend = AcpBackend::spawn_claude_code(ctx.root.clone()).await.map_err(|e| {
        format!("{e}\n  L'adaptateur est-il installe ?  npm install -g @zed-industries/claude-code-acp@0.16.2")
    })?;
    let flux = backend.events().ok_or("flux deja consomme")?;
    // Avant `new_session` : la liste est fusionnee par l'adaptateur au moment ou il
    // construit la ligne de commande de l'agent.
    //
    // La sonde du trou Bash est la seule a ne rien fermer : c'est son objet meme.
    if ctx.ferme_les_outils {
        backend.disallow_tools(OUTILS_FERMES.iter().copied());
        tracing::info!(session = nom, outils_fermes = ?OUTILS_FERMES, "outils fermes");
    }
    backend.new_session().await?;
    tracing::info!(session = nom, "session ouverte");
    let mut pilote = gabarit(
        &ctx.root,
        ctx.project,
        nom,
        ctx.registry.clone(),
        ctx.clock.clone(),
    );
    if let Some(pipeline) = pipeline {
        pilote = pilote.with_pipeline(pipeline);
    }
    if let Some(observer) = ctx.observer.clone() {
        // Le transport se lit sur les capacites REELLES du backend. C'est le premier
        // affichage ou il vaut `ACP` sur une vraie session : la banniere de degradation ne
        // doit donc pas apparaitre, et si elle apparait c'est qu'on a menti quelque part.
        let transport = Transport::from(backend.capabilities());
        pilote = pilote.observed_by(observer, transport);
    }
    pilote.register().await?;
    Ok(Branchee {
        nom,
        backend,
        flux,
        pilote,
        timeout_tour: ctx.timeout_tour,
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
async fn sonde_bash(timeout_tour: Duration) -> Result<(), Box<dyn std::error::Error>> {
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
    // La sonde du trou Bash ne ferme evidemment PAS Bash : c'est son objet.
    let ctx = Contexte {
        root: root.clone(),
        project,
        registry,
        clock,
        timeout_tour,
        ferme_les_outils: false,
        // La sonde Bash n'a pas d'interface : elle imprime son constat et s'arrete.
        observer: None,
    };
    let mut s = brancher(&ctx, "sonde-shell", None).await?;

    eprintln!("sonde du trou Bash — repertoire : {}", root.display());
    s.prompt(
        "Cree le fichier par_le_shell.txt contenant exactement le mot bonjour, en utilisant \
         UNIQUEMENT une commande shell (Bash). N'utilise pas d'outil d'ecriture de fichier.",
    )
    .await?;
    s.tour("sonde-bash").await;

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
