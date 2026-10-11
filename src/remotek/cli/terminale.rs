//! `terminal <ID>`: il terminale di un PC, come la sessione terminale del
//! client Flutter (src/flutter.rs, `session_add` e `session_start_`), ma senza
//! interfaccia. Si entra solo se il cliente accetta la finestra (ADR-0021,
//! regola 2): nessuna password del PC; mai come amministratore (regola 3).

use crate::client::{Data, Interface, QualityStatus};
use crate::ui_session_interface::{io_loop, InvokeUiSession, Session};
use hbb_common::{
    config::{Ab, LocalConfig, PeerConfig},
    message_proto::{terminal_response::Union, *},
    rendezvous_proto::ConnType,
};
use std::io::{Read, Write};
use std::sync::{
    atomic::AtomicUsize,
    mpsc::{channel, RecvTimeoutError, Sender},
    Arc, RwLock,
};
use std::time::{Duration, Instant};

/// Il terminale che il CLI apre sul PC (uno per sessione).
const ID_TERMINALE: i32 = 0;
/// Dopo la fine di stdin, quanto si aspetta il `TerminalClosed` del PC.
const ATTESA_CHIUSURA: Duration = Duration::from_secs(5);

#[derive(Debug, PartialEq)]
pub(super) enum Evento {
    /// Il login e' partito senza password: si aspetta il clic del cliente.
    Attesa,
    /// Il controllato offre una connessione non cifrata da un capo all'altro.
    NonCifrata,
    Connesso,
    Aperto,
    Dati(Vec<u8>),
    Chiuso(i32),
    Errore(String),
    FineInput,
}

/// L'handler della sessione: trasforma i richiami di `io_loop` in eventi per
/// il ciclo del CLI. I metodi che servono solo a schermo, file e chiamate
/// vocali restano vuoti.
#[derive(Clone, Default)]
pub(super) struct Gestore {
    eventi: Option<Sender<Evento>>,
}

impl Gestore {
    fn invia(&self, evento: Evento) {
        if let Some(tx) = &self.eventi {
            // Il ricevitore sparisce solo quando il CLI sta gia' uscendo.
            tx.send(evento).ok();
        }
    }
}

/// Il significato di un messaggio per il CLI (src/client.rs, `handle_hash` e
/// `handle_login_error`). `None`: niente da fare.
pub(super) fn evento_da_msgbox(tipo: &str, titolo: &str, testo: &str) -> Option<Evento> {
    match tipo {
        "input-password" | "wait-remote-accept-nook" => Some(Evento::Attesa),
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
        Union::Opened(o) if o.success => Some(Evento::Aperto),
        Union::Opened(o) => Some(Evento::Errore(format!(
            "terminale non aperto: {}",
            o.message
        ))),
        Union::Data(d) if d.compressed => {
            Some(Evento::Dati(hbb_common::compress::decompress(&d.data)))
        }
        Union::Data(d) => Some(Evento::Dati(d.data.to_vec())),
        Union::Closed(c) => Some(Evento::Chiuso(c.exit_code)),
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
    fn set_peer_info(&self, _: &PeerInfo) {}
    fn set_displays(&self, _: &Vec<DisplayInfo>) {}
    fn set_platform_additions(&self, _: &str) {}
    fn on_connected(&self, conn_type: ConnType) {
        if conn_type == ConnType::TERMINAL {
            self.invia(Evento::Connesso);
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
        if let Some(evento) = evento_da_msgbox(tipo, titolo, testo) {
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
    fn set_multiple_windows_session(&self, _: Vec<WindowsSession>) {}
    fn set_current_display(&self, _: i32) {}
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
/// nessuna password condivisa), tipo TERMINAL, mai amministratore.
fn sessione(id: &str, gestore: Gestore) -> Result<Session<Gestore>, String> {
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
    lc.initialize(
        id.to_owned(),
        ConnType::TERMINAL,
        None,
        false,
        None,
        None,
        None,
    );
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

/// Apre il terminale del PC `id` e restituisce il codice d'uscita.
pub(super) fn terminale(id: &str, attesa: u64, righe: u32, colonne: u32) -> Result<i32, String> {
    let (tx, rx) = channel();
    let session = sessione(
        id,
        Gestore {
            eventi: Some(tx.clone()),
        },
    )?;
    // L'ID come lo usa il client (initialize toglie `/r` e simili).
    let id = session
        .lc
        .read()
        .map_err(|_| "sessione non disponibile".to_owned())?
        .get_id()
        .to_owned();
    let ciclo = session.clone();
    let fine = FineSessione(tx.clone());
    std::thread::spawn(move || {
        // Anche dopo un panic o un'uscita senza msgbox: `attendi` e il
        // `Gestore` tengono un `tx`, quindi il canale non si chiude da solo.
        let _fine = fine;
        let round = match ciclo.connection_round_state.lock() {
            Ok(mut stato) => stato.new_round(),
            Err(_) => return,
        };
        io_loop(ciclo, round);
    });
    let esito = attendi(
        &session,
        &rx,
        tx,
        Duration::from_secs(attesa),
        righe,
        colonne,
    );
    session.close();
    // handle_peer_info salva la configurazione del PC (src/client.rs): il CLI
    // non ne tiene.
    PeerConfig::remove(&id);
    esito
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
            Evento::Connesso => session.open_terminal(ID_TERMINALE, righe, colonne),
            Evento::Aperto if scadenza.is_some() && !chiusura => {
                scadenza = None;
                leggi_stdin(session.clone(), tx.clone());
            }
            Evento::Aperto => {}
            Evento::Dati(dati) => {
                stdout
                    .write_all(&dati)
                    .and_then(|()| stdout.flush())
                    .map_err(|e| format!("stdout: {e}"))?;
            }
            Evento::Chiuso(codice) => return Ok(codice_uscita(codice)),
            Evento::Errore(e) => return Err(e),
            Evento::FineInput => {
                session.close_terminal(ID_TERMINALE);
                chiusura = true;
                scadenza = Some(Instant::now() + ATTESA_CHIUSURA);
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use hbb_common::tokio::sync::mpsc::unbounded_channel;
    use std::sync::Mutex;

    /// I test che toccano IS_TERMINAL_ADMIN o costruiscono una sessione non
    /// girano in parallelo: `initialize` legge l'ambiente.
    static AMBIENTE: Mutex<()> = Mutex::new(());

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
        let (esito, inviati) = ciclo(vec![Evento::Connesso, Evento::Chiuso(3)], 5);
        assert_eq!(esito, Ok(3));
        // Connesso: `open_terminal` con righe e colonne.
        assert!(matches!(&inviati[..], [Data::Message(m)] if m.has_terminal_action()));
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
        let rifiuto = evento_da_msgbox("error", "Connection Error", "Connection declined");
        assert!(matches!(rifiuto, Some(Evento::Errore(_))));
        for tipo in [
            "re-input-password",
            "input-2fa",
            "terminal-admin-login",
            "terminal-admin-login-password",
        ] {
            assert!(
                matches!(evento_da_msgbox(tipo, "", ""), Some(Evento::Errore(_))),
                "{tipo}"
            );
        }
        assert!(ciclo(vec![Evento::Errore("x".into())], 5).0.is_err());
        // Nessuna risposta del cliente entro --attesa.
        let attesa = evento_da_msgbox("wait-remote-accept-nook", "", "");
        assert_eq!(attesa, Some(Evento::Attesa));
        assert!(ciclo(vec![Evento::Attesa], 0).0.is_err());
    }

    #[test]
    fn fine_del_ciclo_senza_msgbox_e_un_errore() {
        for panic in [false, true] {
            let (tx, rx) = channel();
            let fine = FineSessione(tx.clone());
            let ciclo = std::thread::spawn(move || {
                let _fine = fine;
                assert!(!panic, "io_loop in panic");
            });
            assert_eq!(ciclo.join().is_err(), panic);
            let session = Session::<Gestore>::default();
            let esito = attendi(&session, &rx, tx, Duration::from_secs(5), 24, 80);
            assert_eq!(esito, Err("la sessione e' finita".to_owned()));
        }
    }

    #[test]
    fn connessione_non_cifrata_rifiutata() {
        let evento = evento_da_msgbox("insecure-connection-nocancel-hasclose", "", "");
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
        let session = sessione("remotek-cli-test-id", Gestore::default()).unwrap();
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
        let esito = sessione(&format!("{id}/r"), Gestore::default());
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
        let session = sessione(id, Gestore::default()).unwrap();
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
    #[test]
    fn login_senza_password_e_senza_os_login() {
        let _ambiente = AMBIENTE.lock().unwrap();
        let session = sessione("remotek-cli-test-login", Gestore::default()).unwrap();
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
        let lr = login.login_request();
        assert!(lr.password.is_empty());
        let os = lr.os_login.as_ref();
        assert!(os.map_or(true, |o| o.username.is_empty() && o.password.is_empty()));
        assert!(lr.has_terminal());
    }
}
