//! `terminal <ID>`: il terminale di un PC, come la sessione terminale del
//! client Flutter (src/flutter.rs, `session_add` e `session_start_`), ma senza
//! interfaccia. Si entra solo se il cliente accetta la finestra (ADR-0021,
//! regola 2): nessuna password del PC; mai col modo amministratore di
//! upstream (regola 3). L'amministratore si chiede solo col lasciapassare
//! dell'API (`--amministratore`, ADR-0028), e decide il PC.
//!
//! `exec <ID>` (ADR-0033) usa la stessa sessione: lo script letto da stdin va
//! al PC solo dopo il `TerminalOpened` con `remotek-exec`
//! (src/remotek/exec.rs), poi stdout e stderr del PC escono separati.

use crate::client::{Data, Interface, QualityStatus};
use crate::ui_session_interface::{io_loop, InvokeUiSession, Session};
use hbb_common::{
    config::{Ab, LocalConfig, PeerConfig},
    message_proto::{terminal_response::Union, *},
    rendezvous_proto::ConnType,
};
use std::io::{Read, Write};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc::{channel, RecvTimeoutError, Sender},
    Arc, RwLock,
};
use std::time::{Duration, Instant};

/// Il terminale che il CLI apre sul PC (uno per sessione).
const ID_TERMINALE: i32 = 0;
/// Dopo la fine di stdin, quanto si aspetta il `TerminalClosed` del PC.
const ATTESA_CHIUSURA: Duration = Duration::from_secs(5);
/// Le righe e le colonne di default di `terminal`, e quelle con cui exec
/// apre il suo terminale (senza pseudo-console non contano).
pub(super) const RIGHE_PREDEFINITE: u32 = 24;
pub(super) const COLONNE_PREDEFINITE: u32 = 80;
/// I `TerminalData` dello script di exec: al massimo 64 KiB l'uno.
const BLOCCO_SCRIPT: usize = 64 * 1024;

#[derive(Debug, PartialEq)]
pub(super) enum Evento {
    /// Il login e' partito senza password: si aspetta il clic del cliente.
    Attesa,
    /// Il controllato offre una connessione non cifrata da un capo all'altro.
    NonCifrata,
    /// La connessione e' su; `true` se il PC dichiara trascrizione e vista
    /// del terminale (`dichiarazione::dichiara_il_terminale`, ADR-0034).
    Connesso(bool),
    /// Con exec: il PC dichiara trascrizione e vista ma non mostra lo script
    /// nella finestra della richiesta (`dichiarazione::
    /// dichiara_lo_script_nella_scelta`, ADR-0035): il PC del -7 o del -8.
    ScriptNonNellaScelta,
    /// Il PC ha aperto il terminale; il `message` del `TerminalOpened`.
    Aperto(String),
    Dati(Vec<u8>),
    /// stderr di exec (`exec::TERMINAL_ID_STDERR`).
    Stderr(Vec<u8>),
    Chiuso(i32),
    /// L'helper del PC e' uscito prima che la shell partisse
    /// (`uscita_terminale::NON_PARTITO`).
    NonPartito,
    Errore(String),
    FineInput,
}

/// L'handler della sessione: trasforma i richiami di `io_loop` in eventi per
/// il ciclo del CLI. I metodi che servono solo a schermo, file e chiamate
/// vocali restano vuoti.
#[derive(Clone, Default)]
pub(super) struct Gestore {
    eventi: Option<Sender<Evento>>,
    /// Il login porta il lasciapassare (`--amministratore`).
    amministratore: bool,
    /// La `PeerInfo` del PC dichiara trascrizione e vista del terminale.
    /// `set_peer_info` la scrive, `on_connected`, che viene dopo, la legge.
    pc_dichiara: Arc<AtomicBool>,
    /// La sessione e' di exec: `on_connected` guarda anche
    /// `pc_script_nella_scelta`.
    exec: bool,
    /// La `PeerInfo` del PC dichiara lo script nella finestra della scelta.
    pc_script_nella_scelta: Arc<AtomicBool>,
}

impl Gestore {
    /// Il gestore del tunnel (src/remotek/cli/tunnel.rs): solo gli eventi.
    pub(super) fn con_eventi(eventi: Sender<Evento>) -> Self {
        Self {
            eventi: Some(eventi),
            ..Default::default()
        }
    }

    fn invia(&self, evento: Evento) {
        if let Some(tx) = &self.eventi {
            // Il ricevitore sparisce solo quando il CLI sta gia' uscendo.
            tx.send(evento).ok();
        }
    }
}

/// Cosa fare ai due rifiuti del client non installato
/// (`token_terminale::RIFIUTO_SOLO_AMMINISTRATORE` e
/// `RIFIUTO_NIENTE_AMMINISTRATORE`); `amministratore`: il login portava il
/// lasciapassare, che il PC puo' aver scartato (scaduto, gia' usato, di un
/// altro PC). `app`: il nome del client sul PC (`super::marchio`, APP_NAME
/// senza `-cli`). Il CLI non rilancia da solo.
fn consiglio(testo: &str, amministratore: bool, app: &str) -> Option<String> {
    use crate::remotek::token_terminale::{
        RIFIUTO_NIENTE_AMMINISTRATORE, RIFIUTO_SOLO_AMMINISTRATORE,
    };
    if testo == RIFIUTO_SOLO_AMMINISTRATORE && amministratore {
        Some(format!(
            "lasciapassare rifiutato dal PC (scaduto, gia' usato o di un altro PC), che accetta l'agente solo come amministratore ({app} non installato, avviato come amministratore): controlla l'ID del PC e l'orologio"
        ))
    } else if testo == RIFIUTO_SOLO_AMMINISTRATORE {
        Some(format!(
            "il PC accetta l'agente solo come amministratore ({app} non installato, avviato come amministratore): rilancia con --amministratore, il cliente vedra' la richiesta"
        ))
    } else if testo == RIFIUTO_NIENTE_AMMINISTRATORE {
        Some(format!(
            "su questo PC l'amministratore non c'e' ({app} non installato, avviato senza permessi di amministratore): rilancia senza --amministratore, o fai installare {app}"
        ))
    } else {
        None
    }
}

/// Il significato di un messaggio per il CLI (src/client.rs, `handle_hash` e
/// `handle_login_error`). `None`: niente da fare.
pub(super) fn evento_da_msgbox(
    tipo: &str,
    titolo: &str,
    testo: &str,
    amministratore: bool,
) -> Option<Evento> {
    if tipo == "error" {
        if let Some(c) = consiglio(testo, amministratore, &super::marchio()) {
            return Some(Evento::Errore(c));
        }
    }
    match tipo {
        // Solo il PC che aspetta il clic (`LOGIN_MSG_NO_PASSWORD_ACCESS`). Non
        // "input-password": `handle_hash` lo mostra a ogni login senza
        // password, prima che il PC decida, anche dove entra senza clic.
        "wait-remote-accept-nook" => Some(Evento::Attesa),
        "insecure-connection-nocancel-hasclose" => Some(Evento::NonCifrata),
        t if t.starts_with("terminal-admin-login") => Some(Evento::Errore(
            "il PC chiede il login come amministratore: il CLI non lo fa mai".to_owned(),
        )),
        "re-input-password" | "input-2fa" => Some(Evento::Errore(format!(
            "il PC chiede una password o un codice ({titolo}): il CLI entra solo col clic del cliente"
        ))),
        t if t.contains("error") || t.starts_with("session-login") => {
            Some(Evento::Errore(format!("{titolo}: {testo}")))
        }
        _ => None,
    }
}

/// La risposta del terminale del PC; i dati compressi si decomprimono come in
/// src/flutter.rs (`handle_terminal_response`).
pub(super) fn evento_da_risposta(risposta: TerminalResponse) -> Option<Evento> {
    match risposta.union? {
        Union::Opened(o) if o.success => Some(Evento::Aperto(o.message)),
        Union::Opened(o) => Some(Evento::Errore(format!(
            "terminale non aperto: {}",
            o.message
        ))),
        Union::Data(d) => {
            let dati = if d.compressed {
                hbb_common::compress::decompress(&d.data)
            } else {
                d.data.to_vec()
            };
            if d.terminal_id == crate::remotek::exec::TERMINAL_ID_STDERR {
                Some(Evento::Stderr(dati))
            } else {
                Some(Evento::Dati(dati))
            }
        }
        Union::Closed(c) => Some(Evento::Chiuso(c.exit_code)),
        Union::Error(e) if e.message == crate::remotek::uscita_terminale::NON_PARTITO => {
            Some(Evento::NonPartito)
        }
        Union::Error(e) => Some(Evento::Errore(format!("terminale: {}", e.message))),
        _ => None,
    }
}

/// I byte di stdin per il terminale, anche non UTF-8: lo stesso messaggio di
/// `Session::send_terminal_input`, che prende una `String`.
fn messaggio_dati(dati: &[u8]) -> Message {
    let mut azione = TerminalAction::new();
    azione.set_data(TerminalData {
        terminal_id: ID_TERMINALE,
        data: bytes::Bytes::copy_from_slice(dati),
        ..Default::default()
    });
    let mut msg = Message::new();
    msg.set_terminal_action(azione);
    msg
}

impl InvokeUiSession for Gestore {
    fn set_cursor_data(&self, _: CursorData) {}
    fn set_cursor_id(&self, _: String) {}
    fn set_cursor_position(&self, _: CursorPosition) {}
    fn set_display(&self, _: i32, _: i32, _: i32, _: i32, _: bool, _: f64) {}
    fn switch_display(&self, _: &SwitchDisplay) {}
    fn set_peer_info(&self, pi: &PeerInfo) {
        let dichiara = crate::remotek::dichiarazione::dichiara_il_terminale(&pi.platform_additions);
        self.pc_dichiara.store(dichiara, Ordering::SeqCst);
        let script =
            crate::remotek::dichiarazione::dichiara_lo_script_nella_scelta(&pi.platform_additions);
        self.pc_script_nella_scelta.store(script, Ordering::SeqCst);
    }
    fn set_displays(&self, _: &Vec<DisplayInfo>) {}
    fn set_platform_additions(&self, _: &str) {}
    fn on_connected(&self, conn_type: ConnType) {
        if conn_type == ConnType::TERMINAL {
            let dichiara = self.pc_dichiara.load(Ordering::SeqCst);
            if dichiara && self.exec && !self.pc_script_nella_scelta.load(Ordering::SeqCst) {
                self.invia(Evento::ScriptNonNellaScelta);
            } else {
                self.invia(Evento::Connesso(dichiara));
            }
        }
    }
    fn update_privacy_mode(&self) {}
    fn set_permission(&self, _: &str, _: bool) {}
    fn close_success(&self) {}
    fn update_quality_status(&self, _: QualityStatus) {}
    fn set_connection_type(&self, _: bool, _: bool, _: &str) {}
    fn set_fingerprint(&self, _: String) {}
    fn job_error(&self, _: i32, _: String, _: i32) {}
    fn job_done(&self, _: i32, _: i32) {}
    fn clear_all_jobs(&self) {}
    fn new_message(&self, _: String) {}
    fn update_transfer_list(&self) {}
    fn load_last_job(&self, _: i32, _: &str, _: bool) {}
    fn update_folder_files(&self, _: i32, _: &Vec<FileEntry>, _: String, _: bool, _: bool) {}
    fn confirm_delete_files(&self, _: i32, _: i32, _: String) {}
    fn override_file_confirm(&self, _: i32, _: i32, _: String, _: bool, _: bool) {}
    fn update_block_input_state(&self, _: bool) {}
    fn job_progress(&self, _: i32, _: i32, _: f64, _: f64) {}
    fn adapt_size(&self) {}
    fn on_rgba(&self, _: usize, _: &mut scrap::ImageRgb) {}
    fn msgbox(&self, tipo: &str, titolo: &str, testo: &str, _: &str, _: bool) {
        if let Some(evento) = evento_da_msgbox(tipo, titolo, testo, self.amministratore) {
            self.invia(evento);
        }
    }
    fn cancel_msgbox(&self, _: &str) {}
    fn switch_back(&self, _: &str) {}
    fn portable_service_running(&self, _: bool) {}
    fn on_voice_call_started(&self) {}
    fn on_voice_call_closed(&self, _: &str) {}
    fn on_voice_call_waiting(&self) {}
    fn on_voice_call_incoming(&self) {}
    fn get_rgba(&self, _: usize) -> *const u8 {
        std::ptr::null()
    }
    fn next_rgba(&self, _: usize) {}
    // Il build Windows compila la libreria con `flutter` e `vram` (build.py).
    #[cfg(all(feature = "vram", feature = "flutter"))]
    fn on_texture(&self, _: usize, _: *mut std::ffi::c_void) {}
    fn set_multiple_windows_session(&self, _: Vec<WindowsSession>) {}
    fn set_current_display(&self, _: i32) {}
    #[cfg(feature = "flutter")]
    fn is_multi_ui_session(&self) -> bool {
        false
    }
    fn update_record_status(&self, _: bool) {}
    fn printer_request(&self, _: i32, _: String) {}
    fn handle_screenshot_resp(&self, _: String, _: String) {}
    fn handle_terminal_response(&self, risposta: TerminalResponse) {
        if let Some(evento) = evento_da_risposta(risposta) {
            self.invia(evento);
        }
    }
}

/// La sessione come in `session_add`: nessuna password (`password` vuota,
/// nessuna password condivisa), tipo TERMINAL (o PORT_FORWARD per il
/// tunnel), mai amministratore.
pub(super) fn sessione(
    id: &str,
    gestore: Gestore,
    conn_type: ConnType,
) -> Result<Session<Gestore>, String> {
    // `LoginConfigHandler::initialize` legge IS_TERMINAL_ADMIN (src/client.rs).
    std::env::remove_var("IS_TERMINAL_ADMIN");
    let session = Session {
        ui_handler: gestore,
        server_keyboard_enabled: Arc::new(RwLock::new(true)),
        server_file_transfer_enabled: Arc::new(RwLock::new(true)),
        server_clipboard_enabled: Arc::new(RwLock::new(true)),
        reconnect_count: Arc::new(AtomicUsize::new(0)),
        ..Default::default()
    };
    let mut lc = session
        .lc
        .write()
        .map_err(|_| "sessione non disponibile".to_owned())?;
    lc.initialize(id.to_owned(), conn_type, None, false, None, None, None);
    if lc.is_terminal_admin || !session.password.is_empty() {
        return Err("sessione come amministratore o con password: rifiutata".to_owned());
    }
    // Le password che `handle_hash` userebbe senza il clic del cliente
    // (src/client.rs): quella salvata per il PC, sulla configurazione che
    // `initialize` ha caricato con l'ID normalizzato, e quella della rubrica.
    if lc.remember || !lc.load_config().password.is_empty() {
        return Err(format!(
            "il PC {} ha una password salvata: il CLI non la usa",
            lc.get_id()
        ));
    }
    let token = LocalConfig::get_option("access_token");
    if rubrica_ha_password(&Ab::load(), &token, lc.get_id()) {
        return Err(format!(
            "la rubrica ha una password per il PC {}: il CLI non la usa",
            lc.get_id()
        ));
    }
    // Il servizio terminale di un CLI ucciso prima di `PeerConfig::remove`:
    // `create_login_msg` (src/client.rs) ne manderebbe l'id e il PC
    // riattaccherebbe la sessione a quella shell.
    let mut config = lc.load_config();
    if config
        .options
        .remove(lc.get_key_terminal_service_id())
        .is_some()
    {
        lc.save_config(config);
    }
    drop(lc);
    Ok(session)
}

/// Le condizioni con cui `try_get_password_from_personal_ab` (src/client.rs)
/// usa l'hash della rubrica personale in cache al posto del clic.
fn rubrica_ha_password(ab: &Ab, token: &str, id: &str) -> bool {
    !token.is_empty()
        && token == ab.access_token
        && ab
            .ab_entries
            .iter()
            .filter(|e| e.personal())
            .any(|e| e.peers.iter().any(|p| p.id == id && !p.hash.is_empty()))
}

/// Prima i controlli locali di [`sessione`], poi il lasciapassare: un
/// rifiuto locale non lascia nell'API un lasciapassare mai usato, e quello
/// chiesto (vale 60 secondi) arriva subito prima di collegarsi.
fn prepara<S>(
    locali: impl FnOnce() -> Result<S, String>,
    chiedi: Option<impl FnOnce() -> Result<String, String>>,
) -> Result<(S, Option<String>), String> {
    let session = locali()?;
    let lasciapassare = chiedi.map(|c| c()).transpose()?;
    Ok((session, lasciapassare))
}

/// Apre il terminale del PC `id` e restituisce il codice d'uscita. Il
/// lasciapassare (`--amministratore`, chiesto con `chiedi`) va solo nel login
/// (src/client.rs, `handle_hash`).
pub(super) fn terminale(
    id: &str,
    attesa: u64,
    righe: u32,
    colonne: u32,
    chiedi: Option<impl FnOnce() -> Result<String, String>>,
) -> Result<i32, String> {
    let amministratore = chiedi.is_some();
    con_sessione(id, amministratore, false, chiedi, |session, rx, tx| {
        attendi(session, rx, tx, Duration::from_secs(attesa), righe, colonne)
    })
}

/// Collega la sessione (controlli locali, poi il lasciapassare), fa girare
/// `corpo` sugli eventi e chiude.
fn con_sessione(
    id: &str,
    amministratore: bool,
    exec: bool,
    chiedi: Option<impl FnOnce() -> Result<String, String>>,
    corpo: impl FnOnce(
        &Session<Gestore>,
        &std::sync::mpsc::Receiver<Evento>,
        Sender<Evento>,
    ) -> Result<i32, String>,
) -> Result<i32, String> {
    let (tx, rx) = channel();
    let (session, lasciapassare) = prepara(
        || {
            sessione(
                id,
                Gestore {
                    eventi: Some(tx.clone()),
                    amministratore,
                    pc_dichiara: Arc::new(AtomicBool::new(false)),
                    exec,
                    pc_script_nella_scelta: Arc::new(AtomicBool::new(false)),
                },
                ConnType::TERMINAL,
            )
        },
        chiedi,
    )?;
    if let Some(l) = lasciapassare {
        session
            .lc
            .write()
            .map_err(|_| "sessione non disponibile".to_owned())?
            .remotek_lasciapassare = l;
    }
    // L'ID come lo usa il client (initialize toglie `/r` e simili).
    let id = session
        .lc
        .read()
        .map_err(|_| "sessione non disponibile".to_owned())?
        .get_id()
        .to_owned();
    let ciclo = session.clone();
    avvia_ciclo(FineSessione(tx.clone()), move || {
        let round = match ciclo.connection_round_state.lock() {
            Ok(mut stato) => stato.new_round(),
            Err(_) => return,
        };
        io_loop(ciclo, round);
    });
    let esito = corpo(&session, &rx, tx);
    session.close();
    // handle_peer_info salva la configurazione del PC (src/client.rs): il CLI
    // non ne tiene.
    PeerConfig::remove(&id);
    esito
}

/// Fa girare `ciclo` (`io_loop`) in un thread che tiene `fine` fino alla fine.
fn avvia_ciclo(
    fine: FineSessione,
    ciclo: impl FnOnce() + Send + 'static,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        // Anche dopo un panic o un'uscita senza msgbox: `attendi` e il
        // `Gestore` tengono un `tx`, quindi il canale non si chiude da solo.
        let _fine = fine;
        ciclo();
    })
}

/// Quando il thread di `io_loop` finisce, comunque finisca, `attendi` lo sa.
struct FineSessione(Sender<Evento>);

impl Drop for FineSessione {
    fn drop(&mut self) {
        // Fallisce solo se il CLI sta gia' uscendo.
        self.0
            .send(Evento::Errore("la sessione e' finita".to_owned()))
            .ok();
    }
}

fn attendi(
    session: &Session<Gestore>,
    rx: &std::sync::mpsc::Receiver<Evento>,
    tx: Sender<Evento>,
    attesa: Duration,
    righe: u32,
    colonne: u32,
) -> Result<i32, String> {
    let mut scadenza = Some(Instant::now() + attesa);
    let mut avvisato = false;
    let mut chiusura = false;
    let mut stdout = std::io::stdout().lock();
    loop {
        let evento = match scadenza {
            Some(s) => match rx.recv_timeout(s.saturating_duration_since(Instant::now())) {
                Ok(e) => e,
                // Stdin finita e il PC non ha mandato `TerminalClosed`.
                Err(RecvTimeoutError::Timeout) if chiusura => {
                    return Err("il PC non ha confermato la chiusura del terminale".to_owned())
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err(format!(
                        "nessuna risposta entro {} secondi",
                        attesa.as_secs()
                    ))
                }
                Err(RecvTimeoutError::Disconnected) => return Err("sessione chiusa".to_owned()),
            },
            None => rx.recv().map_err(|_| "sessione chiusa".to_owned())?,
        };
        match evento {
            Evento::Attesa if !avvisato => {
                eprintln!("remotek-cli: in attesa che il cliente accetti");
                avvisato = true;
            }
            Evento::Attesa => {}
            Evento::NonCifrata => {
                session.send(Data::RejectInsecureConnection);
                return Err("connessione non cifrata da un capo all'altro: rifiutata".to_owned());
            }
            Evento::Connesso(false) => {
                return Ok(rifiuta_il_pc_che_non_dichiara(&mut std::io::stderr()))
            }
            Evento::Connesso(true) => session.open_terminal(ID_TERMINALE, righe, colonne),
            // Solo con exec (`Gestore::exec`): il terminale non la guarda.
            Evento::ScriptNonNellaScelta => session.open_terminal(ID_TERMINALE, righe, colonne),
            Evento::Aperto(_) if scadenza.is_some() && !chiusura => {
                scadenza = None;
                leggi_stdin(session.clone(), tx.clone());
            }
            Evento::Aperto(_) => {}
            Evento::Dati(dati) | Evento::Stderr(dati) => {
                stdout
                    .write_all(&dati)
                    .and_then(|()| stdout.flush())
                    .map_err(|e| format!("stdout: {e}"))?;
            }
            Evento::Chiuso(codice) => return Ok(codice_uscita(codice)),
            Evento::NonPartito => {
                eprintln!(
                    "remotek-cli: {}",
                    crate::remotek::uscita_terminale::NON_PARTITO
                );
                return Ok(super::USCITA_NON_PARTITO);
            }
            Evento::Errore(e) => return Err(e),
            Evento::FineInput => {
                session.close_terminal(ID_TERMINALE);
                chiusura = true;
                scadenza = Some(Instant::now() + ATTESA_CHIUSURA);
            }
        }
    }
}

/// Cosa dice il CLI quando il PC non dichiara trascrizione e vista.
const NON_DICHIARA: &str = "il PC non dichiara la trascrizione e la vista del terminale: aggiornare Remotek sul PC (remotek-1.4.9-7 o seguente)";

/// Cosa dice il CLI con exec quando il PC non dichiara lo script nella scelta.
const SCRIPT_NON_NELLA_SCELTA: &str = "il PC non mostra lo script al cliente prima della scelta: exec non parte, aggiornare Remotek sul PC (remotek-1.4.9-9 o seguente)";

/// Il PC non dichiara trascrizione e vista del terminale (ADR-0034): il CLI
/// non gli manda niente, lo scrive su `avvisi` (stderr) e restituisce
/// `USCITA_NON_DICHIARA`. Non c'e' modo di forzare. La sessione la chiude
/// `con_sessione`.
fn rifiuta_il_pc_che_non_dichiara(avvisi: &mut impl Write) -> i32 {
    rifiuta_con(avvisi, NON_DICHIARA)
}

/// Come [`rifiuta_il_pc_che_non_dichiara`], con exec, per il PC che non
/// dichiara lo script nella scelta (ADR-0035): niente `OpenTerminal`.
fn rifiuta_il_pc_senza_script_nella_scelta(avvisi: &mut impl Write) -> i32 {
    rifiuta_con(avvisi, SCRIPT_NON_NELLA_SCELTA)
}

fn rifiuta_con(avvisi: &mut impl Write, perche: &str) -> i32 {
    // Se stderr non si scrive, il codice d'uscita dice comunque il motivo.
    writeln!(avvisi, "remotek-cli: {perche}")
        .and_then(|()| avvisi.flush())
        .ok();
    super::USCITA_NON_DICHIARA
}

/// Il codice del terminale come codice d'uscita del CLI: Linux tiene solo
/// 8 bit, quindi fuori da 0-255 (anche -1 o un codice Windows) si esce con
/// 255 e mai con 0.
fn codice_uscita(codice: i32) -> i32 {
    if (0..=255).contains(&codice) {
        codice
    } else {
        255
    }
}

/// Stdin va al terminale cosi' com'e', a blocchi; alla fine manda `FineInput`.
fn leggi_stdin(session: Session<Gestore>, tx: Sender<Evento>) {
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let mut buf = [0u8; 4096];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => session.send(Data::Message(messaggio_dati(&buf[..n]))),
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    // Fallisce solo se il CLI sta gia' uscendo.
                    tx.send(Evento::Errore(format!("stdin: {e}"))).ok();
                    return;
                }
            }
        }
        // Fallisce solo se il CLI sta gia' uscendo.
        tx.send(Evento::FineInput).ok();
    });
}

/// Lo script piu' grande che il PC accetta (`exec::MASSIMO_SCRIPT`).
pub(super) const MASSIMO_SCRIPT: usize = crate::remotek::exec::MASSIMO_SCRIPT;

/// Perche' lo script non si legge: il codice d'uscita e il messaggio.
#[derive(Debug, PartialEq)]
pub(super) enum ScriptNonLetto {
    /// stdin non si legge: un errore, come la rete o la configurazione.
    Stdin(String),
    /// Oltre i limiti del PC: un uso sbagliato, come gli argomenti.
    OltreILimiti(String),
}

impl ScriptNonLetto {
    pub(super) fn codice(&self) -> i32 {
        match self {
            Self::Stdin(_) => super::USCITA_ERRORE,
            Self::OltreILimiti(_) => super::USCITA_USO,
        }
    }

    pub(super) fn messaggio(&self) -> &str {
        match self {
            Self::Stdin(m) | Self::OltreILimiti(m) => m,
        }
    }
}

/// Tutto stdin, prima di collegarsi: dentro i limiti del PC
/// (`script_exec::oltre_i_limiti`).
pub(super) fn leggi_script(r: impl Read) -> Result<Vec<u8>, ScriptNonLetto> {
    let mut script = Vec::new();
    r.take(MASSIMO_SCRIPT as u64 + 1)
        .read_to_end(&mut script)
        .map_err(|e| ScriptNonLetto::Stdin(format!("stdin: {e}")))?;
    if crate::remotek::script_exec::oltre_i_limiti(&script) {
        use crate::remotek::script_exec::MASSIMO_RIGHE;
        let kib = MASSIMO_SCRIPT / 1024;
        return Err(ScriptNonLetto::OltreILimiti(format!(
            "lo script supera {kib} KiB o {MASSIMO_RIGHE} righe"
        )));
    }
    Ok(script)
}

/// Il lasciapassare e' di exec: `v` = 2 con `capacita` = `exec`. Si legge
/// solo il payload; la firma la verifica il PC (src/remotek/lasciapassare.rs).
pub(super) fn lasciapassare_di_exec(lasciapassare: &str) -> bool {
    use hbb_common::base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    lasciapassare
        .split_once('.')
        .and_then(|(p, _)| URL_SAFE_NO_PAD.decode(p).ok())
        .and_then(|p| serde_json::from_slice::<serde_json::Value>(&p).ok())
        .is_some_and(|p| p["v"] == 2 && p["capacita"] == "exec")
}

/// Esegue `script` sul PC `id` e restituisce il codice d'uscita. Il
/// lasciapassare (sempre, `chiedi`) si chiede dopo i controlli locali.
pub(super) fn exec(
    id: &str,
    attesa: u64,
    amministratore: bool,
    script: Vec<u8>,
    chiedi: impl FnOnce() -> Result<String, String>,
) -> Result<i32, String> {
    // Il lasciapassare e lo script nel login: il cliente vede lo script nella
    // finestra della richiesta, prima di scegliere (ADR-0035).
    let chiedi = || {
        let l = chiedi()?;
        if lasciapassare_di_exec(&l) {
            Ok(crate::remotek::script_al_login::componi(&l, &script))
        } else {
            Err("l'API non conosce exec".to_owned())
        }
    };
    con_sessione(id, amministratore, true, Some(chiedi), |session, rx, _| {
        let (mut out, mut err) = (std::io::stdout().lock(), std::io::stderr().lock());
        attendi_exec(
            session,
            rx,
            Duration::from_secs(attesa),
            &script,
            &mut out,
            &mut err,
        )
    })
}

/// Il ciclo di exec: lo script parte solo dopo il `TerminalOpened` con
/// `exec::APERTO`; stdout e stderr del PC vanno a `out` e `err`, byte per
/// byte; si esce col codice del `TerminalClosed`. Un `TerminalClosed` prima
/// dello script e' un errore: il codice non e' dello script.
fn attendi_exec(
    session: &Session<Gestore>,
    rx: &std::sync::mpsc::Receiver<Evento>,
    attesa: Duration,
    script: &[u8],
    out: &mut impl Write,
    err: &mut impl Write,
) -> Result<i32, String> {
    let mut scadenza = Some(Instant::now() + attesa);
    let mut avvisato = false;
    loop {
        let evento = match scadenza {
            Some(s) => match rx.recv_timeout(s.saturating_duration_since(Instant::now())) {
                Ok(e) => e,
                Err(RecvTimeoutError::Timeout) => {
                    return Err(format!(
                        "nessuna risposta entro {} secondi",
                        attesa.as_secs()
                    ))
                }
                Err(RecvTimeoutError::Disconnected) => return Err("sessione chiusa".to_owned()),
            },
            None => rx.recv().map_err(|_| "sessione chiusa".to_owned())?,
        };
        let scrivi = |w: &mut dyn Write, d: &[u8], nome: &str| {
            w.write_all(d)
                .and_then(|()| w.flush())
                .map_err(|e| format!("{nome}: {e}"))
        };
        match evento {
            Evento::Attesa if !avvisato => {
                eprintln!("remotek-cli: in attesa che il cliente accetti");
                avvisato = true;
            }
            Evento::Attesa | Evento::FineInput => {}
            Evento::NonCifrata => {
                session.send(Data::RejectInsecureConnection);
                return Err("connessione non cifrata da un capo all'altro: rifiutata".to_owned());
            }
            Evento::Connesso(false) => return Ok(rifiuta_il_pc_che_non_dichiara(err)),
            Evento::ScriptNonNellaScelta => {
                return Ok(rifiuta_il_pc_senza_script_nella_scelta(err))
            }
            Evento::Connesso(true) => {
                session.open_terminal(ID_TERMINALE, RIGHE_PREDEFINITE, COLONNE_PREDEFINITE)
            }
            Evento::Aperto(m) if m != crate::remotek::exec::APERTO => {
                session.close_terminal(ID_TERMINALE);
                return Err("il PC non esegue exec (client vecchio?)".to_owned());
            }
            Evento::Aperto(_) if scadenza.is_some() => {
                scadenza = None;
                for blocco in script.chunks(BLOCCO_SCRIPT) {
                    session.send(Data::Message(messaggio_dati(blocco)));
                }
                // Il `TerminalData` vuoto: la fine dello script.
                session.send(Data::Message(messaggio_dati(&[])));
            }
            Evento::Aperto(_) => {}
            Evento::Dati(d) => scrivi(out, &d, "stdout")?,
            Evento::Stderr(d) => scrivi(err, &d, "stderr")?,
            // `scadenza` c'e' finche' lo script non e' partito.
            Evento::Chiuso(_) if scadenza.is_some() => {
                return Err("il PC ha chiuso il terminale prima dello script".to_owned())
            }
            Evento::Chiuso(codice) => return Ok(codice_uscita(codice)),
            Evento::NonPartito => {
                eprintln!(
                    "remotek-cli: {}",
                    crate::remotek::uscita_terminale::NON_PARTITO
                );
                return Ok(super::USCITA_NON_PARTITO);
            }
            Evento::Errore(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hbb_common::tokio::sync::mpsc::unbounded_channel;

    use super::super::AMBIENTE;

    fn risposta(union: Union) -> TerminalResponse {
        TerminalResponse {
            union: Some(union),
            ..Default::default()
        }
    }

    fn dati(data: Vec<u8>, compressed: bool) -> TerminalResponse {
        risposta(Union::Data(TerminalData {
            data: data.into(),
            compressed,
            ..Default::default()
        }))
    }

    /// Il ciclo del CLI su eventi gia' in coda, con la sessione che registra
    /// cio' che manderebbe a `io_loop`.
    fn ciclo(eventi: Vec<Evento>, attesa: u64) -> (Result<i32, String>, Vec<Data>) {
        let session = Session::<Gestore>::default();
        let (tx_io, mut rx_io) = unbounded_channel();
        *session.sender.write().unwrap() = Some(tx_io);
        let (tx, rx) = channel();
        for e in eventi {
            tx.send(e).unwrap();
        }
        let esito = attendi(&session, &rx, tx, Duration::from_secs(attesa), 24, 80);
        let mut inviati = Vec::new();
        while let Ok(d) = rx_io.try_recv() {
            inviati.push(d);
        }
        (esito, inviati)
    }

    #[test]
    fn dati_compressi_e_no_cosi_come_sono() {
        let testo = "ls -l\r\nàè\x1b[0m".as_bytes().to_vec();
        let compresso = hbb_common::compress::compress(&testo);
        assert_eq!(
            evento_da_risposta(dati(compresso, true)),
            Some(Evento::Dati(testo.clone()))
        );
        assert_eq!(
            evento_da_risposta(dati(vec![0xff, 0x00], false)),
            Some(Evento::Dati(vec![0xff, 0x00]))
        );
    }

    #[test]
    fn codice_fuori_da_0_255_mai_zero() {
        assert_eq!(ciclo(vec![Evento::Chiuso(256)], 5).0, Ok(255));
        assert_eq!(ciclo(vec![Evento::Chiuso(-1)], 5).0, Ok(255));
        assert_eq!(ciclo(vec![Evento::Chiuso(0)], 5).0, Ok(0));
    }

    #[test]
    fn fine_input_senza_conferma_del_pc_e_un_errore() {
        let (esito, inviati) = ciclo(vec![Evento::FineInput], 5);
        assert!(esito.is_err());
        // `close_terminal` mandato a io_loop.
        assert!(matches!(&inviati[..], [Data::Message(m)] if m.has_terminal_action()));
    }

    #[test]
    fn esce_col_codice_del_terminale() {
        let chiuso = risposta(Union::Closed(TerminalClosed {
            exit_code: 3,
            ..Default::default()
        }));
        assert_eq!(evento_da_risposta(chiuso), Some(Evento::Chiuso(3)));
        let (esito, inviati) = ciclo(vec![Evento::Connesso(true), Evento::Chiuso(3)], 5);
        assert_eq!(esito, Ok(3));
        // Connesso: `open_terminal` con righe e colonne.
        assert!(matches!(&inviati[..], [Data::Message(m)] if m.has_terminal_action()));
    }

    #[test]
    fn il_terminale_non_partito_esce_col_suo_codice() {
        let non_partito = crate::remotek::uscita_terminale::risposta_chiusura(
            ID_TERMINALE,
            crate::remotek::uscita_terminale::HELPER_NON_PARTITO,
        );
        assert_eq!(evento_da_risposta(non_partito), Some(Evento::NonPartito));
        let (esito, _) = ciclo(vec![Evento::Connesso(true), Evento::NonPartito], 5);
        assert_eq!(esito, Ok(super::super::USCITA_NON_PARTITO));
        // Mai 0 e mai il 255 di un codice della shell fuori da 0-255.
        assert_ne!(super::super::USCITA_NON_PARTITO, 0);
        assert_ne!(
            super::super::USCITA_NON_PARTITO,
            codice_uscita(crate::remotek::uscita_terminale::HELPER_NON_PARTITO)
        );
        assert_ne!(
            super::super::USCITA_NON_PARTITO,
            super::super::USCITA_ERRORE
        );
    }

    #[test]
    fn errore_rifiuto_e_attesa_scaduta_escono_con_errore() {
        let errore = risposta(Union::Error(TerminalError {
            message: "no".into(),
            ..Default::default()
        }));
        assert!(matches!(
            evento_da_risposta(errore),
            Some(Evento::Errore(_))
        ));
        let non_aperto = risposta(Union::Opened(TerminalOpened::default()));
        assert!(matches!(
            evento_da_risposta(non_aperto),
            Some(Evento::Errore(_))
        ));
        let rifiuto = evento_da_msgbox("error", "Connection Error", "Connection declined", false);
        assert!(matches!(rifiuto, Some(Evento::Errore(_))));
        for tipo in [
            "re-input-password",
            "input-2fa",
            "terminal-admin-login",
            "terminal-admin-login-password",
        ] {
            assert!(
                matches!(
                    evento_da_msgbox(tipo, "", "", false),
                    Some(Evento::Errore(_))
                ),
                "{tipo}"
            );
        }
        assert!(ciclo(vec![Evento::Errore("x".into())], 5).0.is_err());
        // Nessuna risposta del cliente entro --attesa.
        let attesa = evento_da_msgbox("wait-remote-accept-nook", "", "", false);
        assert_eq!(attesa, Some(Evento::Attesa));
        // Il login senza password del CLI, anche senza clic (collaudo 33(a)).
        let hash = evento_da_msgbox("input-password", "Password Required", "", false);
        assert_eq!(hash, None);
        assert!(ciclo(vec![Evento::Attesa], 0).0.is_err());
    }

    #[test]
    fn i_rifiuti_del_portatile_dicono_cosa_fare() {
        use crate::remotek::token_terminale::{
            RIFIUTO_NIENTE_AMMINISTRATORE, RIFIUTO_SOLO_AMMINISTRATORE,
        };
        let solo = consiglio(RIFIUTO_SOLO_AMMINISTRATORE, false, "Prova").unwrap();
        assert!(solo.contains("rilancia con --amministratore"), "{solo}");
        assert!(solo.contains("Prova non installato"), "{solo}");
        // Col lasciapassare gia' mandato il PC l'ha scartato: non basta
        // aggiungere --amministratore.
        let scartato = consiglio(RIFIUTO_SOLO_AMMINISTRATORE, true, "Prova").unwrap();
        assert!(!scartato.contains("rilancia"), "{scartato}");
        assert!(
            scartato.contains("lasciapassare rifiutato dal PC"),
            "{scartato}"
        );
        let niente = consiglio(RIFIUTO_NIENTE_AMMINISTRATORE, true, "Prova").unwrap();
        assert!(
            niente.contains("rilancia senza --amministratore"),
            "{niente}"
        );
        assert!(niente.contains("fai installare Prova"), "{niente}");
        // Il marchio solo da APP_NAME.
        for c in [&solo, &scartato, &niente] {
            assert!(!c.contains("Remotek"), "{c}");
        }
        assert_eq!(consiglio("altro", true, "Prova"), None);
        // Come li mostra handle_login_error (src/client.rs).
        let app = super::super::marchio();
        assert_eq!(
            evento_da_msgbox("error", "Login Error", RIFIUTO_SOLO_AMMINISTRATORE, false),
            Some(Evento::Errore(
                consiglio(RIFIUTO_SOLO_AMMINISTRATORE, false, &app).unwrap()
            ))
        );
        assert_eq!(
            evento_da_msgbox("error", "Login Error", RIFIUTO_NIENTE_AMMINISTRATORE, true),
            Some(Evento::Errore(
                consiglio(RIFIUTO_NIENTE_AMMINISTRATORE, true, &app).unwrap()
            ))
        );
        // Un altro errore di login resta com'e'.
        assert_eq!(
            evento_da_msgbox("error", "Login Error", "altro", true),
            Some(Evento::Errore("Login Error: altro".to_owned()))
        );
        assert!(ciclo(vec![Evento::Errore(solo)], 5).0.is_err());
    }

    #[test]
    fn fine_del_ciclo_senza_msgbox_e_un_errore() {
        for panic in [false, true] {
            let (tx, rx) = channel();
            let (via, attesa) = channel::<()>();
            let ciclo = avvia_ciclo(FineSessione(tx.clone()), move || {
                attesa.recv().unwrap();
                assert!(!panic, "io_loop in panic");
            });
            // Finche' il ciclo gira la guardia tace: senza `let _fine = fine;`
            // in `avvia_ciclo` parlerebbe subito.
            assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
            via.send(()).unwrap();
            assert_eq!(ciclo.join().is_err(), panic);
            let session = Session::<Gestore>::default();
            let esito = attendi(&session, &rx, tx, Duration::from_secs(5), 24, 80);
            assert_eq!(esito, Err("la sessione e' finita".to_owned()));
        }
    }

    #[test]
    fn connessione_non_cifrata_rifiutata() {
        let evento = evento_da_msgbox("insecure-connection-nocancel-hasclose", "", "", false);
        assert_eq!(evento, Some(Evento::NonCifrata));
        let (esito, inviati) = ciclo(vec![Evento::NonCifrata], 5);
        assert!(esito.is_err());
        assert!(matches!(&inviati[..], [Data::RejectInsecureConnection]));
    }

    #[test]
    fn login_senza_password_e_mai_amministratore() {
        let _ambiente = AMBIENTE.lock().unwrap();
        // Anche con IS_TERMINAL_ADMIN nell'ambiente la sessione non e' admin.
        std::env::set_var("IS_TERMINAL_ADMIN", "Y");
        let session = sessione(
            "remotek-cli-test-id",
            Gestore::default(),
            ConnType::TERMINAL,
        )
        .unwrap();
        assert!(std::env::var("IS_TERMINAL_ADMIN").is_err());
        let lc = session.lc.read().unwrap();
        assert_eq!(lc.conn_type, ConnType::TERMINAL);
        assert!(!lc.is_terminal_admin);
        assert!(lc.password.is_empty() && session.password.is_empty());
        // Nessuna password di default per le connessioni (custom.txt).
        let p = crate::ui_interface::get_builtin_option(
            hbb_common::config::keys::OPTION_DEFAULT_CONNECT_PASSWORD,
        );
        assert!(p.is_empty());
    }

    #[test]
    fn password_salvata_rifiutata_anche_con_id_scritto_diverso() {
        let _ambiente = AMBIENTE.lock().unwrap();
        let id = "remotek-cli-test-pw";
        let config = PeerConfig {
            password: vec![1, 2, 3].into(),
            ..Default::default()
        };
        config.store(id);
        // `initialize` toglie `/r` e carica la configurazione di `id`.
        let esito = sessione(&format!("{id}/r"), Gestore::default(), ConnType::TERMINAL);
        PeerConfig::remove(id);
        assert!(esito.is_err());
    }

    #[test]
    fn servizio_terminale_rimasto_non_si_riattacca() {
        let _ambiente = AMBIENTE.lock().unwrap();
        let id = "remotek-cli-test-servizio";
        let mut config = PeerConfig::default();
        config
            .options
            .insert("terminal-service-id".into(), "ts_vecchio".into());
        config.store(id);
        let session = sessione(id, Gestore::default(), ConnType::TERMINAL).unwrap();
        let lc = session.lc.read().unwrap();
        // Il valore che `create_login_msg` mette in `Terminal::service_id`.
        let mandato = lc.get_option(lc.get_key_terminal_service_id());
        let salvato = PeerConfig::load(id)
            .options
            .get("terminal-service-id")
            .cloned();
        PeerConfig::remove(id);
        assert_eq!(mandato, "");
        assert_eq!(salvato, None);
    }

    #[test]
    fn password_della_rubrica_rifiutata() {
        let rubrica = |token: &str, nome: &str, hash: &str| {
            let mut ab = Ab {
                access_token: token.into(),
                ..Default::default()
            };
            let mut voce = hbb_common::config::AbEntry {
                name: nome.into(),
                ..Default::default()
            };
            voce.peers.push(hbb_common::config::AbPeer {
                id: "123".into(),
                hash: hash.into(),
                ..Default::default()
            });
            ab.ab_entries.push(voce);
            ab
        };
        assert!(rubrica_ha_password(
            &rubrica("t", "My address book", "aGFzaA"),
            "t",
            "123"
        ));
        // Altro token, altro ID, rubrica non personale o senza hash: niente password.
        assert!(!rubrica_ha_password(
            &rubrica("t", "My address book", "aGFzaA"),
            "x",
            "123"
        ));
        assert!(!rubrica_ha_password(
            &rubrica("t", "My address book", "aGFzaA"),
            "t",
            "456"
        ));
        assert!(!rubrica_ha_password(
            &rubrica("t", "Condivisa", "aGFzaA"),
            "t",
            "123"
        ));
        assert!(!rubrica_ha_password(
            &rubrica("t", "My address book", ""),
            "t",
            "123"
        ));
        assert!(!rubrica_ha_password(
            &rubrica("", "My address book", "aGFzaA"),
            "",
            "123"
        ));
    }

    /// Il login che il CLI manda davvero: `handle_hash` come in io_loop.rs,
    /// con la password della sessione (vuota), verso un socket locale.
    fn login_mandato(lasciapassare: &str) -> hbb_common::message_proto::LoginRequest {
        login_mandato_per(lasciapassare, ConnType::TERMINAL, None).0
    }

    /// [`login_mandato`] per un tipo di connessione e, per il port forward,
    /// host e porta come li mette cli/tunnel.rs; con `login_del_tunnel`.
    fn login_mandato_per(
        lasciapassare: &str,
        conn_type: ConnType,
        port_forward: Option<(&str, i32)>,
    ) -> (hbb_common::message_proto::LoginRequest, bool) {
        let _ambiente = AMBIENTE.lock().unwrap();
        let session = sessione("remotek-cli-test-login", Gestore::default(), conn_type).unwrap();
        let del_tunnel = {
            let mut lc = session.lc.write().unwrap();
            lc.remotek_lasciapassare = lasciapassare.to_owned();
            if let Some((host, porta)) = port_forward {
                lc.port_forward = (host.to_owned(), porta);
            }
            crate::remotek::tunnel::login_del_tunnel(&lc)
        };
        let rt = hbb_common::tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let login = rt.block_on(async {
            use hbb_common::tokio::net::{TcpListener, TcpStream};
            let ascolto = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let indirizzo = ascolto.local_addr().unwrap();
            let cliente = TcpStream::connect(indirizzo).await.unwrap();
            let (server, _) = ascolto.accept().await.unwrap();
            let mut peer =
                hbb_common::Stream::Tcp(hbb_common::tcp::FramedStream::from(cliente, indirizzo));
            let mut server = hbb_common::tcp::FramedStream::from(server, indirizzo);
            let hash = Hash {
                salt: "sale".into(),
                challenge: "sfida".into(),
                ..Default::default()
            };
            let password = session.password.clone();
            session.handle_hash(&password, hash, &mut peer).await;
            let dati = server.next().await.unwrap().unwrap();
            <Message as hbb_common::protobuf::Message>::parse_from_bytes(&dati).unwrap()
        });
        (login.login_request().clone(), del_tunnel)
    }

    #[test]
    fn il_lasciapassare_si_chiede_dopo_i_controlli_locali() {
        let mai = || -> Result<String, String> { panic!("lasciapassare chiesto") };
        assert_eq!(
            prepara(|| Err::<(), _>("password salvata".into()), Some(mai)),
            Err("password salvata".into())
        );
        assert_eq!(
            prepara(|| Ok(1), Some(|| Err("400".into()))),
            Err("400".into())
        );
        assert_eq!(
            prepara(|| Ok(1), Some(|| Ok("l".into()))),
            Ok((1, Some("l".into())))
        );
        assert_eq!(
            prepara(|| Ok(1), None::<fn() -> Result<String, String>>),
            Ok((1, None))
        );
    }

    #[test]
    fn login_senza_password_e_senza_os_login() {
        let lr = login_mandato("");
        assert!(lr.password.is_empty());
        let os = lr.os_login.as_ref();
        assert!(os.map_or(true, |o| o.username.is_empty() && o.password.is_empty()));
        assert!(lr.has_terminal());
    }

    #[test]
    fn login_col_lasciapassare_solo_in_os_login_password() {
        let lr = login_mandato("pay.firma");
        assert!(lr.password.is_empty());
        let os = lr.os_login.as_ref().unwrap();
        assert_eq!(
            (os.username.as_str(), os.password.as_str()),
            ("", "pay.firma")
        );
        assert!(lr.has_terminal());
    }

    #[test]
    fn login_del_tunnel_col_lasciapassare_solo_in_os_login_password() {
        // Come cli/tunnel.rs: port forward, host vuoto e porta 0.
        let (lr, del_tunnel) =
            login_mandato_per("pay.firma", ConnType::PORT_FORWARD, Some(("", 0)));
        assert!(del_tunnel);
        assert!(lr.password.is_empty());
        let os = lr.os_login.as_ref().unwrap();
        assert_eq!(
            (os.username.as_str(), os.password.as_str()),
            ("", "pay.firma")
        );
        let pf = lr.port_forward();
        assert!(lr.has_port_forward() && pf.host.is_empty() && pf.port == 0);
        assert!(crate::remotek::tunnel::forma_del_tunnel(&pf.host, pf.port));
    }

    #[test]
    fn fuori_dalla_forma_del_tunnel_il_lasciapassare_non_parte() {
        // Il port forward dei tecnici (host o porta veri), il tunnel senza
        // lasciapassare, il terminale: login_del_tunnel falso, e col port
        // forward il lasciapassare non va nel login.
        let casi = [
            (
                "pay.firma",
                ConnType::PORT_FORWARD,
                Some(("localhost", 3389)),
            ),
            ("pay.firma", ConnType::PORT_FORWARD, Some(("", 3389))),
            ("pay.firma", ConnType::PORT_FORWARD, Some(("", 1433))),
            ("pay.firma", ConnType::PORT_FORWARD, Some(("10.0.0.5", 0))),
            ("", ConnType::PORT_FORWARD, Some(("", 0))),
            ("pay.firma", ConnType::TERMINAL, Some(("", 0))),
        ];
        for (lasciapassare, conn_type, port_forward) in casi {
            let caso = (lasciapassare, conn_type, port_forward);
            let (lr, del_tunnel) = login_mandato_per(lasciapassare, conn_type, port_forward);
            assert!(!del_tunnel, "{:?}", caso);
            if conn_type == ConnType::PORT_FORWARD {
                let os = lr.os_login.as_ref();
                assert!(os.map_or(true, |o| o.password != "pay.firma"), "{:?}", caso);
            }
        }
    }

    /// Il ciclo di exec su eventi gia' in coda: esito, cio' che va a io_loop,
    /// stdout e stderr.
    fn ciclo_exec(eventi: Vec<Evento>) -> (Result<i32, String>, Vec<Data>, Vec<u8>, Vec<u8>) {
        let session = Session::<Gestore>::default();
        let (tx_io, mut rx_io) = unbounded_channel();
        *session.sender.write().unwrap() = Some(tx_io);
        let (tx, rx) = channel();
        for e in eventi {
            tx.send(e).unwrap();
        }
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let script = vec![b'x'; BLOCCO_SCRIPT + 1];
        let esito = attendi_exec(
            &session,
            &rx,
            Duration::from_secs(5),
            &script,
            &mut out,
            &mut err,
        );
        let mut inviati = Vec::new();
        while let Ok(d) = rx_io.try_recv() {
            inviati.push(d);
        }
        (esito, inviati, out, err)
    }

    /// I `TerminalData` mandati, in ordine.
    fn dati_mandati(inviati: &[Data]) -> Vec<Vec<u8>> {
        inviati
            .iter()
            .filter_map(|d| match d {
                Data::Message(m) => match &m.terminal_action().union {
                    Some(terminal_action::Union::Data(d)) => Some(d.data.to_vec()),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    #[test]
    fn script_letto_tutto_dentro_i_limiti() {
        assert_eq!(leggi_script(&b"Get-Date"[..]), Ok(b"Get-Date".to_vec()));
        let esatto = vec![b'#'; MASSIMO_SCRIPT];
        assert_eq!(
            leggi_script(&esatto[..]).map(|s| s.len()),
            Ok(MASSIMO_SCRIPT)
        );
        let oltre = vec![b'#'; MASSIMO_SCRIPT + 1];
        let limiti = format!(
            "{} KiB o {} righe",
            MASSIMO_SCRIPT / 1024,
            crate::remotek::script_exec::MASSIMO_RIGHE
        );
        let rifiuto = leggi_script(&oltre[..]).unwrap_err();
        assert!(rifiuto.messaggio().contains(&limiti));
        assert_eq!(rifiuto.codice(), super::super::USCITA_USO);
        let righe = "a\n".repeat(2000);
        assert!(leggi_script(righe.as_bytes()).is_ok());
        assert_eq!(
            leggi_script(format!("{righe}a").as_bytes()).map_err(|e| e.codice()),
            Err(super::super::USCITA_USO)
        );
    }

    #[test]
    fn stdin_illeggibile_esce_con_errore() {
        struct Rotto;
        impl Read for Rotto {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("rotto"))
            }
        }
        let e = leggi_script(Rotto).unwrap_err();
        assert_eq!(e, ScriptNonLetto::Stdin("stdin: rotto".to_owned()));
        assert_eq!(e.codice(), super::super::USCITA_ERRORE);
    }

    #[test]
    fn solo_il_lasciapassare_di_exec() {
        use hbb_common::base64::{
            engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD},
            Engine as _,
        };
        let con = |p: &str| format!("{}.firma", URL_SAFE_NO_PAD.encode(p));
        // Il vettore v2 del README di remotek-api (5eec81f): capacita' tunnel.
        let tunnel = "eyJ2IjoyLCJwYyI6Ijk5OTAwMDExMSIsImFnZW50ZSI6ImFnZW50ZS1tYXJpbyIsInRlY25pY28iOiJtYXJpbyIsIm1vZG8iOiJhbW1pbmlzdHJhdG9yZSIsImNhcGFjaXRhIjoidHVubmVsIiwicG9ydGUiOlsyMiwzMzg5XSwiZHVyYXRhIjozNjAwLCJlbWVzc28iOjE3OTEwMDAwMDAsInNjYWRlIjoxNzkxMDAwMDYwLCJub25jZSI6IkFBRUNBd1FGQmdjSUNRb0xEQTBPRHcifQ.kahITkYNGYmo1y2rCRX08LPvdZjiMCzrQPrcvZExTBF3g_eAAAYyPqPjjgl4pImNa-rcg5LmhkH_yqGFu_hOAw";
        assert!(!lasciapassare_di_exec(tunnel));
        assert!(lasciapassare_di_exec(&con(
            r#"{"v":2,"modo":"utente","capacita":"exec"}"#
        )));
        assert!(!lasciapassare_di_exec(&con(
            r#"{"v":1,"modo":"amministratore"}"#
        )));
        assert!(!lasciapassare_di_exec("non-un-lasciapassare"));
        // Payload non base64url (base64 standard, con `/`, di un payload exec),
        // base64 non JSON, `v` = 1, altra capacita'.
        let standard = r#"{"v":2,"modo":"utente","capacita":"exec","n":"??"}"#;
        assert!(lasciapassare_di_exec(&con(standard)));
        let standard = STANDARD_NO_PAD.encode(standard);
        assert!(standard.contains('/'));
        assert!(!lasciapassare_di_exec(&format!("{standard}.firma")));
        assert!(!lasciapassare_di_exec(&con("non e' JSON")));
        assert!(!lasciapassare_di_exec(&con(r#"{"v":1,"capacita":"exec"}"#)));
        assert!(!lasciapassare_di_exec(&con(
            r#"{"v":2,"modo":"utente","capacita":"terminale"}"#
        )));
        // Un payload non exec ferma tutto prima di collegarsi.
        let _ambiente = AMBIENTE.lock().unwrap();
        let fermato = exec("123456789", 1, false, Vec::new(), || Ok(tunnel.to_owned()));
        assert_eq!(fermato, Err("l'API non conosce exec".to_owned()));
    }

    #[test]
    fn niente_script_senza_remotek_exec() {
        let vecchio = Evento::Aperto("Terminal opened (helper mode)".into());
        let (esito, inviati, _, _) = ciclo_exec(vec![Evento::Connesso(true), vecchio]);
        assert_eq!(
            esito,
            Err("il PC non esegue exec (client vecchio?)".to_owned())
        );
        assert!(dati_mandati(&inviati).is_empty(), "nessuno script mandato");
        // open_terminal e close_terminal.
        assert_eq!(inviati.len(), 2);
    }

    #[test]
    fn exec_manda_lo_script_e_separa_i_due_flussi() {
        let aperto = risposta(Union::Opened(TerminalOpened {
            success: true,
            message: crate::remotek::exec::APERTO.into(),
            ..Default::default()
        }));
        let evento = evento_da_risposta(aperto).unwrap();
        let errore = risposta(Union::Data(TerminalData {
            terminal_id: crate::remotek::exec::TERMINAL_ID_STDERR,
            data: b"e\n".to_vec().into(),
            ..Default::default()
        }));
        assert_eq!(
            evento_da_risposta(errore),
            Some(Evento::Stderr(b"e\n".to_vec()))
        );
        let (esito, inviati, out, err) = ciclo_exec(vec![
            Evento::Connesso(true),
            evento,
            Evento::Dati(b"o\n".to_vec()),
            Evento::Stderr(vec![0xff, b'\n']),
            Evento::Chiuso(300),
        ]);
        assert_eq!(esito, Ok(255));
        assert_eq!((out, err), (b"o\n".to_vec(), vec![0xff, b'\n']));
        let mandati = dati_mandati(&inviati);
        // Lo script in due blocchi, poi il blocco vuoto: la fine.
        assert_eq!(
            mandati.iter().map(Vec::len).collect::<Vec<_>>(),
            vec![BLOCCO_SCRIPT, 1, 0]
        );
        let (esito, _, _, _) = ciclo_exec(vec![Evento::Connesso(true), Evento::NonPartito]);
        assert_eq!(esito, Ok(super::super::USCITA_NON_PARTITO));
    }

    #[test]
    fn chiuso_prima_dello_script_e_un_errore() {
        let prima = Err("il PC ha chiuso il terminale prima dello script".to_owned());
        let (esito, inviati, _, _) = ciclo_exec(vec![Evento::Chiuso(7)]);
        assert_eq!(esito, prima);
        assert!(dati_mandati(&inviati).is_empty(), "nessuno script mandato");
        let (esito, inviati, _, _) = ciclo_exec(vec![Evento::Connesso(true), Evento::Chiuso(0)]);
        assert_eq!(esito, prima);
        assert!(dati_mandati(&inviati).is_empty(), "nessuno script mandato");
        // Il CLI ne esce con 1 (`USCITA_ERRORE`), come per ogni `Err`.
    }

    /// Nessun `OpenTerminal` fra i messaggi mandati al PC.
    fn nessuna_apertura(inviati: &[Data]) -> bool {
        !inviati.iter().any(|d| match d {
            Data::Message(m) => matches!(
                &m.terminal_action().union,
                Some(terminal_action::Union::Open(_))
            ),
            _ => false,
        })
    }

    #[test]
    fn terminal_non_apre_sul_pc_che_non_dichiara() {
        let (esito, inviati) = ciclo(vec![Evento::Connesso(false), Evento::Chiuso(0)], 5);
        assert_eq!(esito, Ok(super::super::USCITA_NON_DICHIARA));
        assert!(inviati.is_empty(), "niente al PC");
        let mut avvisi = Vec::new();
        assert_eq!(rifiuta_il_pc_che_non_dichiara(&mut avvisi), 126);
        assert_eq!(
            String::from_utf8(avvisi).unwrap(),
            "remotek-cli: il PC non dichiara la trascrizione e la vista del terminale: aggiornare Remotek sul PC (remotek-1.4.9-7 o seguente)\n"
        );
        // Il PC che dichiara: open_terminal come prima.
        let (esito, inviati) = ciclo(vec![Evento::Connesso(true), Evento::Chiuso(0)], 5);
        assert_eq!(esito, Ok(0));
        assert!(!nessuna_apertura(&inviati));
        // 126 non e' ne' 1, ne' 2, ne' 125.
        assert_ne!(
            super::super::USCITA_NON_DICHIARA,
            super::super::USCITA_NON_PARTITO
        );
        assert_ne!(
            super::super::USCITA_NON_DICHIARA,
            super::super::USCITA_ERRORE
        );
        assert_ne!(super::super::USCITA_NON_DICHIARA, super::super::USCITA_USO);
    }

    #[test]
    fn exec_non_apre_sul_pc_che_non_dichiara() {
        let aperto = Evento::Aperto(crate::remotek::exec::APERTO.into());
        let (esito, inviati, out, err) =
            ciclo_exec(vec![Evento::Connesso(false), aperto, Evento::Chiuso(0)]);
        assert_eq!(esito, Ok(super::super::USCITA_NON_DICHIARA));
        assert!(inviati.is_empty(), "niente al PC, nemmeno lo script");
        assert!(out.is_empty());
        assert_eq!(
            String::from_utf8(err).unwrap(),
            format!("remotek-cli: {NON_DICHIARA}\n")
        );
        let aperto = Evento::Aperto(crate::remotek::exec::APERTO.into());
        let (esito, inviati, _, err) =
            ciclo_exec(vec![Evento::Connesso(true), aperto, Evento::Chiuso(0)]);
        assert_eq!(esito, Ok(0));
        assert!(!nessuna_apertura(&inviati));
        assert!(!dati_mandati(&inviati).is_empty(), "lo script e' partito");
        assert!(err.is_empty());
    }

    #[test]
    fn set_peer_info_decide_connesso() {
        let connesso = |platform_additions: &str| {
            let (tx, rx) = channel();
            let gestore = Gestore {
                eventi: Some(tx),
                ..Default::default()
            };
            gestore.set_peer_info(&PeerInfo {
                platform_additions: platform_additions.into(),
                ..Default::default()
            });
            gestore.on_connected(ConnType::TERMINAL);
            rx.try_recv().unwrap()
        };
        let mut dichiara = serde_json::Map::new();
        dichiara.insert("is_installed".into(), serde_json::json!(true));
        crate::remotek::dichiarazione::aggiungi(&mut dichiara);
        let dichiara = serde_json::to_string(&dichiara).unwrap();
        assert_eq!(connesso(&dichiara), Evento::Connesso(true));
        assert_eq!(
            connesso(r#"{"is_installed":true,"support_view_camera":true}"#),
            Evento::Connesso(false)
        );
        assert_eq!(connesso(""), Evento::Connesso(false));
    }

    /// Con exec il PC del -7 e del -8 (le chiavi del terminale, non quella
    /// dello script nella scelta): `ScriptNonNellaScelta`, 126, niente al PC.
    /// Il terminale non la guarda e apre come prima.
    #[test]
    fn exec_non_apre_sul_pc_senza_script_nella_scelta() {
        let vecchio = r#"{"remotek_trascrizione":true,"remotek_vista_cm":true}"#;
        let nuovo = {
            let mut m = serde_json::Map::new();
            crate::remotek::dichiarazione::aggiungi(&mut m);
            serde_json::to_string(&m).unwrap()
        };
        let connesso = |exec: bool, platform_additions: &str| {
            let (tx, rx) = channel();
            let gestore = Gestore {
                eventi: Some(tx),
                exec,
                ..Default::default()
            };
            gestore.set_peer_info(&PeerInfo {
                platform_additions: platform_additions.into(),
                ..Default::default()
            });
            gestore.on_connected(ConnType::TERMINAL);
            rx.try_recv().unwrap()
        };
        assert_eq!(connesso(true, vecchio), Evento::ScriptNonNellaScelta);
        assert_eq!(connesso(true, &nuovo), Evento::Connesso(true));
        assert_eq!(connesso(true, ""), Evento::Connesso(false));
        assert_eq!(connesso(false, vecchio), Evento::Connesso(true));
        let aperto = Evento::Aperto(crate::remotek::exec::APERTO.into());
        let (esito, inviati, out, err) = ciclo_exec(vec![
            Evento::ScriptNonNellaScelta,
            aperto,
            Evento::Chiuso(0),
        ]);
        assert_eq!(esito, Ok(super::super::USCITA_NON_DICHIARA));
        assert!(
            inviati.is_empty(),
            "niente al PC: ne' OpenTerminal ne' lo script"
        );
        assert!(out.is_empty());
        assert_eq!(
            String::from_utf8(err).unwrap(),
            format!("remotek-cli: {SCRIPT_NON_NELLA_SCELTA}\n")
        );
        // Il terminale sul PC del -8 apre come prima.
        let (esito, inviati) = ciclo(vec![connesso(false, vecchio), Evento::Chiuso(0)], 5);
        assert_eq!(esito, Ok(0));
        assert!(!nessuna_apertura(&inviati));
    }
}
