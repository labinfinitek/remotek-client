//! Quali azioni di chi controlla fanno parte dello script di exec. E' una
//! regola sola per chi lo esegue (`exec::Esecuzione`, nel servizio del
//! terminale) e per chi lo mostra al cliente (`vista_cm::Script`, nella
//! connessione): cosi' la vista mostra esattamente lo script che parte.
//! Il modulo non e' sotto `cfg` perche' la vista c'e' su ogni sistema.

use hbb_common::message_proto::{terminal_action, TerminalAction};

/// Lo script piu' grande che exec accetta: 32 KiB e [`MASSIMO_RIGHE`] righe.
/// Il cliente vede lo script nella sua forma visibile (`vista_cm`) in un
/// xterm del connection manager che tiene 10000 righe: entro questi limiti
/// lo script ci sta intero anche a 40 colonne (il test
/// `il_caso_peggiore_sta_nell_xterm_del_cm` di `vista_cm` fa il conto).
pub const MASSIMO_SCRIPT: usize = 32 * 1024;
/// Le righe dello script: i `\n`, piu' una se l'ultimo byte non e' `\n`.
pub const MASSIMO_RIGHE: usize = 2000;

/// Il messaggio del `TerminalError` per uno script oltre i limiti: il PC non
/// lo esegue e la vista non lo mostra.
pub const TROPPO_GRANDE: &str = "lo script supera 32 KiB o 2000 righe: non eseguito";

/// Le righe dello script, come le conta [`MASSIMO_RIGHE`].
pub fn righe(script: &[u8]) -> usize {
    let a_capo = script.iter().filter(|b| **b == b'\n').count();
    a_capo + usize::from(script.last().is_some_and(|b| *b != b'\n'))
}

/// Il controllo dei limiti, l'unico: per chi esegue e mostra lo script
/// ([`Raccolta`]) e per `remotek-cli exec`, prima di collegarsi.
pub fn oltre_i_limiti(script: &[u8]) -> bool {
    script.len() > MASSIMO_SCRIPT || righe(script) > MASSIMO_RIGHE
}

#[derive(Debug, PartialEq)]
pub enum Blocco {
    Continua,
    Completo,
    TroppoGrande,
}

/// Un `TerminalData` dello script: vuoto e' la fine, e solo li' si contano
/// le righe ([`oltre_i_limiti`]); prima si guarda solo la lunghezza, per non
/// tenere in memoria piu' di [`MASSIMO_SCRIPT`] byte.
pub fn aggiungi(script: &mut Vec<u8>, dati: &[u8]) -> Blocco {
    if dati.is_empty() {
        if oltre_i_limiti(script) {
            Blocco::TroppoGrande
        } else {
            Blocco::Completo
        }
    } else if script.len() + dati.len() > MASSIMO_SCRIPT {
        Blocco::TroppoGrande
    } else {
        script.extend_from_slice(dati);
        Blocco::Continua
    }
}

/// Che cosa e' un'azione per lo script.
#[derive(Debug, PartialEq)]
pub enum Passo {
    /// Il primo `OpenTerminal`: lo script e' del suo `terminal_id`.
    Aperto(i32),
    /// Un altro `OpenTerminal`: non cambia niente.
    AltroOpen(i32),
    /// Un pezzo dello script, messo da parte.
    Pezzo,
    /// Il `TerminalData` vuoto: lo script intero, una volta sola.
    Completo(i32, Vec<u8>),
    /// Oltre [`MASSIMO_SCRIPT`] o [`MASSIMO_RIGHE`]: lo script non c'e' piu'.
    TroppoGrande(i32),
    /// Il `CloseTerminal` del terminale dello script.
    Chiuso(i32),
    /// Un'azione che non tocca lo script: un `TerminalData` prima
    /// dell'`OpenTerminal`, di un altro terminale o dopo la fine (lo script
    /// completo, oltre i limiti o chiuso), e ogni altra azione.
    Scartata,
}

/// Lo script di una sessione di exec, raccolto dalle azioni di chi controlla:
/// conta solo dal primo `OpenTerminal` e col suo `terminal_id`; prende i
/// `TerminalData` di quel terminale fino a quello vuoto (la fine); dopo la
/// fine, il `CloseTerminal` di quel terminale o oltre i limiti
/// ([`oltre_i_limiti`]) non prende piu' niente.
#[derive(Default)]
pub struct Raccolta {
    terminal_id: Option<i32>,
    script: Vec<u8>,
    finita: bool,
}

impl Raccolta {
    pub fn azione(&mut self, azione: &TerminalAction) -> Passo {
        match &azione.union {
            Some(terminal_action::Union::Open(o)) => {
                if self.terminal_id.is_some() {
                    return Passo::AltroOpen(o.terminal_id);
                }
                self.terminal_id = Some(o.terminal_id);
                Passo::Aperto(o.terminal_id)
            }
            Some(terminal_action::Union::Data(d))
                if self.terminal_id == Some(d.terminal_id) && !self.finita =>
            {
                match aggiungi(&mut self.script, &d.data) {
                    Blocco::Continua => Passo::Pezzo,
                    Blocco::TroppoGrande => {
                        self.finita = true;
                        self.script = Vec::new();
                        Passo::TroppoGrande(d.terminal_id)
                    }
                    Blocco::Completo => {
                        self.finita = true;
                        Passo::Completo(d.terminal_id, std::mem::take(&mut self.script))
                    }
                }
            }
            Some(terminal_action::Union::Close(c)) if self.terminal_id == Some(c.terminal_id) => {
                self.finita = true;
                self.script = Vec::new();
                Passo::Chiuso(c.terminal_id)
            }
            _ => Passo::Scartata,
        }
    }

    /// Il terminale dello script, dal primo `OpenTerminal`.
    pub fn terminal_id(&self) -> Option<i32> {
        self.terminal_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hbb_common::message_proto::{CloseTerminal, OpenTerminal, TerminalData};

    fn azione(u: terminal_action::Union) -> TerminalAction {
        let mut a = TerminalAction::new();
        a.union = Some(u);
        a
    }

    fn apri(id: i32) -> TerminalAction {
        azione(terminal_action::Union::Open(OpenTerminal {
            terminal_id: id,
            ..Default::default()
        }))
    }

    fn dati(id: i32, d: &[u8]) -> TerminalAction {
        azione(terminal_action::Union::Data(TerminalData {
            terminal_id: id,
            data: d.to_vec().into(),
            ..Default::default()
        }))
    }

    fn chiudi(id: i32) -> TerminalAction {
        azione(terminal_action::Union::Close(CloseTerminal {
            terminal_id: id,
            ..Default::default()
        }))
    }

    #[test]
    fn aggiungi_fino_a_32_kib_e_2000_righe() {
        let mut s = Vec::new();
        assert_eq!(aggiungi(&mut s, &vec![0; MASSIMO_SCRIPT]), Blocco::Continua);
        assert_eq!(aggiungi(&mut s, b"x"), Blocco::TroppoGrande);
        assert_eq!(aggiungi(&mut s, b""), Blocco::Completo);
        for (script, oltre) in [
            ("a\n".repeat(MASSIMO_RIGHE), false),
            (format!("{}a", "a\n".repeat(MASSIMO_RIGHE - 1)), false),
            ("\n".repeat(MASSIMO_RIGHE + 1), true),
            (format!("{}a", "a\n".repeat(MASSIMO_RIGHE)), true),
        ] {
            assert_eq!(oltre_i_limiti(script.as_bytes()), oltre, "{}", script.len());
            let mut s = script.into_bytes();
            let atteso = if oltre {
                Blocco::TroppoGrande
            } else {
                Blocco::Completo
            };
            assert_eq!(aggiungi(&mut s, b""), atteso);
        }
        assert_eq!((righe(b""), righe(b"\r"), righe(b"a\nb")), (0, 1, 2));
        // Il testo del rifiuto dice i limiti veri.
        let limiti = format!("{} KiB o {} righe", MASSIMO_SCRIPT / 1024, MASSIMO_RIGHE);
        assert!(TROPPO_GRANDE.contains(&limiti));
    }

    #[test]
    fn le_esche_non_toccano_lo_script() {
        let mut r = Raccolta::default();
        assert_eq!(r.azione(&dati(1, b"prima\n")), Passo::Scartata);
        assert_eq!(r.azione(&dati(1, b"")), Passo::Scartata);
        assert_eq!(r.azione(&apri(1)), Passo::Aperto(1));
        assert_eq!(r.azione(&apri(2)), Passo::AltroOpen(2));
        assert_eq!(r.azione(&dati(2, b"altro\n")), Passo::Scartata);
        assert_eq!(r.azione(&dati(1, b"a\n")), Passo::Pezzo);
        assert_eq!(r.azione(&dati(2, b"")), Passo::Scartata);
        assert_eq!(r.azione(&chiudi(2)), Passo::Scartata);
        assert_eq!(r.azione(&dati(1, b"b\n")), Passo::Pezzo);
        assert_eq!(
            r.azione(&dati(1, b"")),
            Passo::Completo(1, b"a\nb\n".to_vec())
        );
        assert_eq!(r.azione(&dati(1, b"dopo\n")), Passo::Scartata);
        assert_eq!(r.azione(&dati(1, b"")), Passo::Scartata, "una volta sola");
        assert_eq!(r.azione(&chiudi(1)), Passo::Chiuso(1));
        assert_eq!(r.terminal_id(), Some(1));
    }

    #[test]
    fn oltre_i_limiti_o_chiuso_niente_script() {
        let mut grande = Raccolta::default();
        grande.azione(&apri(1));
        grande.azione(&dati(1, &vec![b'#'; MASSIMO_SCRIPT]));
        assert_eq!(grande.azione(&dati(1, b"x")), Passo::TroppoGrande(1));
        assert_eq!(grande.azione(&dati(1, b"")), Passo::Scartata);
        let mut lungo = Raccolta::default();
        lungo.azione(&apri(1));
        lungo.azione(&dati(1, &vec![b'\n'; MASSIMO_RIGHE + 1]));
        assert_eq!(lungo.azione(&dati(1, b"")), Passo::TroppoGrande(1));
        let mut chiusa = Raccolta::default();
        chiusa.azione(&apri(1));
        chiusa.azione(&dati(1, b"x"));
        assert_eq!(chiusa.azione(&chiudi(1)), Passo::Chiuso(1));
        assert_eq!(chiusa.azione(&dati(1, b"")), Passo::Scartata);
    }
}
