//! Trascrizione delle sessioni terminale (ADR-0021, regole 5 e 6). Il
//! contratto e' il README di remotek-api, sezione "Trascrizione delle sessioni
//! terminale": ogni sessione terminale si trascrive dal PC controllato, a
//! blocchi numerati con hash concatenato; una copia va all'API
//! (`POST /api/audit/terminal`), una resta sul PC, nella cartella `terminale`
//! di Remotek (vedi `cartella`).
//!
//! Un blocco e' di una sola direzione e di un solo `terminal_id`: si accumulano
//! i byte consecutivi e il blocco parte quando cambia direzione o terminale,
//! quando arriva a 64 KiB o quando il byte piu' vecchio in attesa ha almeno un
//! secondo. API e file locale hanno gli stessi blocchi e la stessa catena, cosi'
//! l'hash finale del file si confronta con la verifica del pannello.

use hbb_common::{
    chrono::Local,
    config::Config,
    log,
    message_proto::{message, terminal_action, terminal_response, Message, TerminalAction},
    tokio::sync::mpsc::UnboundedSender,
};
use super::scelta_terminale::Lingua;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::mpsc as std_mpsc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Limiti dell'API: 64 KiB per blocco, 20 MiB e 100000 blocchi per sessione.
const BLOCCO_MAX: usize = 64 * 1024;
const SESSIONE_MAX_BYTE: usize = 20 * 1024 * 1024;
const SESSIONE_MAX_BLOCCHI: u64 = 100_000;
/// Il byte piu' vecchio in attesa non aspetta piu' di questo.
const ATTESA_MAX: Duration = Duration::from_secs(1);
/// Come il registro delle connessioni: i file locali restano un anno.
const CONSERVAZIONE: Duration = Duration::from_secs(365 * 24 * 3600);
const CARTELLA: &str = "terminale";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dir {
    In,
    Out,
}

impl Dir {
    fn nome(self) -> &'static str {
        match self {
            Dir::In => "in",
            Dir::Out => "out",
        }
    }

    fn byte(self) -> u8 {
        match self {
            Dir::In => b'i',
            Dir::Out => b'o',
        }
    }
}

/// SHA-256( H(seq-1) || d || data ), con H(0) = 32 byte a zero.
fn hash_blocco(precedente: &[u8; 32], dir: Dir, dati: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(precedente);
    h.update([dir.byte()]);
    h.update(dati);
    h.finalize().into()
}

#[derive(Debug)]
struct Blocco {
    seq: u64,
    dir: Dir,
    terminal_id: i32,
    dati: Vec<u8>,
    hash: [u8; 32],
    fine: bool,
}

struct Attesa {
    dir: Dir,
    terminal_id: i32,
    dati: Vec<u8>,
    dal: Instant,
}

/// Accumula i byte e taglia i blocchi; tiene la catena degli hash.
struct Accumulo {
    attesa: Option<Attesa>,
    seq: u64,
    ultimo: [u8; 32],
    chiuso: bool,
}

impl Accumulo {
    fn new() -> Self {
        Self {
            attesa: None,
            seq: 0,
            ultimo: [0; 32],
            chiuso: false,
        }
    }

    fn blocco(&mut self, dir: Dir, terminal_id: i32, dati: Vec<u8>, fine: bool) -> Blocco {
        self.seq += 1;
        self.ultimo = hash_blocco(&self.ultimo, dir, &dati);
        Blocco {
            seq: self.seq,
            dir,
            terminal_id,
            dati,
            hash: self.ultimo,
            fine,
        }
    }

    fn svuota(&mut self, fine: bool) -> Option<Blocco> {
        let a = self.attesa.take()?;
        Some(self.blocco(a.dir, a.terminal_id, a.dati, fine))
    }

    fn aggiungi(&mut self, dir: Dir, terminal_id: i32, dati: &[u8], ora: Instant) -> Vec<Blocco> {
        let mut pronti = Vec::new();
        if self.chiuso || dati.is_empty() {
            return pronti;
        }
        if matches!(&self.attesa, Some(a) if a.dir != dir || a.terminal_id != terminal_id) {
            pronti.extend(self.svuota(false));
        }
        let a = self.attesa.get_or_insert_with(|| Attesa {
            dir,
            terminal_id,
            dati: Vec::new(),
            dal: ora,
        });
        a.dati.extend_from_slice(dati);
        while self.attesa.as_ref().map_or(0, |a| a.dati.len()) >= BLOCCO_MAX {
            let Some(mut a) = self.attesa.take() else {
                break;
            };
            let resto = a.dati.split_off(BLOCCO_MAX);
            pronti.push(self.blocco(dir, terminal_id, a.dati, false));
            if !resto.is_empty() {
                self.attesa = Some(Attesa {
                    dir,
                    terminal_id,
                    dati: resto,
                    dal: a.dal,
                });
            }
        }
        pronti.extend(self.scadenza(ora));
        pronti
    }

    fn scadenza(&mut self, ora: Instant) -> Option<Blocco> {
        if ora.duration_since(self.attesa.as_ref()?.dal) >= ATTESA_MAX {
            return self.svuota(false);
        }
        None
    }

    /// Quello che resta, con `fine`; se non resta niente, un blocco vuoto.
    fn fine(&mut self, terminal_id: i32) -> Option<Blocco> {
        if self.chiuso {
            return None;
        }
        self.chiuso = true;
        match self.svuota(true) {
            Some(b) => Some(b),
            None => Some(self.blocco(Dir::Out, terminal_id, Vec::new(), true)),
        }
    }
}

/// Invio all'API, nella coda in ordine della connessione.
struct Api {
    url: String,
    tx: UnboundedSender<(String, Value)>,
    id: String,
    uuid: String,
    conn_id: i32,
    byte: usize,
    fermo: bool,
}

impl Api {
    fn manda(&mut self, b: &Blocco) {
        if self.fermo {
            return;
        }
        if b.seq > SESSIONE_MAX_BLOCCHI || self.byte + b.dati.len() > SESSIONE_MAX_BYTE {
            self.fermo = true;
            log::warn!(
                "#{} trascrizione terminale oltre i limiti dell'API (blocco {}, {} byte): non si manda piu', il file locale continua",
                self.conn_id,
                b.seq,
                self.byte
            );
            return;
        }
        self.byte += b.dati.len();
        let v = json!({
            "id": self.id,
            "uuid": self.uuid,
            "conn_id": self.conn_id,
            "seq": b.seq,
            "dir": b.dir.nome(),
            "data": crate::encode64(&b.dati),
            "hash": hex::encode(b.hash),
            "fine": b.fine,
        });
        if self.tx.send((self.url.clone(), v)).is_err() {
            // La coda c'e' finche' c'e' la connessione: senza, non c'e' piu'
            // niente da mandare.
            self.fermo = true;
            log::warn!(
                "#{} trascrizione terminale: coda dell'audit chiusa",
                self.conn_id
            );
        }
    }
}

/// La riga del file locale di un blocco.
fn riga(b: &Blocco, ora_ms: u128) -> String {
    json!({
        "ora": ora_ms as u64,
        "terminal_id": b.terminal_id,
        "seq": b.seq,
        "dir": b.dir.nome(),
        "data": crate::encode64(&b.dati),
        "hash": hex::encode(b.hash),
        "fine": b.fine,
    })
    .to_string()
}

/// Su Windows accanto ai log del servizio (`...\Remotek\terminale` nel suo
/// profilo); altrove sotto la cartella di configurazione di Remotek, perche'
/// la cartella dei log li' non e' dentro quella di Remotek (su Linux
/// `~/.local/share/logs/<APP_NAME>`). `None` se la cartella non si conosce.
fn cartella() -> Option<PathBuf> {
    #[cfg(windows)]
    let c = Config::log_path().parent()?.join(CARTELLA);
    #[cfg(not(windows))]
    let c = Config::path(CARTELLA);
    // `Config::path` senza cartella di configurazione restituisce un percorso relativo.
    c.is_absolute().then_some(c)
}

/// Scrive le righe in un thread suo, per non bloccare il ciclo della
/// connessione. Un errore va nel log e non ferma la sessione.
fn scrittore(file: PathBuf, rx: std_mpsc::Receiver<String>) {
    let apri = || -> io::Result<fs::File> {
        if let Some(c) = file.parent() {
            fs::create_dir_all(c)?;
        }
        fs::OpenOptions::new().create(true).append(true).open(&file)
    };
    let mut f: Option<fs::File> = None;
    let mut perse = 0usize;
    for r in rx {
        let esito = match f.as_mut() {
            Some(f) => writeln!(f, "{}", r),
            None => apri().and_then(|mut nuovo| {
                let esito = writeln!(nuovo, "{}", r);
                f = Some(nuovo);
                esito
            }),
        };
        if let Err(e) = esito {
            if perse == 0 {
                log::error!("trascrizione terminale, scrittura di {:?}: {}", file, e);
            }
            perse += 1;
        }
    }
    if perse > 0 {
        log::error!(
            "trascrizione terminale, {:?}: {} righe non scritte",
            file,
            perse
        );
    }
}

pub struct Trascrizione {
    accumulo: Accumulo,
    api: Option<Api>,
    file: Option<std_mpsc::Sender<String>>,
    vista: Option<super::vista_cm::Vista>,
    /// Solo con exec: lo script per la vista (`vista_cm::Script`).
    script: super::vista_cm::Script,
    #[cfg(test)]
    scrittore: Option<std::thread::JoinHandle<()>>,
    terminal_id: i32,
}

impl Trascrizione {
    fn nuova(
        conn_id: i32,
        url: String,
        tx: &UnboundedSender<(String, Value)>,
        cartella: Option<PathBuf>,
        vista: Option<super::vista_cm::Vista>,
    ) -> Self {
        let api = (!url.is_empty()).then(|| Api {
            url,
            tx: tx.clone(),
            id: Config::get_id(),
            uuid: crate::encode64(hbb_common::get_uuid()),
            conn_id,
            byte: 0,
            fermo: false,
        });
        let mut t = Self {
            accumulo: Accumulo::new(),
            api,
            file: None,
            vista,
            script: Default::default(),
            #[cfg(test)]
            scrittore: None,
            terminal_id: 0,
        };
        match cartella {
            Some(c) => {
                let nome = format!("{}-{}.jsonl", Local::now().format("%Y%m%d-%H%M%S"), conn_id);
                let (tx_file, rx_file) = std_mpsc::channel();
                let file = c.join(nome);
                match std::thread::Builder::new()
                    .name("trascrizione".to_owned())
                    .spawn(move || scrittore(file, rx_file))
                {
                    Ok(_h) => {
                        t.file = Some(tx_file);
                        #[cfg(test)]
                        {
                            t.scrittore = Some(_h);
                        }
                    }
                    Err(e) => log::error!(
                        "#{} trascrizione terminale, thread del file: {}",
                        conn_id,
                        e
                    ),
                }
            }
            None => log::error!(
                "#{} trascrizione terminale: cartella dei log sconosciuta",
                conn_id
            ),
        }
        t
    }

    fn registra(&mut self, blocchi: impl IntoIterator<Item = Blocco>) {
        let ora_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default();
        for b in blocchi {
            if let Some(api) = self.api.as_mut() {
                api.manda(&b);
            }
            if let Some(f) = self.file.as_ref() {
                if f.send(riga(&b, ora_ms)).is_err() {
                    // Il thread del file e' uscito e l'ha gia' scritto nel log.
                    self.file = None;
                }
            }
        }
    }

    fn dati(&mut self, dir: Dir, terminal_id: i32, dati: &[u8]) {
        self.terminal_id = terminal_id;
        if let (Dir::Out, Some(v)) = (dir, self.vista.as_ref()) {
            v.uscita(terminal_id, dati);
        }
        let blocchi = self
            .accumulo
            .aggiungi(dir, terminal_id, dati, Instant::now());
        self.registra(blocchi);
    }

    /// Exec non ha eco: lo script completo va alla vista prima che parta (e
    /// quindi prima del suo output, che passa dallo stesso canale). La
    /// trascrizione resta quella di `dati`.
    fn script_alla_vista(&mut self, terminal_id: i32, dati: &[u8], lingua: Lingua) {
        if let (Some(v), Some(testo)) = (self.vista.as_ref(), self.script.aggiungi(dati, lingua)) {
            v.uscita(terminal_id, &testo);
        }
    }

    /// Da chiamare ogni secondo: manda il blocco in attesa da almeno 1 s.
    pub fn tick(&mut self) {
        let b = self.accumulo.scadenza(Instant::now());
        self.registra(b);
    }

    fn chiudi(&mut self) {
        let b = self.accumulo.fine(self.terminal_id);
        self.registra(b);
    }
}

impl Drop for Trascrizione {
    fn drop(&mut self) {
        self.chiudi();
    }
}

/// Un `TerminalAction` arrivato da chi controlla: apre la trascrizione della
/// connessione alla prima azione e registra i byte delle azioni Data cosi'
/// come arrivano. L'output va anche al connection manager (`tx_cm`), che lo
/// mostra al cliente; con `exec` (`lasciapassare::exec`) anche lo script.
pub fn ingresso(
    t: &mut Option<Trascrizione>,
    conn_id: i32,
    tx: &UnboundedSender<(String, Value)>,
    tx_cm: &UnboundedSender<crate::ipc::Data>,
    azione: &TerminalAction,
    exec: bool,
) {
    let t = t.get_or_insert_with(|| {
        let url = crate::get_audit_server(
            Config::get_option("api-server"),
            Config::get_option("custom-rendezvous-server"),
            "terminal".to_owned(),
        );
        // Remotek (ADR-0028, caso 3): in prelogin (schermata di accesso) non
        // c'e' nessuno alla console, quindi l'output non va al CM. Altrimenti
        // si accumulerebbe senza tetto verso un CM che non legge e chi poi
        // accede lo riceverebbe tutto in arretrato. Fuori dal prelogin la
        // vista e' quella di sempre.
        let vista = if crate::remotek::non_presidiato::vista_al_cm(crate::platform::is_prelogin())
        {
            Some(super::vista_cm::Vista::nuova(conn_id, tx_cm.clone()))
        } else {
            None
        };
        Trascrizione::nuova(conn_id, url, tx, cartella(), vista)
    });
    if let Some(terminal_action::Union::Data(d)) = &azione.union {
        t.dati(Dir::In, d.terminal_id, &d.data);
        if exec {
            let lingua = super::scelta_terminale::lingua_del_cm();
            t.script_alla_vista(d.terminal_id, &d.data, lingua);
        }
    }
}

/// Un messaggio che va a chi controlla: registra l'output dei terminali,
/// decompresso se serve.
pub fn uscita(t: &mut Option<Trascrizione>, msg: &Message) {
    let Some(t) = t.as_mut() else {
        return;
    };
    if let Some(message::Union::TerminalResponse(r)) = &msg.union {
        if let Some(terminal_response::Union::Data(d)) = &r.union {
            if d.compressed {
                t.dati(
                    Dir::Out,
                    d.terminal_id,
                    &hbb_common::compress::decompress(&d.data),
                );
            } else {
                t.dati(Dir::Out, d.terminal_id, &d.data);
            }
        }
    }
}

fn pulisci(cartella: &Path, ora: SystemTime) -> io::Result<usize> {
    let mut tolti = 0;
    for voce in fs::read_dir(cartella)? {
        let voce = voce?;
        let meta = voce.metadata()?;
        if !meta.is_file() {
            continue;
        }
        let vecchio = ora
            .duration_since(meta.modified()?)
            .is_ok_and(|eta| eta > CONSERVAZIONE);
        if vecchio {
            fs::remove_file(voce.path())?;
            tolti += 1;
        }
    }
    Ok(tolti)
}

/// All'avvio del servizio: cancella le trascrizioni locali piu' vecchie di
/// 365 giorni e scrive nel log quante.
pub fn pulizia() {
    let Some(c) = cartella() else {
        return;
    };
    if !c.exists() {
        return;
    }
    match pulisci(&c, SystemTime::now()) {
        Ok(n) => log::info!(
            "trascrizioni terminale piu' vecchie di 365 giorni cancellate: {}",
            n
        ),
        Err(e) => log::error!("pulizia delle trascrizioni terminale in {:?}: {}", c, e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hbb_common::{message_proto::*, tokio, tokio::sync::mpsc};

    fn hex_hash(b: &Blocco) -> String {
        hex::encode(b.hash)
    }

    #[test]
    fn catena_del_vettore() {
        let ora = Instant::now();
        let mut a = Accumulo::new();
        assert!(a.aggiungi(Dir::In, 1, b"dir\r", ora).is_empty());
        let b = a.aggiungi(Dir::Out, 1, "Volume in unità C\r\n".as_bytes(), ora);
        assert_eq!(b.len(), 1);
        assert_eq!(
            hex_hash(&b[0]),
            "6963244db801a8d9acddd6c85a0a89bec0f3711df27d128d15d3f98d5be829b4"
        );
        let b2 = a.scadenza(ora + ATTESA_MAX).expect("blocco 2");
        assert_eq!((b2.seq, b2.dir, b2.fine), (2, Dir::Out, false));
        assert_eq!(
            hex_hash(&b2),
            "fe8040f5b83445643966bb94d99d87d87d882fcbb468b32ca101e849fcad74ee"
        );
        let b3 = a.fine(1).expect("blocco 3");
        assert_eq!((b3.seq, b3.dati.len(), b3.fine), (3, 0, true));
        assert_eq!(
            hex_hash(&b3),
            "155ac5a514282cb9adce116ced49243a196c041f7421dffbe71e752e5ca2c857"
        );
        assert!(a.fine(1).is_none());
    }

    #[test]
    fn cambio_di_direzione_e_di_terminale() {
        let ora = Instant::now();
        let mut a = Accumulo::new();
        assert!(a.aggiungi(Dir::In, 1, b"l", ora).is_empty());
        assert!(a.aggiungi(Dir::In, 1, b"s\r", ora).is_empty());
        let b = a.aggiungi(Dir::Out, 1, b"a.txt", ora);
        assert_eq!(b.len(), 1);
        assert_eq!((b[0].dir, b[0].dati.as_slice()), (Dir::In, &b"ls\r"[..]));
        let b = a.aggiungi(Dir::Out, 2, b"x", ora);
        assert_eq!((b[0].terminal_id, b[0].dati.as_slice()), (1, &b"a.txt"[..]));
        assert!(a.scadenza(ora + ATTESA_MAX / 2).is_none());
    }

    #[test]
    fn taglio_a_64_kib() {
        let ora = Instant::now();
        let mut a = Accumulo::new();
        let b = a.aggiungi(Dir::Out, 1, &vec![b'x'; 2 * BLOCCO_MAX + 10], ora);
        assert_eq!(b.len(), 2);
        assert!(b.iter().all(|b| b.dati.len() == BLOCCO_MAX));
        let fine = a.fine(1).expect("fine");
        assert_eq!((fine.seq, fine.dati.len(), fine.fine), (3, 10, true));
    }

    #[test]
    fn invio_all_api_e_fine_alla_chiusura() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut t = Some(Trascrizione::nuova(
            7,
            "https://api/api/audit/terminal".to_owned(),
            &tx,
            None,
            None,
        ));
        let mut azione = TerminalAction::new();
        azione.set_data(TerminalData {
            terminal_id: 1,
            data: b"dir\r".to_vec().into(),
            ..Default::default()
        });
        let (tx_cm, _rx_cm) = mpsc::unbounded_channel();
        ingresso(&mut t, 7, &tx, &tx_cm, &azione, false);
        let mut risposta = TerminalResponse::new();
        risposta.set_data(TerminalData {
            terminal_id: 1,
            data: hbb_common::compress::compress("Volume in unità C\r\n".as_bytes()).into(),
            compressed: true,
            ..Default::default()
        });
        let mut msg = Message::new();
        msg.set_terminal_response(risposta);
        uscita(&mut t, &msg);
        drop(t);
        let mut v = Vec::new();
        while let Ok((url, b)) = rx.try_recv() {
            assert_eq!(url, "https://api/api/audit/terminal");
            v.push(b);
        }
        assert_eq!(v.len(), 2);
        assert_eq!(v[0]["seq"], 1);
        assert_eq!(v[0]["dir"], "in");
        assert_eq!(v[0]["data"], "ZGlyDQ==");
        assert_eq!(v[0]["conn_id"], 7);
        assert_eq!(v[0]["fine"], false);
        assert_eq!(v[1]["dir"], "out");
        assert_eq!(v[1]["fine"], true);
        assert_eq!(
            v[1]["hash"],
            "fe8040f5b83445643966bb94d99d87d87d882fcbb468b32ca101e849fcad74ee"
        );
        assert!(v[0].get("id").is_some() && v[0].get("uuid").is_some());
    }

    #[test]
    fn exec_passa_come_il_terminale() {
        use crate::remotek::exec::{tests::*, Esecuzione, TERMINAL_ID_STDERR};
        let c = cartella_di_prova("exec");
        let (tx, _rx) = mpsc::unbounded_channel();
        let (tx_cm, _rx_cm) = mpsc::unbounded_channel();
        let mut t = Some(Trascrizione::nuova(7, String::new(), &tx, Some(c.clone()), None));
        let mut e = Esecuzione::nuova();
        for azione in [apri(1), manda(1, b"echo o; echo e >&2\n"), manda(1, b"")] {
            ingresso(&mut t, 7, &tx, &tx_cm, &azione, true);
            e.azione(&azione, None);
        }
        let inizio = Instant::now();
        while !rx_tutto(&mut t, &mut e) && inizio.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(20));
        }
        let mut t = t.expect("trascrizione");
        t.chiudi();
        t.file = None;
        if let Some(h) = t.scrittore.take() {
            h.join().expect("thread del file");
        }
        let file = fs::read_dir(&c).expect("cartella").flatten().next().expect("file");
        let v: Vec<String> = fs::read_to_string(file.path())
            .expect("file")
            .lines()
            .map(|r| serde_json::from_str::<Value>(r).expect("json"))
            .map(|b| format!("{} {} {}", b["dir"], b["terminal_id"], b["data"]))
            .collect();
        assert_eq!(v[0], r#""in" 1 "ZWNobyBvOyBlY2hvIGUgPiYyCg==""#);
        assert!(v.contains(&r#""out" 1 "bwo=""#.to_owned()));
        assert!(v.contains(&format!(r#""out" {} "ZQo=""#, TERMINAL_ID_STDERR)));
        drop(t);
        fs::remove_dir_all(&c).ok();
    }

    /// Le risposte di exec passano da `uscita`, come quelle del terminale.
    fn rx_tutto(t: &mut Option<Trascrizione>, e: &mut crate::remotek::exec::Esecuzione) -> bool {
        let mut chiuso = false;
        for r in e.leggi() {
            chiuso |= matches!(r.union, Some(terminal_response::Union::Closed(_)));
            let mut msg = Message::new();
            msg.set_terminal_response(r);
            uscita(t, &msg);
        }
        chiuso
    }

    #[tokio::test]
    async fn exec_mostra_lo_script_prima_dell_output_il_terminale_no() {
        for exec in [true, false] {
            let (tx, _rx) = mpsc::unbounded_channel();
            let (tx_cm, mut rx_cm) = mpsc::unbounded_channel();
            let vista = crate::remotek::vista_cm::Vista::nuova(7, tx_cm);
            let mut t = Trascrizione::nuova(7, String::new(), &tx, None, Some(vista));
            for d in [&b"Get-"[..], b"Date", b""] {
                t.dati(Dir::In, 1, d);
                if exec {
                    t.script_alla_vista(1, d, Lingua::Inglese);
                }
            }
            t.dati(Dir::Out, 1, b"ok");
            drop(t);
            let mut visto = Vec::new();
            while let Some(crate::ipc::Data::FileTransferLog((_, testo))) = rx_cm.recv().await {
                let v: Value = serde_json::from_str(&testo).expect("json");
                visto.extend(crate::decode64(v["data"].as_str().unwrap_or("")).expect("base64"));
            }
            let visto = String::from_utf8_lossy(&visto).into_owned();
            let atteso = "\r\n---- agent script ----\r\nGet-Date\r\n---- end of script ----\r\nok";
            assert_eq!(visto, if exec { atteso } else { "ok" });
        }
    }

    #[tokio::test]
    async fn solo_l_uscita_va_al_cm() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let (tx_cm, mut rx_cm) = mpsc::unbounded_channel();
        let vista = crate::remotek::vista_cm::Vista::nuova(7, tx_cm);
        let mut t = Trascrizione::nuova(7, String::new(), &tx, None, Some(vista));
        t.dati(Dir::In, 1, b"segreto\r");
        t.dati(Dir::Out, 1, b"C:\\>");
        drop(t);
        let mut testi = Vec::new();
        while let Some(crate::ipc::Data::FileTransferLog((_, testo))) = rx_cm.recv().await {
            testi.push(testo);
        }
        assert_eq!(testi.len(), 1);
        let v: Value = serde_json::from_str(&testi[0]).expect("json");
        assert_eq!(v["dir"], "out");
        assert_eq!(v["data"], crate::encode64(b"C:\\>"));
    }

    #[test]
    fn stop_oltre_i_limiti() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut t = Trascrizione::nuova(1, "u".to_owned(), &tx, None, None);
        if let Some(api) = t.api.as_mut() {
            api.byte = SESSIONE_MAX_BYTE - 2;
        }
        t.dati(Dir::In, 1, b"ab");
        t.dati(Dir::Out, 1, b"c");
        t.dati(Dir::In, 1, b"d");
        assert_eq!(t.accumulo.seq, 2);
        assert_eq!(
            rx.try_recv().map(|(_, v)| v["seq"].clone()).ok(),
            Some(json!(1))
        );
        assert!(rx.try_recv().is_err());
        // Anche il blocco 100001 non parte.
        let mut api = t.api.take().expect("api");
        api.fermo = false;
        api.byte = 0;
        let mut a = Accumulo::new();
        a.seq = SESSIONE_MAX_BLOCCHI;
        let b = a.fine(1).expect("fine");
        api.manda(&b);
        assert!(api.fermo && rx.try_recv().is_err());
    }

    #[cfg(not(windows))]
    #[test]
    fn cartella_dentro_quella_di_remotek() {
        let c = cartella().expect("cartella");
        assert_eq!(c, Config::path("").join(CARTELLA));
        // Su Linux `directories_next` scrive il nome in minuscolo.
        let app = hbb_common::config::APP_NAME.read().unwrap().to_lowercase();
        assert!(
            c.components().any(|p| p
                .as_os_str()
                .to_string_lossy()
                .to_lowercase()
                .contains(&app)),
            "{c:?}"
        );
        assert!(!c.starts_with(Config::log_path().parent().expect("log")));
    }

    fn cartella_di_prova(nome: &str) -> PathBuf {
        let c = std::env::temp_dir().join(format!(
            "remotek-trascrizione-{}-{}",
            nome,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&c);
        c
    }

    #[test]
    fn riga_del_file_locale() {
        let c = cartella_di_prova("file");
        let (tx, _rx) = mpsc::unbounded_channel();
        let mut t = Trascrizione::nuova(42, String::new(), &tx, Some(c.clone()), None);
        assert!(t.api.is_none());
        t.dati(Dir::In, 3, b"dir\r");
        t.chiudi();
        t.file = None;
        if let Some(h) = t.scrittore.take() {
            h.join().expect("thread del file");
        }
        let file: Vec<_> = fs::read_dir(&c).expect("cartella").flatten().collect();
        assert_eq!(file.len(), 1);
        let nome = file[0].file_name().to_string_lossy().into_owned();
        assert!(nome.ends_with("-42.jsonl") && nome.len() == "AAAAMMGG-hhmmss-42.jsonl".len());
        let testo = fs::read_to_string(file[0].path()).expect("file");
        let righe: Vec<Value> = testo
            .lines()
            .map(|r| serde_json::from_str(r).expect("json"))
            .collect();
        assert_eq!(righe.len(), 1);
        assert_eq!(righe[0]["terminal_id"], 3);
        assert_eq!(righe[0]["seq"], 1);
        assert_eq!(righe[0]["dir"], "in");
        assert_eq!(righe[0]["data"], "ZGlyDQ==");
        assert_eq!(righe[0]["fine"], true);
        assert_eq!(righe[0]["hash"].as_str().map(str::len), Some(64));
        assert!(righe[0]["ora"].as_u64().unwrap_or(0) > 1_700_000_000_000);
        drop(t);
        fs::remove_dir_all(&c).ok();
    }

    #[test]
    fn cancella_i_file_vecchi() {
        let c = cartella_di_prova("pulizia");
        fs::create_dir_all(&c).expect("cartella");
        let ora = SystemTime::now();
        for (nome, eta) in [("vecchio.jsonl", 366), ("nuovo.jsonl", 364)] {
            let f = fs::File::create(c.join(nome)).expect("file");
            f.set_modified(ora - Duration::from_secs(eta * 24 * 3600))
                .expect("data");
        }
        assert_eq!(pulisci(&c, ora).expect("pulizia"), 1);
        assert!(!c.join("vecchio.jsonl").exists());
        assert!(c.join("nuovo.jsonl").exists());
        fs::remove_dir_all(&c).ok();
    }
}
