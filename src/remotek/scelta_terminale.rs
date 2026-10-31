//! L'agente AI chiede l'amministratore e il cliente sceglie (ADR-0028, caso 2).
//!
//! Con un lasciapassare valido (src/remotek/lasciapassare.rs), un terminale su
//! un PC Windows con il client installato porta nel `Data::Login` del
//! connection manager la [`Richiesta`] con i nomi verificati di agente e
//! tecnico. La finestra del cliente offre allora tre scelte: accetta come
//! amministratore (shell come SYSTEM, come il modo `os_login` di upstream),
//! accetta come utente (caso 1, token abbassato), rifiuta.
//!
//! "Accetta come amministratore" manda al servizio
//! `Data::RemotekSceltaAmministratore` e subito dopo l'`Authorize` di sempre.
//! Il servizio la accetta solo con un lasciapassare valido, per un terminale,
//! su un PC installato ([`scelta`]); altrimenti vale "accetta come utente".
//! Senza lasciapassare valido la finestra resta quella di oggi, e cosi' col
//! lasciapassare `utente` (ADR-0033): exec da utente ha gli stessi poteri
//! del terminale da utente. Col lasciapassare di exec la finestra dice che
//! l'agente chiede di eseguire comandi, non un terminale.
//!
//! I testi della finestra stanno qui, in italiano e in inglese (ADR-0011), e
//! si scelgono con la lingua che usa `translate` (src/lang.rs).
//!
//! Sul client non installato la shell ha il token del processo ([`Pc`]):
//! se gira elevato l'agente entra solo da amministratore, e la finestra ha
//! due scelte; se no solo da utente. L'altro caso si rifiuta prima della
//! finestra ([`rifiuto`]).

use super::lasciapassare::{self, Valido};
use super::token_terminale;
use hbb_common::{log, ResultType};
use serde_derive::{Deserialize, Serialize};

/// Chi chiede l'amministratore, dal lasciapassare verificato (mai `my_name`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Richiesta {
    pub agente: String,
    pub tecnico: String,
    /// Il lasciapassare e' di exec: si eseguono comandi, non un terminale.
    #[serde(default)]
    pub exec: bool,
    /// Client non installato ed elevato: niente "accetta come utente".
    #[serde(default)]
    pub solo_amministratore: bool,
}

/// Il PC, per la shell senza `os_login` (ADR-0028, regola 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pc {
    /// Windows con il client installato: da amministratore la shell gira
    /// come SYSTEM, da utente col token abbassato.
    Installato,
    /// Windows, client non installato che gira elevato (o non si sa): la
    /// shell ha il token del processo, solo da amministratore.
    PortatileElevato,
    /// Windows, client non installato e non elevato: solo da utente.
    PortatileNonElevato,
    /// Fuori da Windows: niente finestra del caso 2 e niente rifiuti.
    Altro,
}

impl Pc {
    /// Qui la shell da amministratore si puo' dare.
    pub fn amministratore(self) -> bool {
        matches!(self, Pc::Installato | Pc::PortatileElevato)
    }
}

/// Il PC da sistema, installazione e, solo per il portatile, elevazione del
/// processo; un errore nella lettura vale come elevato
/// (`token_terminale::portatile`).
pub fn pc_da(windows: bool, installato: bool, elevato: impl FnOnce() -> ResultType<bool>) -> Pc {
    if !windows {
        Pc::Altro
    } else if installato {
        Pc::Installato
    } else if token_terminale::portatile(elevato()).is_err() {
        Pc::PortatileElevato
    } else {
        Pc::PortatileNonElevato
    }
}

/// Il PC su cui gira questo processo.
pub fn pc() -> Pc {
    #[cfg(target_os = "windows")]
    return pc_da(true, crate::platform::is_installed(), || {
        crate::platform::is_elevated(None)
    });
    #[cfg(not(target_os = "windows"))]
    pc_da(false, false, || Ok(false))
}

/// La richiesta da mostrare al cliente: solo con un lasciapassare valido da
/// amministratore, per un terminale, su un PC che puo' dare l'amministratore.
pub fn richiesta(valido: Option<&Valido>, terminale: bool, pc: Pc) -> Option<Richiesta> {
    if !terminale || !pc.amministratore() || !lasciapassare::amministratore(valido) {
        return None;
    }
    valido.map(|v| Richiesta {
        agente: v.agente.clone(),
        tecnico: v.tecnico.clone(),
        exec: lasciapassare::exec(Some(v)),
        solo_amministratore: pc == Pc::PortatileElevato,
    })
}

/// Il rifiuto del login prima della finestra, per la shell senza `os_login`
/// sul client non installato: elevato vuole il lasciapassare da
/// amministratore, non elevato non lo puo' onorare.
pub fn rifiuto(
    lasciapassare_amministratore: bool,
    terminale: bool,
    os_username: &str,
    pc: Pc,
) -> Option<&'static str> {
    if !terminale || !os_username.trim().is_empty() {
        return None;
    }
    match (pc, lasciapassare_amministratore) {
        (Pc::PortatileElevato, false) => Some(token_terminale::RIFIUTO_SOLO_AMMINISTRATORE),
        (Pc::PortatileNonElevato, true) => Some(token_terminale::RIFIUTO_NIENTE_AMMINISTRATORE),
        _ => None,
    }
}

/// Windows con il client installato: il caso 3 senza clic e il prelogin. Il
/// lasciapassare offre l'amministratore anche sul portatile elevato ([`Pc`]).
pub fn windows_installato() -> bool {
    #[cfg(target_os = "windows")]
    return crate::platform::is_installed();
    #[cfg(not(target_os = "windows"))]
    false
}

/// Il servizio riceve "accetta come amministratore": vale solo con un
/// lasciapassare valido da amministratore (`lasciapassare::amministratore`),
/// per un terminale, su un PC che puo' dare l'amministratore.
/// Altrimenti (un client modificato, una finestra vecchia) e' "accetta come
/// utente", con una riga di warn. Dopo l'autorizzazione la scelta si ignora
/// (warn): il token e l'audit sono gia' fissati.
pub fn scelta(
    lasciapassare_valido: bool,
    terminale: bool,
    pc: Pc,
    gia_autorizzata: bool,
    controllante: &str,
) -> bool {
    if gia_autorizzata {
        log::warn!(
            "scelta di amministratore dopo l'autorizzazione: ignorata (controllante {})",
            controllante
        );
        return false;
    }
    let ammessa = lasciapassare_valido && terminale && pc.amministratore();
    if !ammessa {
        let perche = if !lasciapassare_valido {
            "senza lasciapassare da amministratore"
        } else if !terminale {
            "fuori da un terminale"
        } else {
            "su un PC che non la da'"
        };
        log::warn!(
            "scelta di amministratore {}: vale come utente (controllante {})",
            perche,
            controllante
        );
    }
    ammessa
}

/// Il terminale senza `os_login` gira da amministratore (SYSTEM sul PC
/// installato, il token del processo sul portatile elevato) solo per la
/// scelta esplicita del cliente, gia' passata da [`scelta`], e con il
/// lasciapassare da amministratore ancora valido; altrimenti col token
/// dell'utente abbassato, o si rifiuta.
pub fn come_amministratore(
    lasciapassare_valido: bool,
    pc: Pc,
    scelta_amministratore: bool,
) -> bool {
    lasciapassare_valido && pc.amministratore() && scelta_amministratore
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Lingua {
    Italiano,
    Inglese,
}

/// La lingua di `translate` (src/lang.rs, `resolve_lang`) ridotta alle due
/// della finestra: l'opzione `lang`, se vuota il locale del sistema; inglese
/// solo per "en", italiano altrimenti (ADR-0011).
pub fn lingua(salvata: &str, locale: &str) -> Lingua {
    let scelta = if salvata.is_empty() { locale } else { salvata };
    let codice = scelta
        .split(|c| c == '-' || c == '_')
        .next()
        .unwrap_or_default();
    if codice.eq_ignore_ascii_case("en") {
        Lingua::Inglese
    } else {
        Lingua::Italiano
    }
}

/// La lingua del connection manager, come la sceglie `translate`.
pub fn lingua_del_cm() -> Lingua {
    let salvata = hbb_common::config::LocalConfig::get_option("lang");
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    let locale = sys_locale::get_locale().unwrap_or_default();
    #[cfg(any(target_os = "android", target_os = "ios"))]
    let locale = String::new();
    lingua(&salvata, &locale)
}

/// Quello che la finestra del cliente mostra (flutter/lib/desktop/pages/
/// remotek_scelta_cm.dart): testo e pulsanti gia' nella lingua del CM.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Finestra {
    pub agente: String,
    pub tecnico: String,
    pub testo: String,
    pub accetta_amministratore: String,
    /// `None` sul portatile elevato: la finestra ha due scelte.
    pub accetta_utente: Option<String>,
    pub rifiuta: String,
}

pub fn finestra(richiesta: Richiesta, lingua: Lingua) -> Finestra {
    let (mut testo, amministratore, utente, rifiuta) = match lingua {
        Lingua::Italiano if richiesta.exec => (
            format!(
                "{}, agente AI di {}, chiede di eseguire comandi come amministratore",
                richiesta.agente, richiesta.tecnico
            ),
            "Accetta come amministratore",
            "Accetta come utente",
            "Rifiuta",
        ),
        Lingua::Inglese if richiesta.exec => (
            format!(
                "{}, AI agent of {}, asks to run commands as administrator",
                richiesta.agente, richiesta.tecnico
            ),
            "Accept as administrator",
            "Accept as user",
            "Decline",
        ),
        Lingua::Italiano => (
            format!(
                "{}, agente AI di {}, chiede i permessi di amministratore",
                richiesta.agente, richiesta.tecnico
            ),
            "Accetta come amministratore",
            "Accetta come utente",
            "Rifiuta",
        ),
        Lingua::Inglese => (
            format!(
                "{}, AI agent of {}, asks for administrator permissions",
                richiesta.agente, richiesta.tecnico
            ),
            "Accept as administrator",
            "Accept as user",
            "Decline",
        ),
    };
    if richiesta.solo_amministratore {
        // Il marchio da APP_NAME (REGOLE 10.3), mai scritto qui.
        let app = crate::get_app_name();
        testo.push_str(&match lingua {
            Lingua::Italiano => format!(
                ". Su questo PC {app} gira come amministratore: l'agente puo' entrare solo cosi'."
            ),
            Lingua::Inglese => format!(
                ". {app} runs as administrator on this PC: the agent can only connect this way."
            ),
        });
    }
    Finestra {
        agente: richiesta.agente,
        tecnico: richiesta.tecnico,
        testo,
        accetta_amministratore: amministratore.to_owned(),
        accetta_utente: (!richiesta.solo_amministratore).then(|| utente.to_owned()),
        rifiuta: rifiuta.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn valido() -> Valido {
        Valido {
            agente: "agente-mario".into(),
            tecnico: "mario".into(),
            nonce: "n".into(),
            modo: lasciapassare::Modo::Amministratore,
            capacita: lasciapassare::Capacita::Terminale,
        }
    }

    fn exec(modo: lasciapassare::Modo) -> Valido {
        Valido {
            modo,
            capacita: lasciapassare::Capacita::Exec,
            ..valido()
        }
    }

    #[test]
    fn exec_da_utente_nessuna_finestra_exec_da_amministratore_comandi() {
        let utente = exec(lasciapassare::Modo::Utente);
        assert_eq!(richiesta(Some(&utente), true, Pc::Installato), None);
        let r = richiesta(
            Some(&exec(lasciapassare::Modo::Amministratore)),
            true,
            Pc::Installato,
        )
        .unwrap();
        assert!(r.exec);
        let it = finestra(r.clone(), Lingua::Italiano);
        assert_eq!(
            it.testo,
            "agente-mario, agente AI di mario, chiede di eseguire comandi come amministratore"
        );
        assert_eq!(
            finestra(r, Lingua::Inglese).testo,
            "agente-mario, AI agent of mario, asks to run commands as administrator"
        );
        // Una richiesta di un CM vecchio, senza `exec`: il terminale.
        let vecchia: Richiesta =
            serde_json::from_value(json!({"agente": "a", "tecnico": "t"})).unwrap();
        assert!(!vecchia.exec);
        assert!(!vecchia.solo_amministratore);
    }

    #[test]
    fn la_richiesta_solo_con_lasciapassare_terminale_e_pc_installato() {
        let r = Richiesta {
            agente: "agente-mario".into(),
            tecnico: "mario".into(),
            exec: false,
            solo_amministratore: false,
        };
        assert_eq!(richiesta(Some(&valido()), true, Pc::Installato), Some(r));
        assert_eq!(richiesta(None, true, Pc::Installato), None);
        assert_eq!(richiesta(Some(&valido()), false, Pc::Installato), None);
        // Fuori da Windows, o col client non installato non elevato: la finestra di oggi.
        assert_eq!(richiesta(Some(&valido()), true, Pc::Altro), None);
        assert_eq!(
            richiesta(Some(&valido()), true, Pc::PortatileNonElevato),
            None
        );
    }

    #[test]
    fn il_pc_dal_sistema_installazione_ed_elevazione() {
        fn mai() -> ResultType<bool> {
            panic!("l'elevazione serve solo al portatile su Windows")
        }
        assert_eq!(pc_da(false, false, mai), Pc::Altro);
        assert_eq!(pc_da(true, true, mai), Pc::Installato);
        assert_eq!(pc_da(true, false, || Ok(true)), Pc::PortatileElevato);
        assert_eq!(pc_da(true, false, || Ok(false)), Pc::PortatileNonElevato);
        // Un errore nella lettura vale come elevato.
        assert_eq!(
            pc_da(true, false, || Err(hbb_common::anyhow::anyhow!("token"))),
            Pc::PortatileElevato
        );
    }

    // La tabella del compito 61, una casella per test: (PC, lasciapassare da
    // amministratore) -> rifiuto prima della finestra, finestra, shell.
    #[test]
    fn installato_senza_lasciapassare_da_amministratore_come_oggi() {
        assert_eq!(rifiuto(false, true, "", Pc::Installato), None);
        assert_eq!(richiesta(None, true, Pc::Installato), None);
        assert!(!come_amministratore(false, Pc::Installato, true));
    }

    #[test]
    fn installato_col_lasciapassare_da_amministratore_tre_scelte() {
        assert_eq!(rifiuto(true, true, "", Pc::Installato), None);
        let r = richiesta(Some(&valido()), true, Pc::Installato).unwrap();
        assert!(!r.solo_amministratore);
        assert!(finestra(r, Lingua::Italiano).accetta_utente.is_some());
        assert!(come_amministratore(true, Pc::Installato, true));
        assert!(!come_amministratore(true, Pc::Installato, false));
    }

    #[test]
    fn portatile_elevato_senza_lasciapassare_da_amministratore_rifiuto() {
        assert_eq!(
            rifiuto(false, true, "", Pc::PortatileElevato),
            Some(token_terminale::RIFIUTO_SOLO_AMMINISTRATORE)
        );
        assert_eq!(
            rifiuto(false, true, " ", Pc::PortatileElevato),
            Some(token_terminale::RIFIUTO_SOLO_AMMINISTRATORE)
        );
        assert!(!come_amministratore(false, Pc::PortatileElevato, true));
    }

    #[test]
    fn portatile_elevato_col_lasciapassare_da_amministratore_due_scelte() {
        assert_eq!(rifiuto(true, true, "", Pc::PortatileElevato), None);
        let r = richiesta(Some(&valido()), true, Pc::PortatileElevato).unwrap();
        assert!(r.solo_amministratore);
        let f = finestra(r.clone(), Lingua::Italiano);
        assert_eq!(f.accetta_utente, None);
        assert_eq!(
            f.testo,
            "agente-mario, agente AI di mario, chiede i permessi di amministratore. Su questo PC Remotek gira come amministratore: l'agente puo' entrare solo cosi'."
        );
        assert_eq!(
            finestra(r, Lingua::Inglese).testo,
            "agente-mario, AI agent of mario, asks for administrator permissions. Remotek runs as administrator on this PC: the agent can only connect this way."
        );
        let e = richiesta(
            Some(&exec(lasciapassare::Modo::Amministratore)),
            true,
            Pc::PortatileElevato,
        )
        .unwrap();
        assert_eq!(
            finestra(e, Lingua::Italiano).testo,
            "agente-mario, agente AI di mario, chiede di eseguire comandi come amministratore. Su questo PC Remotek gira come amministratore: l'agente puo' entrare solo cosi'."
        );
        assert!(scelta(true, true, Pc::PortatileElevato, false, "1"));
        assert!(come_amministratore(true, Pc::PortatileElevato, true));
        // Senza la scelta (una finestra vecchia) resta il rifiuto del portatile.
        assert!(!come_amministratore(true, Pc::PortatileElevato, false));
    }

    #[test]
    fn portatile_non_elevato_senza_lasciapassare_da_amministratore_come_oggi() {
        assert_eq!(rifiuto(false, true, "", Pc::PortatileNonElevato), None);
        assert_eq!(richiesta(None, true, Pc::PortatileNonElevato), None);
    }

    #[test]
    fn portatile_non_elevato_col_lasciapassare_da_amministratore_rifiuto() {
        assert_eq!(
            rifiuto(true, true, "", Pc::PortatileNonElevato),
            Some(token_terminale::RIFIUTO_NIENTE_AMMINISTRATORE)
        );
        assert!(!scelta(true, true, Pc::PortatileNonElevato, false, "1"));
        assert!(!come_amministratore(true, Pc::PortatileNonElevato, true));
    }

    #[test]
    fn i_rifiuti_non_toccano_os_login_le_altre_connessioni_e_gli_altri_sistemi() {
        for pc in [Pc::PortatileElevato, Pc::PortatileNonElevato] {
            for amministratore in [false, true] {
                assert_eq!(rifiuto(amministratore, true, "mario", pc), None);
                assert_eq!(rifiuto(amministratore, false, "", pc), None);
            }
        }
        for amministratore in [false, true] {
            assert_eq!(rifiuto(amministratore, true, "", Pc::Altro), None);
        }
        assert!(!come_amministratore(true, Pc::Altro, true));
    }

    #[test]
    fn la_scelta_senza_lasciapassare_vale_come_utente() {
        assert!(scelta(true, true, Pc::Installato, false, "1"));
        assert!(!scelta(false, true, Pc::Installato, false, "1"));
        assert!(!scelta(true, false, Pc::Installato, false, "1"));
        assert!(!scelta(true, true, Pc::Altro, false, "1"));
        // Dopo l'autorizzazione la scelta non conta piu'.
        assert!(!scelta(true, true, Pc::Installato, true, "1"));
    }

    #[test]
    fn lingua_come_translate() {
        assert_eq!(lingua("", "it-IT"), Lingua::Italiano);
        assert_eq!(lingua("", "en_US"), Lingua::Inglese);
        assert_eq!(lingua("en", "it-IT"), Lingua::Inglese);
        assert_eq!(lingua("it", "en-US"), Lingua::Italiano);
        // Le altre lingue non sono selezionabili: italiano.
        assert_eq!(lingua("", "de-DE"), Lingua::Italiano);
        assert_eq!(lingua("", ""), Lingua::Italiano);
    }

    #[test]
    fn i_campi_della_finestra() {
        let r = Richiesta {
            agente: "agente-mario".into(),
            tecnico: "mario".into(),
            exec: false,
            solo_amministratore: false,
        };
        assert_eq!(
            serde_json::to_value(finestra(r.clone(), Lingua::Italiano)).unwrap(),
            json!({
                "agente": "agente-mario",
                "tecnico": "mario",
                "testo": "agente-mario, agente AI di mario, chiede i permessi di amministratore",
                "accetta_amministratore": "Accetta come amministratore",
                "accetta_utente": "Accetta come utente",
                "rifiuta": "Rifiuta",
            })
        );
        let en = finestra(r, Lingua::Inglese);
        assert_eq!(
            en.testo,
            "agente-mario, AI agent of mario, asks for administrator permissions"
        );
        assert_eq!(
            (
                en.accetta_amministratore.as_str(),
                en.accetta_utente.as_deref(),
                en.rifiuta.as_str()
            ),
            ("Accept as administrator", Some("Accept as user"), "Decline")
        );
    }
}
