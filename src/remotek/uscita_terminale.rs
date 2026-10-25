//! La shell che finisce chiude il terminale col suo codice (caso 20(d)).
//!
//! Nel modo helper di Windows (shell col token dell'utente) la shell gira in
//! un processo helper (src/server/terminal_helper.rs) e il servizio non ha il
//! `child`: l'helper esce col codice della shell, e il servizio legge quello
//! dell'helper per `TerminalClosed`. Fuori dal modo helper non c'e' un
//! processo helper e il codice resta quello del `child`.
//!
//! Se l'helper fallisce prima che la shell lavori (collaudo di
//! `remotek-1.4.9-4`, caso 20(d): "Failed to open PTY"), esce con
//! [`HELPER_NON_PARTITO`] e il controllante riceve un `TerminalError` con
//! [`NON_PARTITO`] invece della chiusura normale ([`risposta_chiusura`]).

use hbb_common::message_proto::{TerminalClosed, TerminalError, TerminalResponse};
use std::{
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// Il codice con cui esce l'helper quando fallisce prima che la shell lavori:
/// `run_terminal_helper` (src/server/terminal_helper.rs) torna solo con un
/// errore, prima dei thread dell'input e dell'output; finita la shell esce da
/// se' col suo codice. "RTK" e 1: fuori dai codici di una shell (0-255), dai
/// codici NTSTATUS di un crash (0xC0000000 e oltre) e da STILL_ACTIVE (259).
pub const HELPER_NON_PARTITO: i32 = 0x5254_4B01;

/// Il messaggio del `TerminalError` al controllante quando l'helper esce con
/// [`HELPER_NON_PARTITO`]; `remotek-cli` lo riconosce
/// (src/remotek/cli/terminale.rs).
pub const NON_PARTITO: &str = "il terminale non e' partito sul PC";

/// La risposta per un terminale finito col codice `codice` (quello di
/// [`scegli_codice`]): un `TerminalError` con [`NON_PARTITO`] se l'helper non
/// ha fatto partire la shell, altrimenti la chiusura col codice.
pub fn risposta_chiusura(terminal_id: i32, codice: i32) -> TerminalResponse {
    let mut risposta = TerminalResponse::new();
    if codice == HELPER_NON_PARTITO {
        risposta.set_error(TerminalError {
            terminal_id,
            message: NON_PARTITO.to_owned(),
            ..Default::default()
        });
    } else {
        risposta.set_closed(TerminalClosed {
            terminal_id,
            exit_code: codice,
            ..Default::default()
        });
    }
    risposta
}

/// Il codice con cui esce l'helper, dal codice della shell (`None` se
/// `child.wait()` e' fallito: -1, come una chiusura forzata).
pub fn codice_helper(shell: Option<u32>) -> i32 {
    // Windows legge i codici d'uscita come u32: `exit(c as i32)` torna c.
    shell.map_or(-1, |c| c as i32)
}

/// Il codice per `TerminalClosed`: quello dell'helper uscito, se c'e' un
/// helper; altrimenti quello gia' scelto dal codice di upstream (dal `child`,
/// o 0 se non lo si conosce).
pub fn scegli_codice(da_child: i32, helper: Option<u32>) -> i32 {
    helper.map_or(da_child, |c| c as i32)
}

/// Aspetta che `thread` finisca, al massimo per `massimo`; dice se e' finito.
pub fn attendi<T>(thread: &JoinHandle<T>, massimo: Duration) -> bool {
    let inizio = Instant::now();
    while !thread.is_finished() {
        if inizio.elapsed() >= massimo {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
    true
}

#[cfg(target_os = "windows")]
pub use windows_impl::con_helper;

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::scegli_codice;
    use hbb_common::log;
    use windows::Win32::{
        Foundation::{HANDLE, WAIT_OBJECT_0},
        System::Threading::{GetExitCodeProcess, WaitForSingleObject},
    };

    /// [`scegli_codice`] con l'helper della sessione, se c'e': il suo codice
    /// se esce entro `attesa_ms`, altrimenti resta `da_child`.
    pub fn con_helper(da_child: i32, helper: Option<HANDLE>, attesa_ms: u32) -> i32 {
        scegli_codice(da_child, helper.and_then(|h| uscito(h, attesa_ms)))
    }

    fn uscito(helper: HANDLE, attesa_ms: u32) -> Option<u32> {
        if unsafe { WaitForSingleObject(helper, attesa_ms) } != WAIT_OBJECT_0 {
            if attesa_ms > 0 {
                log::warn!(
                    "l'helper del terminale non e' uscito entro {} ms",
                    attesa_ms
                );
            }
            return None;
        }
        let mut codice = 0u32;
        match unsafe { GetExitCodeProcess(helper, &mut codice) } {
            Ok(()) => Some(codice),
            Err(e) => {
                log::warn!(
                    "codice d'uscita dell'helper del terminale illeggibile: {}",
                    e
                );
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn l_helper_esce_col_codice_della_shell() {
        assert_eq!(codice_helper(Some(3)), 3);
        assert_eq!(codice_helper(Some(0)), 0);
        assert_eq!(codice_helper(Some(u32::MAX)), -1);
        assert_eq!(codice_helper(None), -1);
    }

    #[test]
    fn il_codice_dell_helper_vince() {
        assert_eq!(scegli_codice(0, Some(3)), 3);
        assert_eq!(scegli_codice(0, Some(0xFFFF_FFFF)), -1);
    }

    #[test]
    fn senza_helper_resta_il_codice_del_child() {
        assert_eq!(scegli_codice(7, None), 7);
        assert_eq!(scegli_codice(-1, None), -1);
    }

    #[test]
    fn senza_nessun_codice_resta_zero() {
        assert_eq!(scegli_codice(0, None), 0);
    }

    #[test]
    fn l_helper_non_partito_e_un_errore() {
        use hbb_common::message_proto::terminal_response::Union;
        let codice = scegli_codice(0, Some(HELPER_NON_PARTITO as u32));
        assert_eq!(codice, HELPER_NON_PARTITO);
        match risposta_chiusura(4, codice).union {
            Some(Union::Error(e)) => {
                assert_eq!(e.terminal_id, 4);
                assert_eq!(e.message, NON_PARTITO);
            }
            altro => panic!("attesa una TerminalError, arrivata {:?}", altro),
        }
    }

    #[test]
    fn la_shell_finita_e_una_chiusura() {
        use hbb_common::message_proto::terminal_response::Union;
        for codice in [
            0,
            3,
            255,
            -1,
            HELPER_NON_PARTITO - 1,
            HELPER_NON_PARTITO + 1,
        ] {
            match risposta_chiusura(2, codice).union {
                Some(Union::Closed(c)) => {
                    assert_eq!(c.terminal_id, 2);
                    assert_eq!(c.exit_code, codice);
                }
                altro => panic!("attesa una TerminalClosed, arrivata {:?}", altro),
            }
        }
    }

    #[test]
    fn il_codice_non_partito_e_fuori_dai_codici_di_una_shell() {
        assert!(!(0..=259).contains(&HELPER_NON_PARTITO));
        assert!((HELPER_NON_PARTITO as u32) < 0xC000_0000);
        assert_eq!(
            codice_helper(Some(HELPER_NON_PARTITO as u32)),
            HELPER_NON_PARTITO
        );
    }

    #[test]
    fn attendi_si_ferma_al_massimo() {
        let (tx, rx) = mpsc::channel::<()>();
        let fermo = thread::spawn(move || rx.recv().is_ok());
        assert!(!attendi(&fermo, Duration::from_millis(50)));
        drop(tx);
        assert!(attendi(&fermo, Duration::from_secs(5)));
        assert!(!fermo.join().unwrap());
    }
}
