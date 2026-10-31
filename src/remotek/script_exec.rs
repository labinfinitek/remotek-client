//! Quali azioni di chi controlla fanno parte dello script di exec. E' una
//! regola sola per chi lo esegue (`exec::Esecuzione`, nel servizio del
//! terminale) e per chi lo mostra al cliente (`vista_cm::Script`, nella
//! connessione): cosi' la vista mostra esattamente lo script che parte.
//! Il modulo non e' sotto `cfg` perche' la vista c'e' su ogni sistema.

use hbb_common::message_proto::{terminal_action, TerminalAction};

/// Lo script piu' grande che exec accetta.
pub const MASSIMO_SCRIPT: usize = 1024 * 1024;

#[derive(Debug, PartialEq)]
pub enum Blocco {
    Continua,
    Completo,
    TroppoGrande,
}

/// Un `TerminalData` dello script: vuoto e' la fine.
pub fn aggiungi(script: &mut Vec<u8>, dati: &[u8]) -> Blocco {
    if dati.is_empty() {
        Blocco::Completo
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
    /// Oltre [`MASSIMO_SCRIPT`]: lo script non c'e' piu'.
    TroppoGrande(i32),
    /// Il `CloseTerminal` del terminale dello script.
    Chiuso(i32),
    /// Un'azione che non tocca lo script: un `TerminalData` prima
    /// dell'`OpenTerminal`, di un altro terminale o dopo la fine (lo script
    /// completo, oltre 1 MiB o chiuso), e ogni altra azione.
    Scartata,
}

/// Lo script di una sessione di exec, raccolto dalle azioni di chi controlla:
/// conta solo dal primo `OpenTerminal` e col suo `terminal_id`; prende i
/// `TerminalData` di quel terminale fino a quello vuoto (la fine); dopo la
/// fine, il `CloseTerminal` di quel terminale o oltre 1 MiB non prende piu'
/// niente.
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
    fn aggiungi_fino_a_1_mib() {
        let mut s = Vec::new();
        assert_eq!(aggiungi(&mut s, &vec![0; MASSIMO_SCRIPT]), Blocco::Continua);
        assert_eq!(aggiungi(&mut s, b"x"), Blocco::TroppoGrande);
        assert_eq!(aggiungi(&mut s, b""), Blocco::Completo);
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
    fn oltre_1_mib_o_chiuso_niente_script() {
        let mut grande = Raccolta::default();
        grande.azione(&apri(1));
        grande.azione(&dati(1, &vec![b'#'; MASSIMO_SCRIPT]));
        assert_eq!(grande.azione(&dati(1, b"x")), Passo::TroppoGrande(1));
        assert_eq!(grande.azione(&dati(1, b"")), Passo::Scartata);
        let mut chiusa = Raccolta::default();
        chiusa.azione(&apri(1));
        chiusa.azione(&dati(1, b"x"));
        assert_eq!(chiusa.azione(&chiudi(1)), Passo::Chiuso(1));
        assert_eq!(chiusa.azione(&dati(1, b"")), Passo::Scartata);
    }
}
