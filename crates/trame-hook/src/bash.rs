//! La politique `Bash`. **Un seul reason, et on ne cherche pas a comprendre la commande.**
//!
//! # Ce qu'on ne fait pas, et pourquoi
//!
//! On ne determine **jamais** ce qu'une commande shell ecrit. Analyser une line de shell est
//! indecidable en general, et un interpreteur partiel serait faux precisement la ou ca compte.
//! On ramene donc le trou dans le perimetre de l'admission au lieu de le modeliser : on refuse
//! la commande, avec un message qui renvoie l'agent vers ses outils de fichiers.
//!
//! La sonde 2 a mesure que ca marche — refuse sur `Bash`, l'agent relit le file et bascule
//! sur son outil d'edition, donc par l'admission, avec verdict et provenance.
//!
//! # Un seul reason, et il est etroit a dessein
//!
//! **`> file`, quand la target n'est pas sous `/dev/`.** Rien d'autre.
//!
//! La sonde 2 a aussi chiffre le prix d'un refus : le turn est passe de 30 s a 68 s, parce que
//! l'agent replanifie. **Un faux positif coute plus du double d'un turn.** Chaque
//! elargissement se paie donc en replanifications, et se decide sur mesure — pas par
//! exhaustivite.
//!
//! Ce que ce reason **ne couvre pas**, sciemment, en attendant la mesure :
//!
//! - `>>` en ajout, `tee`, `sed -i`
//! - `mv` et `cp` — frequents en usage legitime, et deja rattrapes par le watcher FSEvents
//! - les heredocs (`<<EOF`), `>|`, les redirections de descripteurs (`2>&1`)
//! - un `Bash` de **lecture** (`cat`, `head`), qui echappe au read-set et n'est pas couvert ici
//!
//! Ce sont des trous nommes, pas des oublis. Le watcher les rattrape apres coup : le registre ne
//! devient pas faux, il apprend juste plus tard.
//!
//! # Les deux exclusions, et la seconde vient de la mesure
//!
//! **`/dev/*`.** `2>/dev/null` est partout et n'ecrit rien d'utile. Un reason naif sur `>` la
//! refuserait et paierait une replanification a chaque appel.
//!
//! **Hors du projet.** Ajoutee apres avoir passe le reason sur 601 commandes du depot : il
//! refusait `just tui 2>/tmp/tui.log`. Or **le registre ne suit que son arbre** — une
//! redirection vers `/tmp` ou vers `~/` ne menace aucun invariant, le watcher ne regarde meme
//! pas la. Ce n'est pas une heuristique de confort, c'est la portee reelle du registre.
//!
//! La mesure a aussi montre une classe de faux positifs qu'aucune relecture n'aurait donnee :
//! les gabarits de documentation comme `but uncommit <commit-id>`, ou le `>` ferme un
//! parametre. Une target qui commence par `-` ou qui ne ressemble pas a un path est donc
//! ignoree.
//!
//! # Les deux mesures, et laquelle compte
//!
//! **Corpus du depot — 601 chaines extraites du code et des docs.** 18,8 % de refus d'abord,
//! **2,3 % apres** les deux exclusions ci-dessus. Ce chiffre ne dit pas grand-chose de l'usage
//! reel : il sur-represente les gabarits `but <...>` de la documentation. Sa valeur a ete de
//! **trouver l'omission de la portee projet**, pas de mesurer un taux.
//!
//! **Session reelle — 9 commandes `Bash` emises par un agent Claude Code sur un vrai petit
//! projet, mode shadow (on enregistre, on ne refuse pas). Resultat : 0 refus, soit 0 %.** Les
//! commandes etaient du `ls`, `find`, `grep -rn`, `wc -l`, dont deux avec `2>/dev/null` —
//! exactement l'exclusion qui decide.
//!
//! **Le fait le plus interessant n'etait pas le taux.** Charge d'ecrire un `rapport.txt`,
//! l'agent a pris son **outil de file**, pas une redirection : l'ecriture est passee par
//! `fs/write_text_file`, donc par l'admission, spontanement. Le trou `Bash` est peut-etre plus
//! etroit en pratique qu'en principe — une raison de plus de n'elargir le reason que sur mesure.

/// La decision de la politique `Bash`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Rien a dire. La commande passe.
    Allow,
    /// Deny, avec la target qui l'a declenche — le reason la nomme.
    Deny {
        /// Le file vers lequel la commande redirige.
        target: String,
    },
}

/// Le message transmis a l'agent en cas de refus.
///
/// Il nomme le file et **l'action de remplacement**. La sonde 2 a montre que l'agent lit ce
/// reason et le cite ; un refus qui dirait seulement « refuse » le laisserait chercher.
#[must_use]
pub fn reason(target: &str) -> String {
    format!(
        "Trame ne laisse pas passer une ecriture par le shell : la redirection vers `{target}` \
         contournerait le registre d'admission, donc n'aurait ni verdict ni provenance. \
         Utilise ton outil d'ecriture de file a la place — il passe par l'admission."
    )
}

/// Applique la politique a une commande shell.
///
/// **Analyse volontairement lexicale**, pas semantique : on cherche un operateur de
/// redirection simple et sa target, et on s'arrete la. Tout ce qui sort du cas evident est
/// laisse passer — le watcher FSEvents rattrape.
#[must_use]
pub fn evaluate(commande: &str) -> Verdict {
    let octets: Vec<char> = commande.chars().collect();
    let mut index = 0;
    // Les guillemets comptent : `echo "a > b"` ne redirige rien. Sans ce tracked, ce serait le
    // premier faux positif, et le plus facile a produire.
    let mut apostrophe = false;
    let mut guillemet = false;

    while index < octets.len() {
        let c = octets[index];
        match c {
            '\'' if !guillemet => apostrophe = !apostrophe,
            '"' if !apostrophe => guillemet = !guillemet,
            '\\' => index += 1, // un caractere echappe n'est pas un operateur
            '>' if !apostrophe && !guillemet => {
                // `>>` (ajout), `>|` (ecrasement force) et `>&` (descripteur) sortent du
                // perimetre de ce premier reason.
                let suivant = octets.get(index + 1).copied();
                if matches!(suivant, Some('>' | '|' | '&')) {
                    index += 2;
                    continue;
                }
                // `2>` et `1>` sont des redirections de descripteur ; la target compte quand
                // meme, et l'exclusion `/dev/` s'en charge.
                if let Some(target) = target_after(&octets, index + 1)
                    && concerns_project(&target)
                {
                    return Verdict::Deny { target };
                }
            }
            _ => {}
        }
        index += 1;
    }
    Verdict::Allow
}

/// Extrait la target d'une redirection : le premier mot apres l'operateur.
fn target_after(octets: &[char], depuis: usize) -> Option<String> {
    let mut index = depuis;
    while index < octets.len() && octets[index].is_whitespace() {
        index += 1;
    }
    let mut target = String::new();
    while index < octets.len() {
        let c = octets[index];
        // Fin de mot : espace, ou un operateur du shell.
        if c.is_whitespace() || matches!(c, '|' | ';' | '&' | '>' | '<' | ')') {
            break;
        }
        // Une target entre guillemets reste une target.
        if !matches!(c, '"' | '\'') {
            target.push(c);
        }
        index += 1;
    }
    (!target.is_empty()).then_some(target)
}

/// Vrai si cette target est une ecriture que le registre a une raison de deny.
///
/// Trois exclusions, chacune payee par une mesure ou par la portee du registre :
///
/// 1. **`/dev/*`** — n'ecrit rien de reel. `2>/dev/null` est partout.
/// 2. **Hors du projet** — le registre ne suit que son arbre. Une redirection vers `/tmp`,
///    `~/` ou `../` ne menace aucun invariant.
/// 3. **Ce qui n'est pas un path** — un template de documentation (`<commit-id>`) laisse un
///    `>` derriere lui, et la « target » devient le drapeau suivant. Trouve sur 601 commandes.
fn concerns_project(target: &str) -> bool {
    if target.starts_with("/dev/") {
        return false;
    }
    // Un drapeau, un commentaire ou un fragment de template n'est pas un file.
    if target.starts_with('-') || target.starts_with('#') || target.starts_with('[') {
        return false;
    }
    if !target.contains('.') && !target.contains('/') {
        // Un mot nu peut etre un file (`> sortie`) : on le keeps. Mais pas s'il ne
        // contient que de la ponctuation de template.
        if !target.chars().any(char::is_alphanumeric) {
            return false;
        }
    }
    // Le repertoire courant de l'agent EST la root du projet : un path relatif y tombe,
    // un path absolu ou remontant n'y tombe pas. Verification lexicale, sans toucher au
    // disque — ce binaire est lance a chaque appel d'outil.
    if target.starts_with('/') || target.starts_with('~') {
        return false;
    }
    !target.split('/').any(|segment| segment == "..")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Le cas que le reason existe pour attraper.
    #[test]
    fn une_redirection_vers_un_fichier_est_refusee() {
        for commande in [
            "echo 'line' > notes.txt",
            "cat modele.rs > src/genere.rs",
            "printf x >sortie",
            "python3 script.py > resultat.json",
        ] {
            assert_eq!(
                evaluate(commande),
                Verdict::Deny {
                    target: match commande {
                        c if c.contains("notes") => "notes.txt".to_owned(),
                        c if c.contains("genere") => "src/genere.rs".to_owned(),
                        c if c.contains("printf") => "sortie".to_owned(),
                        _ => "resultat.json".to_owned(),
                    }
                },
                "commande : {commande}"
            );
        }
    }

    /// ★ Ce que la mesure sur 601 commandes du depot a trouve, et que la relecture n'avait pas.
    ///
    /// Le reason refusait `just tui 2>/tmp/tui.log` et les gabarits de documentation du type
    /// `but uncommit <commit-id>`. Aucun des deux ne menace un invariant : le premier ecrit hors
    /// du projet, le second n'est pas une redirection.
    #[test]
    fn ce_qui_n_ecrit_pas_dans_le_projet_passe() {
        for commande in [
            "just tui 2>/tmp/tui.log",           // hors du projet
            "cargo test > /Users/x/rapport.txt", // absolu, hors du projet
            "echo x > ~/notes.txt",              // home
            "echo x > ../voisin.txt",            // remonte hors de l'arbre
            "but uncommit <commit-id>",          // template de doc, pas une redirection
            "but squash <source> <target> -m",   // idem
            "but uncommit <commit-id> --diff",   // idem
        ] {
            assert_eq!(
                evaluate(commande),
                Verdict::Allow,
                "le registre ne suit que son arbre : {commande}"
            );
        }
    }

    /// ★ L'autre exclusion : `/dev/*` n'ecrit rien qui nous concerne.
    ///
    /// Sans elle, `2>/dev/null` — present dans une commande sur trois — couterait une
    /// replanification a chaque fois.
    #[test]
    fn les_peripheriques_passent() {
        for commande in [
            "ls -la 2>/dev/null",
            "cargo build > /dev/null 2>&1",
            "echo x >/dev/stderr",
            "grep reason file 2> /dev/null",
        ] {
            assert_eq!(
                evaluate(commande),
                Verdict::Allow,
                "une redirection vers /dev/ ne doit jamais couter une replanification : {commande}"
            );
        }
    }

    /// Hors perimetre du premier reason, et c'est ecrit : ces cas passent, le watcher rattrape.
    #[test]
    fn ce_qui_est_hors_perimetre_passe() {
        for commande in [
            "echo x >> notes.txt",      // ajout
            "echo x | tee notes.txt",   // tee
            "sed -i '' s/a/b/ file.rs", // edition sur place
            "mv a.rs b.rs",             // deplacement
            "cp a.rs b.rs",             // copie
            "cat <<EOF > f\nx\nEOF",    // heredoc plus redirection : sort du reason simple
            "echo x >| f",              // ecrasement force
        ] {
            let verdict = evaluate(commande);
            if commande.contains("<<EOF") {
                // Cas limite honnete : le heredoc tracked d'une redirection simple EST attrape.
                // On ne pretend pas le contraire — le test le constate.
                assert_eq!(
                    verdict,
                    Verdict::Deny {
                        target: "f".to_owned()
                    }
                );
            } else {
                assert_eq!(verdict, Verdict::Allow, "commande : {commande}");
            }
        }
    }

    /// Un `>` entre guillemets n'est pas une redirection. C'est le faux positif le plus facile
    /// a produire, donc celui qu'il faut verrouiller en premier.
    #[test]
    fn un_chevron_cite_ne_redirige_rien() {
        for commande in [
            r#"echo "a > b""#,
            "echo 'x > y'",
            r#"grep "=>" file.rs"#,
            r"echo a \> b",
        ] {
            assert_eq!(
                evaluate(commande),
                Verdict::Allow,
                "aucune redirection ici : {commande}"
            );
        }
    }

    /// Une commande sans redirection n'est jamais refusee. ~95 % du trafic doit passer sans un
    /// mot, et cette politique-ci ne doit pas etre celle qui casse ce chiffre.
    #[test]
    fn le_trafic_ordinaire_passe() {
        for commande in [
            "cargo test --workspace",
            "ls -la",
            "git status",
            "grep -rn reason src/",
            "just lint",
        ] {
            assert_eq!(evaluate(commande), Verdict::Allow, "commande : {commande}");
        }
    }

    /// Le reason nomme le file et l'action de remplacement.
    #[test]
    fn le_motif_est_actionnable() {
        let texte = reason("notes.txt");
        assert!(texte.contains("notes.txt"), "il nomme la target : {texte}");
        assert!(
            texte.contains("outil d'ecriture de file"),
            "et l'action de remplacement : {texte}"
        );
        assert!(
            texte.contains("admission"),
            "et la raison, parce que l'agent la relaie : {texte}"
        );
    }
}
