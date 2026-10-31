//! L'output dei terminali verso il connection manager (ADR-0021, regola 6): il
//! cliente vede la sessione terminale in diretta, in sola lettura. Si riusa il
//! canale che porta al CM i log del trasferimento file
//! (`ipc::Data::FileTransferLog`, evento Flutter `cm_file_transfer_log`) con
//! un'azione nostra; il CM lo mostra con
//! `flutter/lib/desktop/pages/remotek_terminale_cm.dart`.
//!
//! Va al CM solo l'output della shell, non quello che digita chi controlla:
//! quello che la shell fa vedere lo vede anche il cliente, una password
//! digitata senza eco no.
//!
//! Exec (src/remotek/exec.rs) non ha eco: lo script arriva dallo stdin e la
//! shell non lo ripete. Percio' in una sessione di exec il cliente vede anche
//! lo script, intero e prima del suo output, fra una riga d'inizio e una di
//! fine ([`Script`]). I segreti di exec (ADR-0033 passo 4) dovranno passare
//! come variabili d'ambiente, mai nel testo dello script, che il cliente vede.

use super::scelta_terminale::Lingua;
use super::script_exec::{Passo, Raccolta};
use crate::ipc::Data;
use hbb_common::message_proto::TerminalAction;
use hbb_common::tokio::{
    self,
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    time,
};
use serde_json::json;
use std::time::Duration;

/// La chiave dell'evento `cm_file_transfer_log` che Flutter manda al modello
/// del terminale (e che `CmFileModel.onFileTransferLog` ignora).
pub const AZIONE: &str = "remotek-terminale";
/// I pezzi si raccolgono e partono ogni 100 ms, per non intasare l'IPC.
const OGNI: Duration = Duration::from_millis(100);
/// Un messaggio porta al massimo 64 KiB di output.
const PEZZO_MAX: usize = 64 * 1024;

pub struct Vista {
    tx: UnboundedSender<(i32, Vec<u8>)>,
}

impl Vista {
    /// Da chiamare dentro il runtime della connessione: il ciclo che raccoglie
    /// i pezzi e' un task suo, che esce quando la `Vista` non c'e' piu'.
    pub fn nuova(conn_id: i32, tx_cm: UnboundedSender<Data>) -> Self {
        let (tx, rx) = unbounded_channel();
        tokio::spawn(ciclo(conn_id, rx, tx_cm));
        Self { tx }
    }

    pub fn uscita(&self, terminal_id: i32, dati: &[u8]) {
        if dati.is_empty() {
            return;
        }
        // Se il task e' uscito il CM non c'e' piu': la vista non ha piu' a
        // chi mostrare l'output, la trascrizione continua lo stesso.
        self.tx.send((terminal_id, dati.to_vec())).ok();
    }
}

/// Lo script di una sessione di exec per la vista. Quali azioni ne fanno
/// parte lo decide `script_exec::Raccolta`, la stessa regola di
/// `exec::Esecuzione::azione`: la vista mostra lo script che parte, e uno
/// script oltre 1 MiB, che non parte, non si mostra.
#[derive(Default)]
pub struct Script {
    raccolta: Raccolta,
}

impl Script {
    /// Il terminale e il testo da mostrare al cliente quando lo script e'
    /// completo.
    pub fn azione(&mut self, azione: &TerminalAction, lingua: Lingua) -> Option<(i32, Vec<u8>)> {
        match self.raccolta.azione(azione) {
            Passo::Completo(terminal_id, script) => {
                Some((terminal_id, testo_script(&script, lingua)))
            }
            _ => None,
        }
    }
}

/// Lo script fra una riga d'inizio e una di fine, nella lingua del CM, con
/// gli a capo da terminale (`\r\n`) per la vista. La vista e' un terminale
/// xterm: ogni carattere di controllo dello script (C0 tranne `\t` e `\n`,
/// `\r` non seguito da `\n`, DEL, C1) e ogni byte non UTF-8 si mostra in
/// forma visibile ([`visibile`]), cosi' lo script non puo' cancellare o
/// riscrivere le sue righe e il cliente vede quello che parte. La
/// trascrizione tiene i byte veri; l'output dello script passa com'e', come
/// quello del terminale.
fn testo_script(script: &[u8], lingua: Lingua) -> Vec<u8> {
    let (inizio, fine) = match lingua {
        Lingua::Italiano => (
            "---- script dell'agente ----",
            "---- fine dello script ----",
        ),
        Lingua::Inglese => ("---- agent script ----", "---- end of script ----"),
    };
    let mut t = format!("\r\n{}\r\n", inizio);
    // A pezzi validi UTF-8 e byte non validi (senza `utf8_chunks`, che il
    // build Windows con Rust 1.75 non ha).
    let mut resto = script;
    while !resto.is_empty() {
        let (valido, non_validi) = match std::str::from_utf8(resto) {
            Ok(s) => (s, 0),
            Err(e) => (
                std::str::from_utf8(&resto[..e.valid_up_to()]).unwrap_or_default(),
                e.error_len().unwrap_or(resto.len() - e.valid_up_to()),
            ),
        };
        let mut caratteri = valido.chars().peekable();
        while let Some(c) = caratteri.next() {
            if c == '\r' && caratteri.peek() == Some(&'\n') {
                continue; // `\r\n`: l'a capo lo scrive il `\n`.
            }
            visibile(c, &mut t);
        }
        let fine_validi = valido.len();
        for b in &resto[fine_validi..fine_validi + non_validi] {
            t.push_str(&format!("<0x{:02X}>", b));
        }
        resto = &resto[fine_validi + non_validi..];
    }
    if !t.ends_with("\r\n") {
        t.push_str("\r\n");
    }
    t.push_str(&format!("{}\r\n", fine));
    t.into_bytes()
}

/// Un carattere dello script per la vista: `\n` diventa `\r\n`, `\t` resta,
/// gli altri controlli C0 e DEL come `^X` (`^[` per ESC, `^?` per DEL), i C1
/// come `<U+0080>`-`<U+009F>`.
fn visibile(c: char, t: &mut String) {
    match c {
        '\n' => t.push_str("\r\n"),
        '\t' => t.push(c),
        '\u{0}'..='\u{1f}' => {
            t.push('^');
            t.push(char::from(c as u8 + 0x40));
        }
        '\u{7f}' => t.push_str("^?"),
        '\u{80}'..='\u{9f}' => t.push_str(&format!("<U+{:04X}>", c as u32)),
        _ => t.push(c),
    }
}

/// Unisce i pezzi consecutivi dello stesso terminale.
fn accoda(attesa: &mut Vec<(i32, Vec<u8>)>, terminal_id: i32, dati: Vec<u8>) {
    match attesa.last_mut() {
        Some((t, d)) if *t == terminal_id => d.extend_from_slice(&dati),
        _ => attesa.push((terminal_id, dati)),
    }
}

/// I testi JSON dei pezzi in attesa, al massimo `PEZZO_MAX` byte l'uno.
fn messaggi(conn_id: i32, attesa: &mut Vec<(i32, Vec<u8>)>) -> Vec<String> {
    let mut v = Vec::new();
    for (terminal_id, dati) in attesa.drain(..) {
        for pezzo in dati.chunks(PEZZO_MAX) {
            v.push(
                json!({
                    "conn_id": conn_id,
                    "terminal_id": terminal_id,
                    "dir": "out",
                    "data": crate::encode64(pezzo),
                })
                .to_string(),
            );
        }
    }
    v
}

/// Manda i pezzi in attesa; `false` se il CM non c'e' piu'.
fn manda(conn_id: i32, attesa: &mut Vec<(i32, Vec<u8>)>, tx_cm: &UnboundedSender<Data>) -> bool {
    messaggi(conn_id, attesa).into_iter().all(|m| {
        tx_cm
            .send(Data::FileTransferLog((AZIONE.to_owned(), m)))
            .is_ok()
    })
}

async fn ciclo(
    conn_id: i32,
    mut rx: UnboundedReceiver<(i32, Vec<u8>)>,
    tx_cm: UnboundedSender<Data>,
) {
    let mut attesa = Vec::new();
    let mut timer = time::interval(OGNI);
    loop {
        tokio::select! {
            pezzo = rx.recv() => match pezzo {
                Some((terminal_id, dati)) => accoda(&mut attesa, terminal_id, dati),
                None => {
                    manda(conn_id, &mut attesa, &tx_cm);
                    break;
                }
            },
            _ = timer.tick() => {
                if !manda(conn_id, &mut attesa, &tx_cm) {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use hbb_common::message_proto::terminal_action;
    use serde_json::Value;

    #[test]
    fn unisce_i_pezzi_dello_stesso_terminale() {
        let mut a = Vec::new();
        accoda(&mut a, 1, b"ab".to_vec());
        accoda(&mut a, 1, b"c".to_vec());
        accoda(&mut a, 2, b"x".to_vec());
        accoda(&mut a, 1, b"d".to_vec());
        assert_eq!(
            a,
            vec![(1, b"abc".to_vec()), (2, b"x".to_vec()), (1, b"d".to_vec())]
        );
    }

    #[test]
    fn messaggi_json_e_taglio() {
        let mut a = vec![(3, vec![b'x'; PEZZO_MAX + 1])];
        let m = messaggi(9, &mut a);
        assert!(a.is_empty());
        assert_eq!(m.len(), 2);
        let v: Value = serde_json::from_str(&m[1]).expect("json");
        assert_eq!(v["conn_id"], 9);
        assert_eq!(v["terminal_id"], 3);
        assert_eq!(v["dir"], "out");
        assert_eq!(v["data"], "eA==");
    }

    #[tokio::test]
    async fn manda_al_cm_e_chiude() {
        let (tx_cm, mut rx_cm) = unbounded_channel();
        let vista = Vista::nuova(5, tx_cm);
        vista.uscita(1, b"dir\r\n");
        vista.uscita(1, b"");
        vista.uscita(1, b"C:\\>");
        drop(vista);
        let mut testi = Vec::new();
        while let Some(d) = rx_cm.recv().await {
            match d {
                Data::FileTransferLog((azione, testo)) => {
                    assert_eq!(azione, AZIONE);
                    testi.push(testo);
                }
                _ => panic!("messaggio inatteso"),
            }
        }
        assert_eq!(testi.len(), 1);
        let v: Value = serde_json::from_str(&testi[0]).expect("json");
        assert_eq!(v["data"], crate::encode64(b"dir\r\nC:\\>"));
    }

    /// `testo_script` per i test di trascrizione.rs.
    pub(crate) fn testo(script: &[u8], lingua: Lingua) -> Vec<u8> {
        testo_script(script, lingua)
    }

    fn azione(u: terminal_action::Union) -> TerminalAction {
        let mut a = TerminalAction::new();
        a.union = Some(u);
        a
    }

    fn dati(id: i32, d: &[u8]) -> TerminalAction {
        azione(terminal_action::Union::Data(
            hbb_common::message_proto::TerminalData {
                terminal_id: id,
                data: d.to_vec().into(),
                ..Default::default()
            },
        ))
    }

    fn apri(id: i32) -> TerminalAction {
        azione(terminal_action::Union::Open(
            hbb_common::message_proto::OpenTerminal {
                terminal_id: id,
                ..Default::default()
            },
        ))
    }

    #[test]
    fn lo_script_intero_fra_inizio_e_fine() {
        let it = Lingua::Italiano;
        let mut s = Script::default();
        assert_eq!(s.azione(&apri(1), it), None);
        assert_eq!(s.azione(&dati(1, b"echo a\n"), it), None);
        assert_eq!(s.azione(&dati(1, b"echo b"), it), None);
        let (id, t) = s.azione(&dati(1, b""), it).expect("testo");
        assert_eq!(id, 1);
        assert_eq!(
            String::from_utf8_lossy(&t),
            "\r\n---- script dell'agente ----\r\necho a\r\necho b\r\n---- fine dello script ----\r\n"
        );
        assert_eq!(s.azione(&dati(1, b""), it), None, "una volta sola");
        let mut en = Script::default();
        en.azione(&apri(1), Lingua::Inglese);
        let (_, t) = en.azione(&dati(1, b""), Lingua::Inglese).expect("testo");
        assert!(String::from_utf8_lossy(&t).contains("---- agent script ----"));
        let mut grande = Script::default();
        grande.azione(&apri(1), it);
        grande.azione(
            &dati(1, &vec![b'#'; super::super::script_exec::MASSIMO_SCRIPT]),
            it,
        );
        grande.azione(&dati(1, b"x"), it);
        assert_eq!(
            grande.azione(&dati(1, b""), it),
            None,
            "non parte, non si mostra"
        );
        let mut chiuso = Script::default();
        chiuso.azione(&apri(1), it);
        chiuso.azione(&dati(1, b"x"), it);
        chiuso.azione(
            &azione(terminal_action::Union::Close(
                hbb_common::message_proto::CloseTerminal {
                    terminal_id: 1,
                    ..Default::default()
                },
            )),
            it,
        );
        assert_eq!(chiuso.azione(&dati(1, b""), it), None, "chiuso: niente");
    }

    #[test]
    fn i_controlli_dello_script_si_vedono_e_non_arrivano_alla_vista() {
        let script = b"a\x1b[1Ab\x1b[2Kc\rd\x08e\xc2\x9bf\x7fg\xff\th\r\ni\n";
        let t = testo_script(script, Lingua::Inglese);
        let t = String::from_utf8(t).expect("utf-8");
        let corpo = t
            .strip_prefix("\r\n---- agent script ----\r\n")
            .and_then(|r| r.strip_suffix("---- end of script ----\r\n"))
            .expect("righe d'inizio e di fine");
        assert_eq!(corpo, "a^[[1Ab^[[2Kc^Md^He<U+009B>f^?g<0xFF>\th\r\ni\r\n");
        // Nessun controllo arriva alla vista, salvo `\t` e gli a capo `\r\n`.
        let senza_a_capo = corpo.replace("\r\n", "");
        assert!(senza_a_capo.chars().all(|c| c == '\t' || !(c.is_control())));
    }
}
