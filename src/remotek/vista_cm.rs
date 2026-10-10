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

use crate::ipc::Data;
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
mod tests {
    use super::*;
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
}
