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
//! fine ([`Script`]), in un riquadro suo ([`RIQUADRO_SCRIPT`]) che nessun
//! output raggiunge; il suo output passa in forma visibile
//! ([`FormaVisibile`]), nei riquadri dei suoi `terminal_id`. I segreti di exec (ADR-0033 passo 4) dovranno passare
//! come variabili d'ambiente, mai nel testo dello script, che il cliente vede.

use super::scelta_terminale::Lingua;
use super::script_exec::{Passo, Raccolta, MASSIMO_RIGHE, MASSIMO_SCRIPT, TROPPO_GRANDE};
use super::uscita_terminale::NON_PARTITO;
use crate::ipc::Data;
use hbb_common::message_proto::{terminal_response, TerminalAction, TerminalResponse};
use hbb_common::tokio::{
    self,
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    time,
};
use serde_json::json;
use std::{collections::HashMap, time::Duration};

/// La chiave dell'evento `cm_file_transfer_log` che Flutter manda al modello
/// del terminale (e che `CmFileModel.onFileTransferLog` ignora).
pub const AZIONE: &str = "remotek-terminale";
/// I pezzi si raccolgono e partono ogni 100 ms, per non intasare l'IPC.
const OGNI: Duration = Duration::from_millis(100);
/// Un messaggio porta al massimo 64 KiB di output.
const PEZZO_MAX: usize = 64 * 1024;

/// La chiave del riquadro dello script nella vista del CM. Il `terminal_id`
/// del protocollo e' `int32`, quindi nessun controllante puo' aprire exec o un
/// terminale su questa chiave e scriverci: nel riquadro c'e' solo quello che
/// afferma il PC (righe d'inizio e di fine, script, riga di chiusura) e nessun
/// output lo raggiunge. Una chiave `i32` riservata non basterebbe, perche' il
/// controllante sceglie liberamente il `terminal_id` dell'`OpenTerminal`. Per
/// questo il canale della vista porta chiavi `i64`. La stessa costante sta in
/// flutter/lib/desktop/pages/remotek_terminale_cm.dart (`_riquadroScript`).
pub const RIQUADRO_SCRIPT: i64 = i32::MIN as i64 - 1;

pub struct Vista {
    tx: UnboundedSender<(i64, Vec<u8>)>,
}

impl Vista {
    /// Da chiamare dentro il runtime della connessione: il ciclo che raccoglie
    /// i pezzi e' un task suo, che esce quando la `Vista` non c'e' piu'.
    pub fn nuova(conn_id: i32, tx_cm: UnboundedSender<Data>) -> Self {
        let (tx, rx) = unbounded_channel();
        tokio::spawn(ciclo(conn_id, rx, tx_cm));
        Self { tx }
    }

    /// `riquadro`: il `terminal_id` (`i64::from`) o [`RIQUADRO_SCRIPT`].
    pub fn uscita(&self, riquadro: i64, dati: &[u8]) {
        if dati.is_empty() {
            return;
        }
        // Se il task e' uscito il CM non c'e' piu': la vista non ha piu' a
        // chi mostrare l'output, la trascrizione continua lo stesso.
        self.tx.send((riquadro, dati.to_vec())).ok();
    }
}

/// Lo script di una sessione di exec per la vista. Quali azioni ne fanno
/// parte lo decide `script_exec::Raccolta`, la stessa regola di
/// `exec::Esecuzione::azione`: la vista mostra lo script che parte, e uno
/// script oltre i limiti (`script_exec::oltre_i_limiti`), che non parte, non
/// si mostra. Tiene anche la forma visibile dell'output, una per
/// `terminal_id`.
#[derive(Default)]
pub struct Script {
    raccolta: Raccolta,
    uscite: HashMap<i32, FormaVisibile>,
}

impl Script {
    /// Il testo da mostrare al cliente, nel riquadro [`RIQUADRO_SCRIPT`],
    /// quando lo script e' completo.
    pub fn azione(&mut self, azione: &TerminalAction, lingua: Lingua) -> Option<Vec<u8>> {
        match self.raccolta.azione(azione) {
            Passo::Completo(_, script) => Some(testo_script(&script, lingua)),
            _ => None,
        }
    }

    /// Un pezzo dell'output di exec per la vista. Senza questo l'output
    /// cancellerebbe o riscriverebbe lo script appena mostrato (`ESC[2J`,
    /// `ESC[3J`, movimenti del cursore); exec non ha pseudo-console, quindi
    /// il suo output non ha bisogno di sequenze di controllo. Un carattere
    /// UTF-8 o un `\r\n` spezzato fra due pezzi esce come in un pezzo solo.
    pub fn uscita(&mut self, terminal_id: i32, dati: &[u8]) -> Vec<u8> {
        self.uscite
            .entry(terminal_id)
            .or_default()
            .pezzo(dati)
            .into_bytes()
    }

    /// Quello che resta dell'output, alla chiusura, in forma visibile, sul
    /// riquadro di ogni `terminal_id`.
    pub fn resto(&mut self) -> Vec<(i64, Vec<u8>)> {
        let mut v: Vec<_> = self
            .uscite
            .iter_mut()
            .map(|(id, f)| (i64::from(*id), f.fine().into_bytes()))
            .filter(|(_, t)| !t.is_empty())
            .collect();
        v.sort_unstable();
        v
    }

    /// La chiusura del terminale dello script, partita dal PC: quello che
    /// resta dell'output, sui suoi riquadri, e poi una riga finale nella lingua
    /// del CM nel riquadro dello script, che dice al cliente come e' finita
    /// (anche uno script rifiutato, che la vista non ha mostrato). Fuori da
    /// exec o per un altro `terminal_id` niente.
    pub fn chiusura(&mut self, r: &TerminalResponse, lingua: Lingua) -> Vec<(i64, Vec<u8>)> {
        let id = self.raccolta.terminal_id();
        let it = lingua == Lingua::Italiano;
        let riga = match &r.union {
            Some(terminal_response::Union::Closed(c)) if id == Some(c.terminal_id) => {
                if it {
                    format!("fine dell'esecuzione, codice {}", c.exit_code)
                } else {
                    format!("execution finished, exit code {}", c.exit_code)
                }
            }
            Some(terminal_response::Union::Error(e)) if id == Some(e.terminal_id) => {
                let (kib, righe) = (MASSIMO_SCRIPT / 1024, MASSIMO_RIGHE);
                match (e.message.as_str(), it) {
                    (NON_PARTITO, true) => "lo script non e' partito".to_owned(),
                    (NON_PARTITO, false) => "the script did not start".to_owned(),
                    (TROPPO_GRANDE, true) => {
                        format!("script rifiutato: oltre {kib} KiB o {righe} righe, non eseguito")
                    }
                    (TROPPO_GRANDE, false) => {
                        format!("script refused: over {kib} KiB or {righe} lines, not run")
                    }
                    _ => return Vec::new(),
                }
            }
            _ => return Vec::new(),
        };
        let mut v = self.resto();
        v.push((
            RIQUADRO_SCRIPT,
            format!("\r\n{BORDO}---- {riga} ----{AZZERA}\r\n").into_bytes(),
        ));
        v
    }
}

/// Il prefisso di ogni riga dello script nella vista; le righe d'inizio e di
/// fine non lo hanno.
const PREFISSO: &str = "| ";
/// Lo stile delle righe dello script (grassetto) e quello delle righe di
/// confine, d'inizio, di fine e di chiusura (inverso): tutti e due si leggono
/// su sfondo chiaro e scuro, e solo il PC li produce.
const STILE: &str = "\x1b[1m";
const BORDO: &str = "\x1b[7m";
const AZZERA: &str = "\x1b[0m";

/// Lo script fra una riga d'inizio e una di fine, nella lingua del CM, con
/// gli a capo da terminale (`\r\n`) per la vista. La vista e' un terminale
/// xterm: ogni carattere di controllo dello script (C0 tranne `\t` e `\n`,
/// `\r` non seguito da `\n`, DEL, C1), ogni carattere di formato Unicode e
/// ogni byte non UTF-8 si mostra in forma visibile ([`FormaVisibile`]), cosi'
/// lo script non puo' cancellare o riscrivere le sue righe e il cliente vede
/// quello che parte.
///
/// Ogni riga dello script comincia con [`PREFISSO`] e ha lo stile [`STILE`],
/// dalla prima all'ultima senza interruzioni; le righe d'inizio e di fine (e
/// quella di chiusura di [`Script::chiusura`]) non hanno il prefisso e hanno
/// lo stile [`BORDO`], diverso. Lo script non puo' mandare ESC (si vede come
/// `^[`), quindi non puo' cambiare stile: una sua riga uguale a quella di
/// fine, anche spezzata da xterm, resta in grassetto e mai in inverso.
/// L'output di exec, anche lui senza ESC, non ha stile. Cosi' l'inverso e'
/// solo dei confini, il grassetto solo dello script. La trascrizione tiene i
/// byte veri.
fn testo_script(script: &[u8], lingua: Lingua) -> Vec<u8> {
    let (inizio, fine) = match lingua {
        Lingua::Italiano => (
            "---- script dell'agente ----",
            "---- fine dello script ----",
        ),
        Lingua::Inglese => ("---- agent script ----", "---- end of script ----"),
    };
    let mut forma = FormaVisibile::default();
    let mut corpo = forma.pezzo(script);
    corpo.push_str(&forma.fine());
    let mut t = format!("\r\n{BORDO}{inizio}{AZZERA}\r\n{STILE}");
    // Nella forma visibile `\r\n` e' solo l'a capo: `\r` da solo e' `^M`.
    for riga in corpo.split_terminator("\r\n") {
        t.push_str(PREFISSO);
        t.push_str(riga);
        t.push_str("\r\n");
    }
    t.push_str(&format!("{AZZERA}{BORDO}{fine}{AZZERA}\r\n"));
    t.into_bytes()
}

/// Un flusso di byte (lo script, o l'output di exec) in forma visibile per
/// la vista, a pezzi: `\n` e `\r\n` diventano `\r\n`, `\t` resta, gli altri
/// controlli, i caratteri di formato e i byte non UTF-8 come in [`visibile`].
/// Fra un pezzo e l'altro tiene un carattere UTF-8 non finito e un `\r`
/// finale, cosi' i pezzi danno lo stesso testo del flusso intero.
#[derive(Default)]
pub struct FormaVisibile {
    /// L'inizio di un carattere UTF-8 alla fine dell'ultimo pezzo.
    sospesi: Vec<u8>,
    /// Un `\r` alla fine dell'ultimo pezzo: con un `\n` dopo e' l'a capo.
    cr: bool,
}

impl FormaVisibile {
    pub fn pezzo(&mut self, dati: &[u8]) -> String {
        let mut t = String::new();
        let mut byte = std::mem::take(&mut self.sospesi);
        byte.extend_from_slice(dati);
        // A pezzi validi UTF-8 e byte non validi (senza `utf8_chunks`, che il
        // build Windows con Rust 1.75 non ha).
        let mut resto = &byte[..];
        while !resto.is_empty() {
            let (valido, errore) = match std::str::from_utf8(resto) {
                Ok(s) => (s, None),
                Err(e) => (
                    std::str::from_utf8(&resto[..e.valid_up_to()]).unwrap_or_default(),
                    Some(e.error_len()),
                ),
            };
            valido.chars().for_each(|c| self.carattere(c, &mut t));
            resto = &resto[valido.len()..];
            match errore {
                None => break,
                // Un carattere non finito: forse lo finisce il pezzo dopo.
                Some(None) => {
                    self.sospesi = resto.to_vec();
                    break;
                }
                Some(Some(n)) => {
                    resto[..n].iter().for_each(|b| self.non_valido(*b, &mut t));
                    resto = &resto[n..];
                }
            }
        }
        t
    }

    /// Alla chiusura: quello che resta.
    pub fn fine(&mut self) -> String {
        let mut t = String::new();
        std::mem::take(&mut self.sospesi)
            .iter()
            .for_each(|b| self.non_valido(*b, &mut t));
        self.cr_sospeso(&mut t);
        t
    }

    fn carattere(&mut self, c: char, t: &mut String) {
        if self.cr && c == '\n' {
            self.cr = false;
            t.push_str("\r\n");
            return;
        }
        self.cr_sospeso(t);
        if c == '\r' {
            self.cr = true;
        } else {
            visibile(c, t);
        }
    }

    fn non_valido(&mut self, b: u8, t: &mut String) {
        self.cr_sospeso(t);
        t.push_str(&format!("<0x{:02X}>", b));
    }

    /// Un `\r` non seguito da `\n`: `^M`.
    fn cr_sospeso(&mut self, t: &mut String) {
        if std::mem::take(&mut self.cr) {
            visibile('\r', t);
        }
    }
}

/// Un carattere per la vista: `\n` diventa `\r\n`, `\t` resta, gli altri
/// controlli C0 e DEL come `^X` (`^[` per ESC, `^?` per DEL), i C1 e i
/// caratteri invisibili Unicode come `<U+XXXX>`. La regola per questi ultimi
/// e' l'insieme Default_Ignorable_Code_Point di Unicode
/// (DerivedCoreProperties.txt), tutto: marcatori e incapsulamenti bidi, spazi
/// e congiunzioni di larghezza zero, BOM, trattino morbido, selettori di
/// variante, tag, riempitivi Hangul (lettere valide negli identificatori di
/// PowerShell che si vedono come spazi) e i punti riservati dell'insieme.
/// Farebbero leggere lo script in un ordine diverso da quello eseguito, vi
/// nasconderebbero caratteri o vi codificherebbero byte che non si vedono.
fn visibile(c: char, t: &mut String) {
    match c {
        '\n' => t.push_str("\r\n"),
        '\t' => t.push(c),
        '\u{0}'..='\u{1f}' => {
            t.push('^');
            t.push(char::from(c as u8 + 0x40));
        }
        '\u{7f}' => t.push_str("^?"),
        // C1, poi Default_Ignorable_Code_Point.
        '\u{80}'..='\u{9f}'
        | '\u{00ad}'
        | '\u{034f}'
        | '\u{061c}'
        | '\u{115f}'..='\u{1160}'
        | '\u{17b4}'..='\u{17b5}'
        | '\u{180b}'..='\u{180f}'
        | '\u{200b}'..='\u{200f}'
        | '\u{202a}'..='\u{202e}'
        | '\u{2060}'..='\u{206f}'
        | '\u{3164}'
        | '\u{fe00}'..='\u{fe0f}'
        | '\u{feff}'
        | '\u{ffa0}'
        | '\u{fff0}'..='\u{fff8}'
        | '\u{1bca0}'..='\u{1bca3}'
        | '\u{1d173}'..='\u{1d17a}'
        | '\u{e0000}'..='\u{e0fff}' => t.push_str(&format!("<U+{:04X}>", c as u32)),
        _ => t.push(c),
    }
}

/// Unisce i pezzi consecutivi dello stesso terminale.
fn accoda(attesa: &mut Vec<(i64, Vec<u8>)>, terminal_id: i64, dati: Vec<u8>) {
    match attesa.last_mut() {
        Some((t, d)) if *t == terminal_id => d.extend_from_slice(&dati),
        _ => attesa.push((terminal_id, dati)),
    }
}

/// I testi JSON dei pezzi in attesa, al massimo `PEZZO_MAX` byte l'uno.
fn messaggi(conn_id: i32, attesa: &mut Vec<(i64, Vec<u8>)>) -> Vec<String> {
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
fn manda(conn_id: i32, attesa: &mut Vec<(i64, Vec<u8>)>, tx_cm: &UnboundedSender<Data>) -> bool {
    messaggi(conn_id, attesa).into_iter().all(|m| {
        tx_cm
            .send(Data::FileTransferLog((AZIONE.to_owned(), m)))
            .is_ok()
    })
}

async fn ciclo(
    conn_id: i32,
    mut rx: UnboundedReceiver<(i64, Vec<u8>)>,
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
        // Il riquadro dello script: fuori dagli `int32` del `terminal_id`.
        let mut a = vec![(RIQUADRO_SCRIPT, b"x".to_vec())];
        let v: Value = serde_json::from_str(&messaggi(9, &mut a)[0]).expect("json");
        assert_eq!(v["terminal_id"].as_i64(), Some(-2_147_483_649));
        assert!(i32::try_from(RIQUADRO_SCRIPT).is_err());
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
        let t = s.azione(&dati(1, b""), it).expect("testo");
        assert_eq!(
            String::from_utf8_lossy(&t),
            "\r\n\x1b[7m---- script dell'agente ----\x1b[0m\r\n\x1b[1m| echo a\r\n| echo b\r\n\x1b[0m\x1b[7m---- fine dello script ----\x1b[0m\r\n"
        );
        assert_eq!(s.azione(&dati(1, b""), it), None, "una volta sola");
        let mut en = Script::default();
        en.azione(&apri(1), Lingua::Inglese);
        let t = en.azione(&dati(1, b""), Lingua::Inglese).expect("testo");
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
            .strip_prefix("\r\n\x1b[7m---- agent script ----\x1b[0m\r\n\x1b[1m")
            .and_then(|r| r.strip_suffix("\x1b[0m\x1b[7m---- end of script ----\x1b[0m\r\n"))
            .expect("righe d'inizio e di fine");
        assert_eq!(
            corpo,
            "| a^[[1Ab^[[2Kc^Md^He<U+009B>f^?g<0xFF>\th\r\n| i\r\n"
        );
        // Nessun controllo arriva alla vista, salvo `\t` e gli a capo `\r\n`.
        let senza_a_capo = corpo.replace("\r\n", "");
        assert!(senza_a_capo.chars().all(|c| c == '\t' || !(c.is_control())));
    }

    /// La forma visibile di un flusso in un pezzo solo.
    fn intero(flusso: &[u8]) -> String {
        let mut f = FormaVisibile::default();
        let mut t = f.pezzo(flusso);
        t.push_str(&f.fine());
        t
    }

    #[test]
    fn lo_script_resta_nel_suo_blocco() {
        let fine = "---- fine dello script ----";
        let lunga = "x".repeat(5000);
        let script = format!("{fine}\n\x1b[0mfinto output\n{lunga}\n");
        let t =
            String::from_utf8(testo_script(script.as_bytes(), Lingua::Italiano)).expect("utf-8");
        let righe: Vec<&str> = t.split_terminator("\r\n").collect();
        assert_eq!(
            righe,
            [
                "".to_owned(),
                format!("{BORDO}---- script dell'agente ----{AZZERA}"),
                format!("{STILE}{PREFISSO}{fine}"),
                format!("{PREFISSO}^[[0mfinto output"),
                format!("{PREFISSO}{lunga}"),
                format!("{AZZERA}{BORDO}{fine}{AZZERA}"),
            ]
        );
        // ESC solo negli stili del PC.
        assert_eq!(t.matches('\x1b').count(), 6);
        assert_eq!(
            confini(&t),
            ["---- script dell'agente ----", fine],
            "la fine finta non e' in inverso"
        );
    }

    /// I testi in inverso: le righe di confine.
    fn confini(t: &str) -> Vec<&str> {
        t.split(BORDO)
            .skip(1)
            .filter_map(|r| r.split(AZZERA).next())
            .collect()
    }

    #[test]
    fn solo_il_pc_scrive_i_confini() {
        let finta = "---- fine dell'esecuzione, codice 0 ----";
        let mut s = Script::default();
        s.azione(&apri(1), Lingua::Italiano);
        let t = s.uscita(1, format!("{finta}\n{BORDO}{finta}{AZZERA}\n").as_bytes());
        let t = String::from_utf8(t).expect("utf-8");
        assert!(!t.contains('\x1b'), "l'output non ha stile: {t:?}");
        let mut chiusura = TerminalResponse::new();
        chiusura.set_closed(hbb_common::message_proto::TerminalClosed {
            terminal_id: 1,
            exit_code: 0,
            ..Default::default()
        });
        let v = s.chiusura(&chiusura, Lingua::Italiano);
        let riga: Vec<u8> = v.into_iter().flat_map(|(_, t)| t).collect();
        let riga = String::from_utf8(riga).expect("utf-8");
        assert_eq!(confini(&riga), [finta]);
    }

    #[test]
    fn l_output_di_exec_in_forma_visibile() {
        let mut s = Script::default();
        let t = s.uscita(1, b"\x1b[2J\x1b[3J\x1b[Hok\rx\xff\t.\n");
        assert_eq!(
            String::from_utf8_lossy(&t),
            "^[[2J^[[3J^[[Hok^Mx<0xFF>\t.\r\n"
        );
        // Un `\r` e un carattere a meta' restano per il pezzo dopo, o per la
        // chiusura; ogni terminale ha il suo stato.
        assert_eq!(s.uscita(1, b"a\r"), b"a");
        assert_eq!(s.uscita(-2, b"\xf0\x9f"), b"");
        assert_eq!(s.uscita(1, b"\n"), b"\r\n");
        assert_eq!(s.resto(), vec![(-2, b"<0xF0><0x9F>".to_vec())]);
        assert!(s.resto().is_empty());
    }

    #[test]
    fn i_pezzi_danno_lo_stesso_testo_del_flusso_intero() {
        for flusso in ["a\u{1F600}b\r\nc".as_bytes(), b"x\xf0\x9fy\r", b"\r\r\n"] {
            let tutto = intero(flusso);
            for k in 0..=flusso.len() {
                let mut f = FormaVisibile::default();
                let mut t = f.pezzo(&flusso[..k]);
                t.push_str(&f.pezzo(&flusso[k..]));
                t.push_str(&f.fine());
                assert_eq!(t, tutto, "{flusso:?} spezzato a {k}");
            }
        }
        assert_eq!(intero("a\u{1F600}b\r\nc".as_bytes()), "a\u{1F600}b\r\nc");
        assert_eq!(intero(b"x\xf0\x9fy\r"), "x<0xF0><0x9F>y^M");
        assert_eq!(intero(b"\r\r\n"), "^M\r\n");
    }

    #[test]
    fn i_caratteri_di_formato_si_vedono() {
        let formato = [
            0x061C, 0x180E, 0x200B, 0x200F, 0x202A, 0x202E, 0x2060, 0x2064, 0x2066, 0x2069, 0xFEFF,
            0x00AD, 0x115F, 0x1160, 0x3164, 0xFFA0, 0xE0000, 0xE0041, 0xE007F, 0x034F, 0xFE00,
            0xFE0F, 0xE0100, 0xE01EF, 0x206F, 0x1D173, 0x2065, 0x17B4, 0x180B, 0xFFF0, 0x1BCA0,
            0xE0FFF,
        ];
        let flusso: String = formato
            .iter()
            .filter_map(|c| char::from_u32(*c))
            .flat_map(|c| [c, '.'])
            .collect();
        let atteso: String = formato.iter().map(|c| format!("<U+{c:04X}>.")).collect();
        let mut s = Script::default();
        assert_eq!(
            String::from_utf8_lossy(&s.uscita(1, flusso.as_bytes())),
            atteso
        );
        let t = testo_script(flusso.as_bytes(), Lingua::Italiano);
        assert!(String::from_utf8_lossy(&t).contains(&format!("{STILE}{PREFISSO}{atteso}\r\n")));
        assert_eq!(
            intero("\u{200A}".as_bytes()),
            "\u{200A}",
            "uno spazio visibile"
        );
    }

    /// Lo script piu' largo che i limiti ammettono sta intero nelle 10000
    /// righe dell'xterm del CM (`maxLines` di remotek_terminale_cm.dart)
    /// anche a 40 colonne: una riga di c colonne ne occupa al piu' c/40 + 1.
    #[test]
    fn il_caso_peggiore_sta_nell_xterm_del_cm() {
        const RIGHE_XTERM: usize = 10000;
        const COLONNE: usize = 40;
        // Per eccesso: la tabulazione fino a 8 colonne, ogni carattere fuori
        // dall'ASCII 2 (i caratteri larghi).
        let colonne = |t: &str| -> usize {
            t.replace("\r\n", "")
                .chars()
                .map(|c| match c {
                    '\t' => 8,
                    ' '..='~' => 1,
                    _ => 2,
                })
                .sum()
        };
        // Le colonne per byte della forma visibile piu' larga: ogni carattere
        // e ogni byte non UTF-8 da solo.
        let caratteri = (0..=0x10FFFF)
            .filter_map(char::from_u32)
            .map(|c| c.to_string().into_bytes());
        let per_byte = caratteri
            .chain((0x80..=0xFF).map(|b| vec![b]))
            .map(|b| colonne(&intero(&b)).div_ceil(b.len()))
            .max()
            .unwrap_or_default();
        assert_eq!(per_byte, 8, "oggi la tabulazione");
        let corpo =
            (per_byte * MASSIMO_SCRIPT + PREFISSO.len() * MASSIMO_RIGHE) / COLONNE + MASSIMO_RIGHE;
        let righe = |t: &str| -> usize {
            t.replace(STILE, "")
                .replace(BORDO, "")
                .replace(AZZERA, "")
                .split_terminator("\r\n")
                .map(|r| colonne(r) / COLONNE + 1)
                .sum()
        };
        for lingua in [Lingua::Italiano, Lingua::Inglese] {
            let vuoto = String::from_utf8(testo_script(b"", lingua)).expect("utf-8");
            // Nel riquadro dello script c'e' anche la riga di chiusura: la
            // piu' lunga delle tre.
            let mut finito = TerminalResponse::new();
            finito.set_closed(hbb_common::message_proto::TerminalClosed {
                terminal_id: 1,
                exit_code: i32::MIN,
                ..Default::default()
            });
            let errore = |m: &str| {
                let mut r = TerminalResponse::new();
                r.set_error(hbb_common::message_proto::TerminalError {
                    terminal_id: 1,
                    message: m.to_owned(),
                    ..Default::default()
                });
                r
            };
            let chiusura = [finito, errore(NON_PARTITO), errore(TROPPO_GRANDE)]
                .iter()
                .map(|r| {
                    let mut s = Script::default();
                    s.azione(&apri(1), lingua);
                    let v = s.chiusura(r, lingua);
                    assert_eq!(v.len(), 1);
                    assert_eq!(v[0].0, RIQUADRO_SCRIPT);
                    righe(&String::from_utf8_lossy(&v[0].1))
                })
                .max()
                .unwrap_or_default();
            let peggiore = corpo + righe(&vuoto) + chiusura;
            assert!(
                peggiore * 10 <= RIGHE_XTERM * 9,
                "{peggiore} righe: senza margine"
            );
        }
    }
}
