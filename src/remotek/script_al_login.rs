//! Lo script di exec viaggia nel login, cosi' il cliente lo vede nella
//! finestra della richiesta prima di scegliere (ADR-0035).
//!
//! Con exec `remotek-cli` mette in `os_login.password` (src/client.rs,
//! `handle_hash`) il lasciapassare, un `\n` e lo script in base64 standard
//! con padding ([`componi`]). Il lasciapassare (`payload.firma` in base64url)
//! non contiene mai `\n`, quindi la divisione e' univoca ([`dividi`]); il
//! base64 tiene anche i byte non UTF-8 dello script. Senza `\n` la password e'
//! il solo lasciapassare, come per il terminale.
//!
//! Sul PC lo script vale solo con un lasciapassare valido di exec e solo nel
//! primo login (`lasciapassare::solo_al_primo_login` scarta l'intera
//! password nei login ripetuti). Il login si rifiuta, prima che il connection
//! manager mostri la richiesta, se: exec arriva senza script; lo script
//! arriva con un lasciapassare assente, non valido o non di exec; il base64
//! non e' valido; lo script e' oltre i limiti di exec
//! (`script_exec::oltre_i_limiti`). Lo script cosi' accettato e' quello che la
//! finestra mostra ed e' l'unico che il PC esegue (`exec::Esecuzione`).

use super::lasciapassare::{self, Valido};
use super::script_exec;
use hbb_common::base64::{engine::general_purpose::STANDARD, Engine as _};
use std::fmt;

/// Fra il lasciapassare e lo script, nella password del login.
const SEPARATORE: char = '\n';

/// La password del login di exec: il lasciapassare, `\n`, lo script in base64.
pub fn componi(lasciapassare: &str, script: &[u8]) -> String {
    format!("{lasciapassare}{SEPARATORE}{}", STANDARD.encode(script))
}

/// Perche' il PC rifiuta il login. Il testo va al controllante con
/// `send_login_error` e `remotek-cli` lo stampa su stderr.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Rifiuto {
    /// Lo script dopo il `\n` non e' base64 standard valido.
    Base64,
    /// Lo script supera i limiti di exec.
    OltreILimiti,
    /// Un lasciapassare valido di exec senza lo script da mostrare.
    ExecSenzaScript,
    /// Uno script senza un lasciapassare valido di exec.
    ScriptSenzaExec,
}

impl fmt::Display for Rifiuto {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Rifiuto::Base64 => f.write_str(
                "exec: lo script nel login non e' base64 valido: login rifiutato",
            ),
            Rifiuto::OltreILimiti => write!(
                f,
                "exec: lo script nel login supera {} KiB o {} righe: login rifiutato",
                script_exec::MASSIMO_SCRIPT / 1024,
                script_exec::MASSIMO_RIGHE
            ),
            Rifiuto::ExecSenzaScript => f.write_str(
                "exec: il login non porta lo script da mostrare al cliente (remotek-cli da aggiornare): login rifiutato",
            ),
            Rifiuto::ScriptSenzaExec => f.write_str(
                "exec: lo script nel login non ha un lasciapassare di exec valido (scaduto, gia' usato o di un altro PC): login rifiutato",
            ),
        }
    }
}

/// Divide la password del login: il solo lasciapassare (nessun `\n`), o il
/// lasciapassare e lo script; un errore se lo script non e' base64 valido o
/// supera i limiti di exec.
pub fn dividi(password: &str) -> Result<(&str, Option<Vec<u8>>), Rifiuto> {
    let Some((lasciapassare, base64)) = password.split_once(SEPARATORE) else {
        return Ok((password, None));
    };
    let script = STANDARD.decode(base64).map_err(|_| Rifiuto::Base64)?;
    if script_exec::oltre_i_limiti(&script) {
        return Err(Rifiuto::OltreILimiti);
    }
    Ok((lasciapassare, Some(script)))
}

/// Le regole del login (src/server/connection.rs): divide la `password` gia'
/// tolta dal login (`lasciapassare::togli`, `solo_al_primo_login`), verifica
/// il lasciapassare con `verifica` (`lasciapassare::verifica_login`) e
/// restituisce il lasciapassare valido e lo script da mostrare al cliente.
/// La forma si controlla prima della verifica: un login rifiutato per lo
/// script non consuma il nonce del lasciapassare.
pub fn al_login(
    password: Option<String>,
    verifica: impl FnOnce(&str) -> Option<Valido>,
) -> Result<(Option<Valido>, Option<Vec<u8>>), Rifiuto> {
    let Some(password) = password else {
        return Ok((None, None));
    };
    let (lasciapassare, script) = dividi(&password)?;
    let valido = if lasciapassare.is_empty() {
        None
    } else {
        verifica(lasciapassare)
    };
    match (lasciapassare::exec(valido.as_ref()), script) {
        (true, Some(script)) => Ok((valido, Some(script))),
        (true, None) => Err(Rifiuto::ExecSenzaScript),
        (false, Some(_)) => Err(Rifiuto::ScriptSenzaExec),
        // Il terminale, con o senza lasciapassare valido: come prima.
        (false, None) => Ok((valido, None)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remotek::lasciapassare::{solo_al_primo_login, Capacita, Modo};
    use crate::remotek::script_exec::{MASSIMO_RIGHE, MASSIMO_SCRIPT};

    const LASCIAPASSARE: &str = "eyJ2IjoyfQ.ZmlybWE";

    fn valido(capacita: Capacita) -> Valido {
        Valido {
            agente: "agente-mario".into(),
            tecnico: "mario".into(),
            nonce: "n".into(),
            modo: Modo::Utente,
            capacita,
        }
    }

    #[test]
    fn andata_e_ritorno() {
        for script in [
            b"Get-Date\r\nexit 3\r\n".to_vec(),
            vec![b'a', 0xff, 0xfe, b'\n', 0x1b, b'['],
            Vec::new(),
        ] {
            let password = componi(LASCIAPASSARE, &script);
            assert_eq!(password.matches('\n').count(), 1);
            assert_eq!(dividi(&password), Ok((LASCIAPASSARE, Some(script))));
        }
        // Il solo lasciapassare: come il terminale.
        assert_eq!(dividi(LASCIAPASSARE), Ok((LASCIAPASSARE, None)));
    }

    #[test]
    fn base64_non_valido_e_oltre_i_limiti() {
        for cattivo in ["***", "QQ", "QQ==\n", "QQ==QQ=="] {
            let password = format!("{LASCIAPASSARE}\n{cattivo}");
            assert_eq!(dividi(&password), Err(Rifiuto::Base64), "{cattivo}");
        }
        let grande = vec![b'#'; MASSIMO_SCRIPT + 1];
        let lungo = vec![b'\n'; MASSIMO_RIGHE + 1];
        for script in [grande, lungo] {
            let password = componi(LASCIAPASSARE, &script);
            assert_eq!(dividi(&password), Err(Rifiuto::OltreILimiti));
        }
        let al_limite = vec![b'#'; MASSIMO_SCRIPT];
        assert!(dividi(&componi(LASCIAPASSARE, &al_limite)).is_ok());
        assert!(Rifiuto::OltreILimiti
            .to_string()
            .contains("32 KiB o 2000 righe"));
    }

    #[test]
    fn le_regole_del_login() {
        let exec = || Some(valido(Capacita::Exec));
        let terminale = || Some(valido(Capacita::Terminale));
        let mai = |_: &str| -> Option<Valido> { panic!("nessuna verifica") };
        let script = b"Get-Date\n".to_vec();
        let con_script = Some(componi(LASCIAPASSARE, &script));
        // Exec con lo script: lo script da mostrare.
        assert_eq!(
            al_login(con_script.clone(), |_| exec()),
            Ok((exec(), Some(script.clone())))
        );
        // Exec senza script.
        assert_eq!(
            al_login(Some(LASCIAPASSARE.into()), |_| exec()),
            Err(Rifiuto::ExecSenzaScript)
        );
        // Lo script con un lasciapassare non valido, di terminale o assente.
        assert_eq!(
            al_login(con_script.clone(), |_| None),
            Err(Rifiuto::ScriptSenzaExec)
        );
        assert_eq!(
            al_login(con_script.clone(), |_| terminale()),
            Err(Rifiuto::ScriptSenzaExec)
        );
        assert_eq!(
            al_login(Some(componi("", &script)), mai),
            Err(Rifiuto::ScriptSenzaExec)
        );
        // La forma prima della verifica: il nonce non si consuma.
        assert_eq!(
            al_login(Some(format!("{LASCIAPASSARE}\n***")), mai),
            Err(Rifiuto::Base64)
        );
        // Il terminale come prima, valido o no.
        assert_eq!(
            al_login(Some(LASCIAPASSARE.into()), |_| terminale()),
            Ok((terminale(), None))
        );
        assert_eq!(
            al_login(Some(LASCIAPASSARE.into()), |_| None),
            Ok((None, None))
        );
        assert_eq!(al_login(None, mai), Ok((None, None)));
        // Nel secondo login la password intera si scarta: niente script.
        let secondo = solo_al_primo_login(con_script, false, "123");
        assert_eq!(al_login(secondo, mai), Ok((None, None)));
    }
}
