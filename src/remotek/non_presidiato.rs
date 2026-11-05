//! L'agente AI sui server non presidiati abilitati (ADR-0028, caso 3).
//!
//! Su un server Windows col client installato e la chiave `remotek-agente` =
//! `non-presidiato` nel `custom.txt` firmato (src/common.rs,
//! `read_custom_client`: le chiavi che non conosce finiscono in
//! `HARD_SETTINGS`), un terminale con un lasciapassare valido si autorizza
//! senza clic e gira come SYSTEM, anche senza nessuno alla console. Chi e'
//! alla console lo vede nel connection manager, gia' autorizzato, e lo puo'
//! chiudere. La chiave si legge solo da `HARD_SETTINGS`: un'opzione locale,
//! `override-settings` o `default-settings` non la accendono.
//!
//! Cosa entra senza clic lo dice la chiave `remotek-agente-capacita` dello
//! stesso `custom.txt` (ADR-0033, punto 7): nomi separati da `,`, senza spazi,
//! fra quelli di [`CAPACITA`], ciascuno una volta sola. Senza la chiave entra
//! senza clic solo il terminale; con la chiave malformata niente, e vale il
//! clic. Sempre e solo col lasciapassare valido da amministratore.

use super::lasciapassare::{Capacita, Modo, Valido};
use hbb_common::{config::HARD_SETTINGS, log};
use std::collections::{BTreeSet, HashMap};

/// La chiave del `custom.txt` firmato e l'unico valore che abilita.
pub const CHIAVE: &str = "remotek-agente";
pub const VALORE: &str = "non-presidiato";

/// La chiave dell'elenco di cio' che entra senza clic e i nomi ammessi. Ogni
/// costante sta su una riga: lo strumento di firma le legge cosi'.
pub const CHIAVE_CAPACITA: &str = "remotek-agente-capacita";
pub const CAPACITA: [&str; 5] = ["terminale", "exec", "file", "tunnel", "schermo"];

/// La chiave nelle impostazioni fisse del `custom.txt` firmato.
pub fn abilitato_da(fisse: &HashMap<String, String>) -> bool {
    fisse.get(CHIAVE).map_or(false, |v| v == VALORE)
}

/// [`abilitato_da`] su `HARD_SETTINGS`; se il lock non si prende, spento.
pub fn abilitato() -> bool {
    match HARD_SETTINGS.read() {
        Ok(fisse) => abilitato_da(&fisse),
        Err(_) => false,
    }
}

/// L'elenco di `remotek-agente-capacita` nelle impostazioni fisse.
#[derive(Debug, Clone, PartialEq)]
pub enum Elenco {
    /// Chiave assente: senza clic solo il terminale, come prima della chiave.
    Assente,
    /// I nomi elencati, ciascuno una volta, tutti fra [`CAPACITA`].
    Nomi(BTreeSet<&'static str>),
    /// Vuota, nome sconosciuto o ripetuto, spazi, maiuscole: niente senza clic.
    Malformato,
}

/// Legge `remotek-agente-capacita` dalle impostazioni fisse.
pub fn elenco_da(fisse: &HashMap<String, String>) -> Elenco {
    let Some(valore) = fisse.get(CHIAVE_CAPACITA) else {
        return Elenco::Assente;
    };
    let mut nomi = BTreeSet::new();
    for nome in valore.split(',') {
        match CAPACITA.iter().find(|ammesso| **ammesso == nome) {
            // `insert` falso: il nome e' ripetuto.
            Some(ammesso) if nomi.insert(*ammesso) => {}
            _ => return Elenco::Malformato,
        }
    }
    Elenco::Nomi(nomi)
}

/// [`elenco_da`] su `HARD_SETTINGS`; se il lock non si prende, malformato.
pub fn elenco() -> Elenco {
    match HARD_SETTINGS.read() {
        Ok(fisse) => elenco_da(&fisse),
        Err(_) => Elenco::Malformato,
    }
}

/// Il lasciapassare chiede qualcosa che l'elenco lascia entrare senza clic:
/// solo da amministratore, e solo cio' che il client sa fare (terminale, exec).
pub fn ammesso_senza_clic(valido: Option<&Valido>, elenco: &Elenco) -> bool {
    let Some(v) = valido.filter(|v| v.modo == Modo::Amministratore) else {
        return false;
    };
    let nome = match v.capacita {
        Capacita::Terminale => "terminale",
        Capacita::Exec => "exec",
    };
    match elenco {
        Elenco::Assente => v.capacita == Capacita::Terminale,
        Elenco::Nomi(nomi) => nomi.contains(nome),
        Elenco::Malformato => false,
    }
}

/// [`ammesso_senza_clic`] con l'elenco di `HARD_SETTINGS`: il secondo
/// argomento di [`senza_clic`] nei tre punti di src/server/connection.rs.
pub fn senza_clic_per(valido: Option<&Valido>) -> bool {
    ammesso_senza_clic(valido, &elenco())
}

/// La riga ERROR per la chiave malformata sul server abilitato. Si chiama una
/// volta per connessione dell'agente, quando il lasciapassare e' valido
/// (`lasciapassare::verifica_login`), mai nelle letture di [`senza_clic_per`].
pub fn segnala_elenco_malformato() {
    if abilitato() && elenco() == Elenco::Malformato {
        log::error!(
            "chiave {} malformata: niente senza clic, vale il clic",
            CHIAVE_CAPACITA
        );
    }
}

/// Si autorizza senza clic solo con tutte e quattro: server abilitato,
/// lasciapassare valido che l'elenco lascia entrare ([`senza_clic_per`]),
/// terminale (anche exec viaggia sul terminale), Windows col client installato.
pub fn senza_clic(
    abilitato: bool,
    ammesso_dall_elenco: bool,
    terminale: bool,
    windows_installato: bool,
) -> bool {
    abilitato && ammesso_dall_elenco && terminale && windows_installato
}

/// In prelogin (schermata di logon, nessuno alla console) il terminale viene
/// rifiutato, tranne nel caso 3: sul server non presidiato il SYSTEM parte lo
/// stesso. Il rifiuto vale dunque solo quando non siamo senza clic.
pub fn rifiuta_prelogin(prelogin: bool, senza_clic: bool) -> bool {
    prelogin && !senza_clic
}

/// Se l'output del terminale va trasmesso al connection manager. In prelogin
/// (schermata di accesso) non c'e' nessuno alla console: trasmetterlo lo
/// accumulerebbe senza tetto nella memoria del servizio verso un CM che non
/// legge, e chi poi accede lo riceverebbe tutto in arretrato. Fuori dal
/// prelogin la vista del CM si attacca come sempre.
pub fn vista_al_cm(prelogin: bool) -> bool {
    !prelogin
}

/// La riga INFO del registro del servizio per ogni terminale non presidiato.
pub fn registra(agente: &str, tecnico: &str, controllante: &str) {
    log::info!(
        "agente non presidiato: agente {}, tecnico {}, controllante {}",
        agente,
        tecnico,
        controllante
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fisse(coppie: &[(&str, &str)]) -> HashMap<String, String> {
        coppie
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn abilitato_solo_con_la_chiave_e_il_valore_giusti() {
        assert!(abilitato_da(&fisse(&[(CHIAVE, VALORE)])));
        assert!(!abilitato_da(&fisse(&[])));
        assert!(!abilitato_da(&fisse(&[(CHIAVE, "Y")])));
        assert!(!abilitato_da(&fisse(&[(CHIAVE, "Non-Presidiato")])));
        assert!(!abilitato_da(&fisse(&[("remotek-agente ", VALORE)])));
    }

    #[test]
    fn senza_clic_solo_con_tutte_le_condizioni() {
        assert!(senza_clic(true, true, true, true));
        for i in 0..4 {
            let mut c = [true; 4];
            c[i] = false;
            assert!(!senza_clic(c[0], c[1], c[2], c[3]), "{:?}", c);
        }
    }

    #[test]
    fn abilitato_legge_solo_le_impostazioni_fisse() {
        use hbb_common::config::{DEFAULT_SETTINGS, OVERWRITE_SETTINGS};
        use std::sync::RwLock;

        // Salva e ripristina solo la nostra chiave in ciascuna mappa globale,
        // cosi' il test non lascia stato dietro di se'.
        let salva = |m: &RwLock<HashMap<String, String>>| m.write().unwrap().remove(CHIAVE);
        let ripristina = |m: &RwLock<HashMap<String, String>>, v: Option<String>| {
            let mut g = m.write().unwrap();
            match v {
                Some(v) => {
                    g.insert(CHIAVE.to_owned(), v);
                }
                None => {
                    g.remove(CHIAVE);
                }
            }
        };

        let fisse = salva(&HARD_SETTINGS);
        let sovrascritte = salva(&OVERWRITE_SETTINGS);
        let predefinite = salva(&DEFAULT_SETTINGS);

        // In override o in default la chiave non accende il caso 3.
        OVERWRITE_SETTINGS
            .write()
            .unwrap()
            .insert(CHIAVE.to_owned(), VALORE.to_owned());
        DEFAULT_SETTINGS
            .write()
            .unwrap()
            .insert(CHIAVE.to_owned(), VALORE.to_owned());
        assert!(!abilitato());

        // Solo nelle impostazioni fisse del custom.txt firmato accende.
        OVERWRITE_SETTINGS.write().unwrap().remove(CHIAVE);
        DEFAULT_SETTINGS.write().unwrap().remove(CHIAVE);
        HARD_SETTINGS
            .write()
            .unwrap()
            .insert(CHIAVE.to_owned(), VALORE.to_owned());
        assert!(abilitato());

        ripristina(&HARD_SETTINGS, fisse);
        ripristina(&OVERWRITE_SETTINGS, sovrascritte);
        ripristina(&DEFAULT_SETTINGS, predefinite);
    }

    fn nomi(elenco: &[&'static str]) -> Elenco {
        Elenco::Nomi(elenco.iter().copied().collect())
    }

    #[test]
    fn elenco_assente_valido_o_malformato() {
        assert_eq!(elenco_da(&fisse(&[])), Elenco::Assente);
        assert_eq!(elenco_da(&fisse(&[(CHIAVE, VALORE)])), Elenco::Assente);
        let validi: [(&str, &[&'static str]); 5] = [
            ("terminale", &["terminale"]),
            ("exec", &["exec"]),
            ("terminale,exec", &["terminale", "exec"]),
            ("exec,terminale", &["terminale", "exec"]),
            ("terminale,exec,file,tunnel,schermo", &CAPACITA),
        ];
        for (valore, attesi) in validi {
            let letto = elenco_da(&fisse(&[(CHIAVE_CAPACITA, valore)]));
            assert_eq!(letto, nomi(attesi), "{:?}", valore);
        }
        let malformati = [
            "",
            "terminale,",
            ",exec",
            "terminale, exec",
            "Exec",
            "terminale,terminale",
            "shell",
        ];
        for valore in malformati {
            let letto = elenco_da(&fisse(&[(CHIAVE_CAPACITA, valore)]));
            assert_eq!(letto, Elenco::Malformato, "{:?}", valore);
        }
    }

    #[test]
    fn decisione_senza_clic_tutta_la_tabella() {
        let lasciapassare = |modo, capacita| Valido {
            agente: "a".into(),
            tecnico: "t".into(),
            nonce: "n".into(),
            modo,
            capacita,
        };
        let terminale = lasciapassare(Modo::Amministratore, Capacita::Terminale);
        let exec = lasciapassare(Modo::Amministratore, Capacita::Exec);
        let terminale_ut = lasciapassare(Modo::Utente, Capacita::Terminale);
        let exec_ut = lasciapassare(Modo::Utente, Capacita::Exec);
        // Server abilitato e Windows installato: decide l'elenco.
        let decide =
            |l: &Valido, e: &Elenco| senza_clic(true, ammesso_senza_clic(Some(l), e), true, true);
        // (elenco, terminale da amministratore, exec da amministratore)
        let tabella = [
            (Elenco::Assente, true, false),
            (nomi(&["exec"]), false, true),
            (nomi(&["terminale"]), true, false),
            (nomi(&["terminale", "exec"]), true, true),
            (nomi(&["file", "tunnel", "schermo"]), false, false),
            (Elenco::Malformato, false, false),
        ];
        for (elenco, si_terminale, si_exec) in &tabella {
            assert_eq!(decide(&terminale, elenco), *si_terminale, "{:?}", elenco);
            assert_eq!(decide(&exec, elenco), *si_exec, "{:?}", elenco);
            // Modo utente: mai.
            assert!(!decide(&terminale_ut, elenco), "{:?}", elenco);
            assert!(!decide(&exec_ut, elenco), "{:?}", elenco);
            // Senza lasciapassare: mai.
            assert!(!ammesso_senza_clic(None, elenco));
            for l in [&terminale, &exec] {
                let ammesso = ammesso_senza_clic(Some(l), elenco);
                // Senza remotek-agente = non-presidiato: mai.
                assert!(!senza_clic(false, ammesso, true, true));
                // Non Windows installato: mai.
                assert!(!senza_clic(true, ammesso, true, false));
            }
        }
    }

    #[test]
    fn le_costanti_hanno_la_forma_che_legge_lo_strumento_di_firma() {
        let sorgente = include_str!("non_presidiato.rs");
        let righe: Vec<&str> = sorgente.lines().collect();
        assert!(righe.contains(&r#"pub const CHIAVE_CAPACITA: &str = "remotek-agente-capacita";"#));
        assert!(righe.contains(
            &r#"pub const CAPACITA: [&str; 5] = ["terminale", "exec", "file", "tunnel", "schermo"];"#
        ));
    }

    #[test]
    fn in_prelogin_niente_vista_al_cm() {
        // In prelogin l'output non va al CM; fuori dal prelogin sì.
        assert!(!vista_al_cm(true));
        assert!(vista_al_cm(false));
    }

    #[test]
    fn in_prelogin_il_caso_3_non_viene_rifiutato() {
        // In prelogin senza il caso 3 si rifiuta; col caso 3 (senza clic) no.
        assert!(rifiuta_prelogin(true, false));
        assert!(!rifiuta_prelogin(true, true));
        // Fuori dal prelogin non si rifiuta mai, con o senza il caso 3.
        assert!(!rifiuta_prelogin(false, false));
        assert!(!rifiuta_prelogin(false, true));
    }
}
