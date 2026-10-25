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

use hbb_common::{config::HARD_SETTINGS, log};
use std::collections::HashMap;

/// La chiave del `custom.txt` firmato e l'unico valore che abilita.
pub const CHIAVE: &str = "remotek-agente";
pub const VALORE: &str = "non-presidiato";

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

/// Il terminale si autorizza senza clic solo con tutte e quattro: server
/// abilitato, lasciapassare valido, terminale, Windows col client installato.
pub fn senza_clic(
    abilitato: bool,
    lasciapassare_valido: bool,
    terminale: bool,
    windows_installato: bool,
) -> bool {
    abilitato && lasciapassare_valido && terminale && windows_installato
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
