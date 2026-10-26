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

use super::lasciapassare::{self, Valido};
use hbb_common::log;
use serde_derive::{Deserialize, Serialize};

/// Chi chiede l'amministratore, dal lasciapassare verificato (mai `my_name`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Richiesta {
    pub agente: String,
    pub tecnico: String,
    /// Il lasciapassare e' di exec: si eseguono comandi, non un terminale.
    #[serde(default)]
    pub exec: bool,
}

/// La richiesta da mostrare al cliente: solo con un lasciapassare valido da
/// amministratore, per un terminale, su un PC Windows con il client installato.
pub fn richiesta(
    valido: Option<&Valido>,
    terminale: bool,
    windows_installato: bool,
) -> Option<Richiesta> {
    if !terminale || !windows_installato || !lasciapassare::amministratore(valido) {
        return None;
    }
    valido.map(|v| Richiesta {
        agente: v.agente.clone(),
        tecnico: v.tecnico.clone(),
        exec: lasciapassare::exec(Some(v)),
    })
}

/// Windows con il client installato: l'unico caso in cui il lasciapassare
/// offre l'amministratore.
pub fn windows_installato() -> bool {
    #[cfg(target_os = "windows")]
    return crate::platform::is_installed();
    #[cfg(not(target_os = "windows"))]
    false
}

/// Il servizio riceve "accetta come amministratore": vale solo con un
/// lasciapassare valido da amministratore (`lasciapassare::amministratore`),
/// per un terminale, su un PC Windows installato.
/// Altrimenti (un client modificato, una finestra vecchia) e' "accetta come
/// utente", con una riga di warn. Dopo l'autorizzazione la scelta si ignora
/// (warn): il token e l'audit sono gia' fissati.
pub fn scelta(
    lasciapassare_valido: bool,
    terminale: bool,
    windows_installato: bool,
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
    let ammessa = lasciapassare_valido && terminale && windows_installato;
    if !ammessa {
        log::warn!(
            "scelta di amministratore senza lasciapassare valido, terminale o PC installato: vale come utente (controllante {})",
            controllante
        );
    }
    ammessa
}

/// Il terminale senza `os_login` gira come SYSTEM solo per la scelta esplicita
/// del cliente, gia' passata da [`scelta`], e con il lasciapassare da
/// amministratore ancora valido su un PC installato; altrimenti col token dell'utente abbassato.
pub fn come_sistema(
    lasciapassare_valido: bool,
    installato: bool,
    scelta_amministratore: bool,
) -> bool {
    lasciapassare_valido && installato && scelta_amministratore
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
    pub accetta_utente: String,
    pub rifiuta: String,
}

pub fn finestra(richiesta: Richiesta, lingua: Lingua) -> Finestra {
    let (testo, amministratore, utente, rifiuta) = match lingua {
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
    Finestra {
        agente: richiesta.agente,
        tecnico: richiesta.tecnico,
        testo,
        accetta_amministratore: amministratore.to_owned(),
        accetta_utente: utente.to_owned(),
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
        assert_eq!(richiesta(Some(&utente), true, true), None);
        let r = richiesta(Some(&exec(lasciapassare::Modo::Amministratore)), true, true).unwrap();
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
    }

    #[test]
    fn la_richiesta_solo_con_lasciapassare_terminale_e_pc_installato() {
        let r = Richiesta {
            agente: "agente-mario".into(),
            tecnico: "mario".into(),
            exec: false,
        };
        assert_eq!(richiesta(Some(&valido()), true, true), Some(r));
        assert_eq!(richiesta(None, true, true), None);
        assert_eq!(richiesta(Some(&valido()), false, true), None);
        // Fuori da Windows, o col client non installato: la finestra di oggi.
        assert_eq!(richiesta(Some(&valido()), true, false), None);
    }

    #[test]
    fn la_scelta_senza_lasciapassare_vale_come_utente() {
        assert!(scelta(true, true, true, false, "1"));
        assert!(!scelta(false, true, true, false, "1"));
        assert!(!scelta(true, false, true, false, "1"));
        assert!(!scelta(true, true, false, false, "1"));
        // Dopo l'autorizzazione la scelta non conta piu'.
        assert!(!scelta(true, true, true, true, "1"));
    }

    #[test]
    fn system_solo_con_lasciapassare_pc_installato_e_scelta_esplicita() {
        assert!(come_sistema(true, true, true));
        for (v, i, s) in [
            (false, true, true),
            (true, false, true),
            (true, true, false),
            (false, false, false),
        ] {
            assert!(!come_sistema(v, i, s), "{v} {i} {s}");
        }
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
                en.accetta_utente.as_str(),
                en.rifiuta.as_str()
            ),
            ("Accept as administrator", "Accept as user", "Decline")
        );
    }
}
