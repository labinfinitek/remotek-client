//! Exec (ADR-0033 passo 2): il terminale senza pseudo-console. L'agente manda
//! uno script intero e riceve stdout, stderr e codice d'uscita separati.
//!
//! La sessione e' di exec quando il lasciapassare della connessione e' di exec
//! (`lasciapassare::exec`); consenso, lasciapassare, trascrizione e identita'
//! sono quelli del terminale: questo modulo non sceglie l'utente. Come la
//! shell del terminale, lo script gira nel processo del servizio, che e' gia'
//! l'utente deciso da `fill_terminal_user_token` (src/server/connection.rs),
//! oppure, col token dell'utente della console (Windows installato),
//! nell'helper di upstream lanciato con quel token (modulo `helper`).
//!
//! Il contratto col CLI:
//! - all'`OpenTerminal` il PC risponde `TerminalOpened` con `success` e
//!   `message` = [`APERTO`], senza aprire una pseudo-console;
//! - lo script arriva nei `TerminalData` del `terminal_id` aperto, fino a un
//!   `TerminalData` vuoto, che e' la fine dello script. Il client Flutter non
//!   manda mai dati vuoti (`send_terminal_input` di src/ui_session_interface.rs
//!   porta l'input di xterm, mai vuoto), e una sessione di exec esiste solo
//!   col lasciapassare di exec, che chiede solo `remotek-cli`;
//! - oltre [`MASSIMO_SCRIPT`] byte o `script_exec::MASSIMO_RIGHE` righe:
//!   `TerminalError` con [`TROPPO_GRANDE`] (da [`Esecuzione::leggi`]),
//!   la sessione si chiude e non si esegue niente;
//! - lo script raccolto deve essere uguale byte per byte a quello del login,
//!   mostrato al cliente (src/remotek/script_al_login.rs, ADR-0035); se no,
//!   o se il login non ne portava uno, `TerminalError` con [`NON_MOSTRATO`]
//!   e non si esegue niente;
//! - stdout va nei `TerminalData` del `terminal_id` aperto, stderr in quelli
//!   di [`TERMINAL_ID_STDERR`]; l'ordine fra i due flussi non e' garantito;
//! - alla fine `TerminalClosed` col codice dello script, oppure un
//!   `TerminalError` con `uscita_terminale::NON_PARTITO` se non e' partito.
//!
//! Il processo (Windows: `powershell.exe` di System32, col percorso intero;
//! altrove: `/bin/sh -s`)
//! e tutti i suoi figli si chiudono insieme: su Windows con un Job object con
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, altrove col gruppo di processi.
//! Chiusura del terminale, della connessione o dal connection manager,
//! oppure lo script finito: muoiono anche i figli rimasti.
//!
//! Come il terminale non persistente, exec chiude dopo un'ora senza
//! `TerminalAction` (`cleanup_inactive_services` di upstream: l'output non
//! conta come attivita'), anche se lo script sta ancora girando.
//!
//! Nella trascrizione il file locale e la vista del cliente distinguono
//! stdout e stderr dal `terminal_id`; verso l'API, che oggi non riceve il
//! `terminal_id`, sono tutti e due `out`, nell'ordine di arrivo.

pub use super::script_exec::{aggiungi, Blocco, MASSIMO_SCRIPT, NON_MOSTRATO, TROPPO_GRANDE};
use super::script_exec::{Passo, Raccolta};
use super::uscita_terminale::{
    codice_dall_helper, codice_dello_script, risposta_chiusura, HELPER_NON_PARTITO,
};
use crate::server::terminal_service::UserToken;
use hbb_common::message_proto::{
    terminal_action, TerminalAction, TerminalData, TerminalError, TerminalOpened, TerminalResponse,
};
use hbb_common::{anyhow, log};
use std::{
    io::{self, Read, Write},
    process::{Child, ChildStdin, Command, ExitStatus, Stdio},
    sync::mpsc::{sync_channel, Receiver, SyncSender, TryRecvError},
    thread,
    time::{Duration, Instant},
};

/// Il `message` del `TerminalOpened` di una sessione di exec.
pub const APERTO: &str = "remotek-exec";
/// Il `terminal_id` di stderr. Il client Flutter numera i terminali da 1 in
/// su (`_nextTerminalId` di flutter/lib/desktop/pages/terminal_tab_page.dart).
pub const TERMINAL_ID_STDERR: i32 = -2;
/// Finito lo script, quanto si aspetta ancora l'output gia' scritto.
const ATTESA_OUTPUT: Duration = Duration::from_secs(2);

/// Il comando fisso di PowerShell: UTF-8 in ingresso e in uscita, poi tutto
/// lo stdin come un solo scriptblock (`exit N` da' il codice N), senza file
/// .ps1 e quindi senza ExecutionPolicy.
#[cfg(windows)]
const COMANDO_POWERSHELL: &str = "$u = New-Object System.Text.UTF8Encoding $false; \
[Console]::InputEncoding = $u; [Console]::OutputEncoding = $u; $OutputEncoding = $u; \
& ([scriptblock]::Create([Console]::In.ReadToEnd()))";

/// Il codice d'uscita; ucciso da un segnale: 128 + il segnale, come le shell.
/// Passa da `codice_dello_script`: lo script e' partito, e il suo codice non
/// deve sembrare `HELPER_NON_PARTITO`.
fn codice(stato: ExitStatus) -> i32 {
    #[cfg(unix)]
    let segnale = std::os::unix::process::ExitStatusExt::signal(&stato);
    #[cfg(not(unix))]
    let segnale = None;
    codice_da(stato.code(), segnale)
}

/// [`codice`] senza il processo, per provarlo su Linux, dove un processo non
/// esce con `HELPER_NON_PARTITO`: 128 + il segnale se c'e', altrimenti il
/// codice (`None`: -1), passati da `codice_dello_script`.
fn codice_da(codice: Option<i32>, segnale: Option<i32>) -> i32 {
    match segnale {
        Some(s) => codice_dello_script(128 + s),
        None => codice_dello_script(codice.unwrap_or(-1)),
    }
}

#[cfg(windows)]
fn comando() -> io::Result<Command> {
    use std::os::windows::process::CommandExt;
    // SystemRoot dal registro di sistema (la sorgente della variabile
    // d'ambiente, che chi avvia il processo puo' cambiare; sez. 19 di
    // verifica-patch.sh: il codice di src/remotek/ non legge l'ambiente).
    let chiave = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")?;
    let radice = std::path::PathBuf::from(chiave.get_value::<String, _>("SystemRoot")?);
    if !radice.is_absolute() {
        return Err(io::Error::other("SystemRoot non e' un percorso intero"));
    }
    let mut c = Command::new(radice.join(r"System32\WindowsPowerShell\v1.0\powershell.exe"));
    c.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        COMANDO_POWERSHELL,
    ]);
    c.creation_flags(windows::Win32::System::Threading::CREATE_NO_WINDOW.0);
    Ok(c)
}

#[cfg(not(windows))]
fn comando() -> io::Result<Command> {
    use std::os::unix::process::CommandExt;
    let mut c = Command::new("/bin/sh");
    c.arg("-s").process_group(0);
    Ok(c)
}

/// Il processo dello script e i suoi figli: chiudere il Job (Windows) o
/// uccidere il gruppo (altrove) li uccide tutti. Si fa una volta sola, prima
/// di raccogliere il processo: dopo, il numero del gruppo potrebbe essere
/// riusato.
struct Albero {
    #[cfg(windows)]
    job: Option<usize>,
    #[cfg(not(windows))]
    gruppo: Option<i32>,
}

impl Albero {
    /// Windows: si chiama subito dopo lo spawn, prima di scrivere un solo
    /// byte dello script. PowerShell non fa niente finche' lo script non
    /// arriva dallo stdin, quindi prima dell'assegnazione al Job non puo'
    /// avere figli: non serve partire sospesi.
    #[cfg(windows)]
    fn nuovo(child: &Child) -> io::Result<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::{Foundation::HANDLE, System::JobObjects::*};
        let job = unsafe { CreateJobObjectW(None, windows::core::PCWSTR::null()) }
            .map_err(io::Error::other)?;
        // Da qui il Drop chiude il Job, anche se i passi sotto falliscono.
        let albero = Self {
            job: Some(job.0 as usize),
        };
        let mut limiti = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limiti.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &limiti as *const _ as *const std::ffi::c_void,
                std::mem::size_of_val(&limiti) as u32,
            )
            .map_err(io::Error::other)?;
            AssignProcessToJobObject(job, HANDLE(child.as_raw_handle()))
                .map_err(io::Error::other)?;
        }
        Ok(albero)
    }

    #[cfg(not(windows))]
    fn nuovo(child: &Child) -> io::Result<Self> {
        // `process_group(0)` in `comando`: il gruppo ha il numero del processo.
        Ok(Self {
            gruppo: Some(child.id() as i32),
        })
    }

    #[cfg(windows)]
    fn uccidi(&mut self) {
        if let Some(job) = self.job.take() {
            let job = windows::Win32::Foundation::HANDLE(job as *mut std::ffi::c_void);
            if let Err(e) = unsafe { windows::Win32::Foundation::CloseHandle(job) } {
                log::warn!("exec: chiusura del Job object non riuscita: {}", e);
            }
        }
    }

    #[cfg(not(windows))]
    fn uccidi(&mut self) {
        use hbb_common::libc;
        if let Some(g) = self.gruppo.take() {
            if unsafe { libc::killpg(g, libc::SIGKILL) } != 0 {
                let e = io::Error::last_os_error();
                if e.raw_os_error() != Some(libc::ESRCH) {
                    log::warn!("exec: kill del gruppo {} non riuscito: {}", g, e);
                }
            }
        }
    }
}

impl Drop for Albero {
    fn drop(&mut self) {
        self.uccidi();
    }
}

#[derive(Debug, PartialEq)]
enum Uscita {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    /// L'helper ha avviato lo script e non gli ha ancora dato un byte dello
    /// script ([`MSG_TYPE_PARTITO`]). La consuma [`Processo::passo`].
    Partito,
}

fn lettore<R: Read + Send + 'static>(mut r: R, tx: SyncSender<Uscita>, stderr: bool) {
    thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match r.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let dati = buf[..n].to_vec();
                    let u = if stderr {
                        Uscita::Stderr(dati)
                    } else {
                        Uscita::Stdout(dati)
                    };
                    if tx.send(u).is_err() {
                        break;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => {
                    log::warn!("exec: lettura dell'output non riuscita: {}", e);
                    break;
                }
            }
        }
    });
}

/// Il processo dello script, in corso.
struct Processo {
    figlio: Figlio,
    uscita: Receiver<Uscita>,
    /// Il codice del processo, e da quando, una volta finito lo script.
    finito: Option<(i32, Instant)>,
    /// L'helper ha detto che lo script e' partito ([`Uscita::Partito`]).
    /// Nella strada diretta resta `false`.
    partito: bool,
}

impl Processo {
    /// Avvio, Job, script: la strada diretta.
    fn avvia(script: Vec<u8>) -> io::Result<Self> {
        let (processo, stdin) = Self::avvia_senza_script()?;
        scrivi_script(stdin, script);
        Ok(processo)
    }

    /// Avvia il processo e lo assegna al Job, senza scrivere lo script: chi
    /// chiama riceve lo stdin e lo da' a [`scrivi_script`]. Finche' lo
    /// script non e' nello stdin il processo non esegue niente; se lo stdin
    /// si chiude senza script, esegue uno script vuoto.
    fn avvia_senza_script() -> io::Result<(Self, ChildStdin)> {
        let mut child = comando()?
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let albero = match Albero::nuovo(&child) {
            Ok(a) => a,
            Err(e) => {
                // Nessuno script parte fuori dal Job.
                if let Err(k) = child.kill() {
                    log::warn!("exec: kill del processo non riuscito: {}", k);
                }
                if let Err(w) = child.wait() {
                    log::warn!("exec: attesa del processo non riuscita: {}", w);
                }
                return Err(e);
            }
        };
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            return Err(io::Error::other("pipe del processo mancanti"));
        };
        let (tx, uscita) = sync_channel(64);
        lettore(stdout, tx.clone(), false);
        lettore(stderr, tx, true);
        let processo = Self {
            figlio: Figlio::Diretto { child, albero },
            uscita,
            finito: None,
            partito: false,
        };
        Ok((processo, stdin))
    }

    /// Un giro: l'output pronto e, a script finito e output letto (o dopo
    /// [`ATTESA_OUTPUT`]), il codice.
    fn passo(&mut self) -> (Vec<Uscita>, Option<i32>) {
        let mut uscite = Vec::new();
        let mut lettori_finiti = false;
        loop {
            match self.uscita.try_recv() {
                Ok(Uscita::Partito) => self.partito = true,
                Ok(u) => uscite.push(u),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    lettori_finiti = true;
                    break;
                }
            }
        }
        if self.finito.is_none() && self.figlio.uscito() {
            self.finito = Some((self.figlio.chiudi(), Instant::now()));
        }
        // Qui "partito" e' gia' letto: e' il primo messaggio dell'helper,
        // scritto prima che lo script giri.
        let partito = self.partito;
        let fine = self
            .finito
            .filter(|(_, da)| lettori_finiti || da.elapsed() > ATTESA_OUTPUT)
            .map(|(codice, _)| codice_dall_helper(codice, partito));
        (uscite, fine)
    }
}

/// Scrive lo script nello stdin del processo, in un thread: uno script che
/// non legge lo stdin non blocca il servizio. Lo stdin si chiude a fine
/// thread.
fn scrivi_script(mut stdin: ChildStdin, script: Vec<u8>) {
    thread::spawn(move || {
        if let Err(e) = stdin.write_all(&script) {
            log::warn!("exec: scrittura dello script non riuscita: {}", e);
        }
    });
}

/// Lo script nel processo del servizio, o nell'helper del terminale.
enum Figlio {
    Diretto {
        child: Child,
        albero: Albero,
    },
    #[cfg(windows)]
    Helper(helper::Helper),
}

impl Figlio {
    /// Lo script e' finito, senza raccogliere il processo (vedi [`Albero`]).
    #[cfg(not(windows))]
    fn uscito(&mut self) -> bool {
        use hbb_common::libc;
        let Figlio::Diretto { child, .. } = self;
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let flag = libc::WEXITED | libc::WNOHANG | libc::WNOWAIT;
        if unsafe { libc::waitid(libc::P_PID, child.id(), &mut info, flag) } != 0 {
            log::warn!("exec: waitid non riuscita: {}", io::Error::last_os_error());
            return false;
        }
        unsafe { info.si_pid() != 0 }
    }

    #[cfg(windows)]
    fn uscito(&mut self) -> bool {
        match self {
            // Su Windows l'handle del processo resta aperto: niente riuso.
            Figlio::Diretto { child, .. } => match child.try_wait() {
                Ok(s) => s.is_some(),
                Err(e) => {
                    log::warn!("exec: try_wait non riuscita: {}", e);
                    false
                }
            },
            Figlio::Helper(h) => h.uscito(),
        }
    }

    /// Uccide i figli rimasti e raccoglie il processo.
    fn chiudi(&mut self) -> i32 {
        match self {
            Figlio::Diretto { child, albero } => {
                albero.uccidi();
                match child.wait() {
                    Ok(s) => codice(s),
                    Err(e) => {
                        log::warn!("exec: attesa del processo non riuscita: {}", e);
                        -1
                    }
                }
            }
            #[cfg(windows)]
            Figlio::Helper(h) => h.chiudi(),
        }
    }
}

impl Drop for Processo {
    fn drop(&mut self) {
        if self.finito.is_none() {
            self.figlio.chiudi();
        }
    }
}

/// Una sessione di exec nel servizio del terminale: un solo script. Quali
/// azioni fanno parte dello script lo decide `script_exec::Raccolta`, la
/// stessa regola della vista del cliente.
pub struct Esecuzione {
    /// Lo script del login, mostrato al cliente: l'unico che parte.
    mostrato: Option<Vec<u8>>,
    raccolta: Raccolta,
    processo: Option<Processo>,
    chiusa: bool,
    /// La fine di uno script rifiutato o non partito, per [`Esecuzione::leggi`].
    fine: Option<TerminalResponse>,
}

fn errore(terminal_id: i32, message: &str) -> TerminalResponse {
    let mut r = TerminalResponse::new();
    r.set_error(TerminalError {
        terminal_id,
        message: message.to_owned(),
        ..Default::default()
    });
    r
}

fn dati(terminal_id: i32, data: Vec<u8>) -> TerminalResponse {
    let mut r = TerminalResponse::new();
    r.set_data(TerminalData {
        terminal_id,
        data: data.into(),
        ..Default::default()
    });
    r
}

impl Esecuzione {
    /// `mostrato`: lo script del login, quello che il cliente ha visto.
    pub fn nuova(mostrato: Option<Vec<u8>>) -> Self {
        Self {
            mostrato,
            raccolta: Raccolta::default(),
            processo: None,
            chiusa: false,
            fine: None,
        }
    }

    /// `token`: quello del servizio del terminale. Se c'e' (Windows
    /// installato, l'helper gira come l'utente della console) lo script va
    /// all'helper di upstream, altrimenti gira nel processo del servizio.
    pub fn azione(
        &mut self,
        azione: &TerminalAction,
        token: Option<UserToken>,
    ) -> Option<TerminalResponse> {
        match self.raccolta.azione(azione) {
            Passo::Aperto(terminal_id) => {
                let mut r = TerminalResponse::new();
                r.set_opened(TerminalOpened {
                    terminal_id,
                    success: true,
                    message: APERTO.to_owned(),
                    ..Default::default()
                });
                Some(r)
            }
            Passo::AltroOpen(terminal_id) => {
                Some(errore(terminal_id, "exec: un solo script per connessione"))
            }
            Passo::Pezzo => None,
            // Rifiutato o non partito: la risposta esce da `leggi`, come la
            // fine di uno script partito, perche' solo le risposte di
            // `read_outputs` passano da `trascrizione::uscita`
            // (src/server/connection.rs), che ne da' la riga alla vista del
            // cliente; quelle di `azione` vanno dritte al controllante.
            Passo::TroppoGrande(terminal_id) => {
                self.chiusa = true;
                self.fine = Some(errore(terminal_id, TROPPO_GRANDE));
                None
            }
            // Solo lo script mostrato al cliente, uguale byte per byte.
            Passo::Completo(terminal_id, script) if self.mostrato.as_ref() != Some(&script) => {
                log::warn!("exec: lo script non e' quello mostrato al cliente: non eseguito");
                self.chiusa = true;
                self.fine = Some(errore(terminal_id, NON_MOSTRATO));
                None
            }
            Passo::Completo(terminal_id, script) => match avvia(script, token, terminal_id) {
                Ok(p) => {
                    self.processo = Some(p);
                    None
                }
                Err(e) => {
                    log::warn!("exec: lo script non e' partito: {}", e);
                    self.chiusa = true;
                    self.fine = Some(risposta_chiusura(terminal_id, HELPER_NON_PARTITO));
                    None
                }
            },
            Passo::Chiuso(terminal_id) => {
                if self.chiusa {
                    return None;
                }
                self.chiusa = true;
                // -1: chiusura forzata, come il terminale (`handle_close`).
                // Il Drop del processo uccide lo script e i figli.
                self.processo = None;
                Some(risposta_chiusura(terminal_id, -1))
            }
            Passo::Scartata => {
                if let Some(terminal_action::Union::Data(d)) = &azione.union {
                    log::warn!("exec: dati ignorati per il terminale {}", d.terminal_id);
                }
                None
            }
        }
    }

    /// Le risposte pronte: l'output, e alla fine la chiusura col codice;
    /// oppure, una volta, il rifiuto o la mancata partenza dello script.
    pub fn leggi(&mut self) -> Vec<TerminalResponse> {
        let mut risposte: Vec<_> = self.fine.take().into_iter().collect();
        let (Some(id), Some(p)) = (self.raccolta.terminal_id(), self.processo.as_mut()) else {
            return risposte;
        };
        let (uscite, fine) = p.passo();
        risposte.extend(uscite.into_iter().filter_map(|u| match u {
            Uscita::Stdout(d) => Some(dati(id, d)),
            Uscita::Stderr(d) => Some(dati(TERMINAL_ID_STDERR, d)),
            // La consuma `passo`.
            Uscita::Partito => None,
        }));
        if let Some(codice) = fine {
            self.processo = None;
            self.chiusa = true;
            risposte.push(risposta_chiusura(id, codice));
        }
        risposte
    }
}

#[cfg(windows)]
fn avvia(script: Vec<u8>, token: Option<UserToken>, terminal_id: i32) -> anyhow::Result<Processo> {
    match token {
        Some(t) => helper::avvia(t, script, terminal_id),
        None => Ok(Processo::avvia(script)?),
    }
}

#[cfg(not(windows))]
fn avvia(
    script: Vec<u8>,
    _token: Option<UserToken>,
    _terminal_id: i32,
) -> anyhow::Result<Processo> {
    Ok(Processo::avvia(script)?)
}

/// L'argomento in piu' di `--terminal-helper` per il modo exec
/// (`launch_terminal_helper_with_token`, src/server/terminal_helper.rs).
pub const ARG_HELPER: &str = "exec";
/// I messaggi della pipe dell'helper sono quelli di upstream
/// (`encode_helper_message`: tipo u8, lunghezza u32 LE, dati). In exec lo
/// stdout e lo script viaggiano come `MSG_TYPE_DATA`; questi due sono nuovi.
/// La fine dello script e' un messaggio, non la chiusura della pipe: una
/// pipe chiusa a meta' (servizio caduto) non deve far partire uno script
/// troncato.
pub const MSG_TYPE_FINE_SCRIPT: u8 = 0x10;
pub const MSG_TYPE_STDERR: u8 = 0x11;
/// "Lo script e' partito", senza dati: il primo messaggio della pipe
/// d'uscita dell'helper, scritto dopo l'assegnazione al Job e prima di un
/// solo byte dello script nello stdin. Lo script non lo puo' scrivere prima
/// ne' cancellare dopo. Col "partito" il servizio passa il codice
/// dell'helper da `codice_dello_script` (`codice_dall_helper`); senza,
/// l'helper e' uscito prima dello script.
pub const MSG_TYPE_PARTITO: u8 = 0x12;
/// `MSG_TYPE_DATA` di src/server/terminal_helper.rs (solo Windows).
const MSG_TYPE_DATA: u8 = 0x01;
#[cfg(windows)]
const _: () = assert!(MSG_TYPE_DATA == crate::server::terminal_helper::MSG_TYPE_DATA);

#[cfg(windows)]
use crate::server::terminal_helper::encode_helper_message as codifica;

/// Su Windows e' `encode_helper_message`; qui la stessa intestazione per i test.
#[cfg(not(windows))]
fn codifica(tipo: u8, dati: &[u8]) -> Vec<u8> {
    let mut m = vec![tipo];
    m.extend_from_slice(&(dati.len() as u32).to_le_bytes());
    m.extend_from_slice(dati);
    m
}

/// Decodifica i messaggi della pipe dell'helper, anche spezzati fra piu'
/// letture. Un tipo fuori da `tipi` o una lunghezza oltre [`MASSIMO_SCRIPT`]
/// sono un errore: la pipe non e' piu' leggibile.
#[derive(Default)]
pub struct Decodifica {
    buf: Vec<u8>,
}

impl Decodifica {
    pub fn aggiungi(&mut self, dati: &[u8], tipi: &[u8]) -> Result<Vec<(u8, Vec<u8>)>, String> {
        self.buf.extend_from_slice(dati);
        let mut messaggi = Vec::new();
        while self.buf.len() >= 5 {
            let tipo = self.buf[0];
            let n =
                u32::from_le_bytes([self.buf[1], self.buf[2], self.buf[3], self.buf[4]]) as usize;
            if !tipi.contains(&tipo) {
                return Err(format!("tipo sconosciuto {:#04x}", tipo));
            }
            if n > MASSIMO_SCRIPT {
                return Err(format!("messaggio di {} byte, oltre il limite", n));
            }
            if self.buf.len() < 5 + n {
                break;
            }
            let resto = self.buf.split_off(5 + n);
            messaggi.push((tipo, self.buf.split_off(5)));
            self.buf = resto;
        }
        Ok(messaggi)
    }
}

/// Nell'helper: lo script dalla pipe d'ingresso, fino a [`MSG_TYPE_FINE_SCRIPT`].
/// `None` se la pipe si chiude prima, se e' illeggibile o se lo script
/// e' oltre i limiti (`script_exec::oltre_i_limiti`): allora non si esegue
/// niente.
pub fn ricevi_script<R: Read>(ingresso: &mut R) -> Option<Vec<u8>> {
    let (mut d, mut script, mut buf) = (Decodifica::default(), Vec::new(), [0u8; 4096]);
    loop {
        let n = match ingresso.read(&mut buf) {
            Ok(0) => return None,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => {
                log::warn!("exec: lettura dello script non riuscita: {}", e);
                return None;
            }
        };
        let messaggi = match d.aggiungi(&buf[..n], &[MSG_TYPE_DATA, MSG_TYPE_FINE_SCRIPT]) {
            Ok(m) => m,
            Err(e) => {
                log::warn!("exec: pipe dello script: {}", e);
                return None;
            }
        };
        for (tipo, dati) in messaggi {
            if tipo == MSG_TYPE_FINE_SCRIPT {
                return (aggiungi(&mut script, &[]) == Blocco::Completo).then_some(script);
            }
            // Un blocco vuoto non c'e': `aggiungi` lo leggerebbe come la fine.
            if !dati.is_empty() && aggiungi(&mut script, &dati) == Blocco::TroppoGrande {
                return None;
            }
        }
    }
}

/// L'helper del terminale in modo exec (`run_terminal_helper`): niente
/// ConPTY; riceve lo script, avvia il processo nel Job ([`Processo`]),
/// manda [`MSG_TYPE_PARTITO`] e solo dopo da' lo script al processo; poi
/// manda stdout e stderr sulla pipe d'uscita con l'intestazione di upstream.
/// Il valore e' il codice con cui esce l'helper: dopo "partito" mai
/// `HELPER_NON_PARTITO`.
pub fn helper<R: Read, W: Write>(mut ingresso: R, mut uscita: W) -> i32 {
    let Some(script) = ricevi_script(&mut ingresso) else {
        return HELPER_NON_PARTITO;
    };
    let (mut p, stdin) = match Processo::avvia_senza_script() {
        Ok(p) => p,
        Err(e) => {
            log::warn!("exec: lo script non e' partito: {}", e);
            return HELPER_NON_PARTITO;
        }
    };
    let partito = codifica(MSG_TYPE_PARTITO, &[]);
    if let Err(e) = uscita.write_all(&partito).and_then(|_| uscita.flush()) {
        // Lo script non si scrive: il Drop di `p` chiude il processo, che
        // non ha eseguito niente.
        log::warn!("exec: scrittura di \"partito\" non riuscita: {}", e);
        return HELPER_NON_PARTITO;
    }
    scrivi_script(stdin, script);
    loop {
        let (uscite, fine) = p.passo();
        for u in uscite {
            let m = match u {
                Uscita::Stdout(d) => codifica(MSG_TYPE_DATA, &d),
                Uscita::Stderr(d) => codifica(MSG_TYPE_STDERR, &d),
                // Nella strada diretta non arriva, e la consuma `passo`.
                Uscita::Partito => continue,
            };
            if let Err(e) = uscita.write_all(&m).and_then(|_| uscita.flush()) {
                // Il servizio non legge piu': il Drop di `p` chiude lo script.
                log::warn!("exec: scrittura dell'output non riuscita: {}", e);
                return -1;
            }
        }
        if let Some(codice) = fine {
            return codice;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// Nel servizio: legge la pipe d'uscita dell'helper fino alla sua chiusura e
/// manda stdout, stderr e "partito" a `tx`. "Partito" conta solo come primo
/// messaggio (l'helper lo scrive prima di dare lo script al processo); uno
/// che arriva dopo si scarta.
#[cfg_attr(not(windows), allow(dead_code))]
fn leggi_helper<R: Read>(mut r: R, tx: SyncSender<Uscita>) {
    let (mut d, mut buf, mut primo) = (Decodifica::default(), [0u8; 4096], true);
    let tipi = [MSG_TYPE_DATA, MSG_TYPE_STDERR, MSG_TYPE_PARTITO];
    loop {
        let n = match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                log::warn!("exec: lettura dall'helper non riuscita: {}", e);
                break;
            }
        };
        let messaggi = match d.aggiungi(&buf[..n], &tipi) {
            Ok(m) => m,
            Err(e) => {
                log::warn!("exec: pipe dell'helper: {}", e);
                break;
            }
        };
        for (tipo, dati) in messaggi {
            let u = match tipo {
                MSG_TYPE_PARTITO if primo => Uscita::Partito,
                MSG_TYPE_PARTITO => {
                    log::warn!("exec: \"partito\" dall'helper non come primo messaggio, scartato");
                    continue;
                }
                MSG_TYPE_STDERR => Uscita::Stderr(dati),
                _ => Uscita::Stdout(dati),
            };
            primo = false;
            if tx.send(u).is_err() {
                return;
            }
        }
    }
}

/// Nel servizio, con il token dell'utente: lo script va all'helper di
/// upstream, che `launch_terminal_helper_with_token` avvia come l'utente
/// della console, con le pipe a DACL ristretta. Questo modulo non crea
/// processi ne' tocca token: usa le funzioni di upstream cosi' come sono.
#[cfg(windows)]
mod helper {
    use super::*;
    use crate::server::terminal_helper::{
        create_named_pipe_server, is_helper_process_running, launch_terminal_helper_with_token,
        wait_for_pipe_connection, OwnedHandle, SendableHandle, WinCloseHandle, WinTerminateProcess,
        PIPE_CONNECTION_TIMEOUT_MS,
    };

    /// Il processo helper: chiuderlo (TerminateProcess) chiude il suo Job
    /// object, che uccide lo script e i figli.
    pub struct Helper(Option<SendableHandle>);

    impl Helper {
        pub fn uscito(&mut self) -> bool {
            self.0
                .as_ref()
                .map_or(true, |h| !is_helper_process_running(h.as_raw()))
        }

        pub fn chiudi(&mut self) -> i32 {
            let Some(h) = self.0.take() else {
                return -1;
            };
            let h = h.as_raw();
            unsafe {
                // Gia' uscito: TerminateProcess fallisce e il codice resta.
                let _ = WinTerminateProcess(h, 1);
            }
            let codice = crate::remotek::uscita_terminale::con_helper(-1, Some(h), 1000);
            if let Err(e) = unsafe { WinCloseHandle(h) } {
                log::warn!("exec: chiusura dell'handle dell'helper non riuscita: {}", e);
            }
            codice
        }
    }

    impl Drop for Helper {
        fn drop(&mut self) {
            self.chiudi();
        }
    }

    /// Le dimensioni che `--terminal-helper` vuole sulla riga di comando: in
    /// exec non c'e' una ConPTY, quindi non servono a niente.
    const RIGHE_HELPER: u16 = 24;
    const COLONNE_HELPER: u16 = 80;

    pub fn avvia(token: UserToken, script: Vec<u8>, terminal_id: i32) -> anyhow::Result<Processo> {
        let id = uuid::Uuid::new_v4();
        let nome_in = format!(r"\\.\pipe\rustdesk_term_in_{}", id);
        let nome_out = format!(r"\\.\pipe\rustdesk_term_out_{}", id);
        let pipe_in = OwnedHandle::new(create_named_pipe_server(&nome_in, false, token)?);
        let pipe_out = OwnedHandle::new(create_named_pipe_server(&nome_out, true, token)?);
        let info = launch_terminal_helper_with_token(
            token,
            &nome_in,
            &nome_out,
            terminal_id,
            RIGHE_HELPER,
            COLONNE_HELPER,
            true,
        )?;
        // Da qui il Drop chiude l'helper, anche se i passi sotto falliscono.
        let helper = Helper(Some(SendableHandle::new(info.handle)));
        let mut ingresso = wait_for_pipe_connection(pipe_in, &nome_in, PIPE_CONNECTION_TIMEOUT_MS)?;
        let mut uscita = wait_for_pipe_connection(pipe_out, &nome_out, PIPE_CONNECTION_TIMEOUT_MS)?;
        thread::spawn(move || {
            let mut m = Vec::with_capacity(script.len() + 16);
            m.extend(codifica(MSG_TYPE_DATA, &script));
            m.extend(codifica(MSG_TYPE_FINE_SCRIPT, &[]));
            if let Err(e) = ingresso.write_all(&m).and_then(|_| ingresso.flush()) {
                log::warn!(
                    "exec: scrittura dello script all'helper non riuscita: {}",
                    e
                );
            }
        });
        let (tx, rx) = sync_channel(64);
        thread::spawn(move || leggi_helper(uscita, tx));
        let figlio = Figlio::Helper(helper);
        Ok(Processo {
            figlio,
            uscita: rx,
            finito: None,
            partito: false,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use hbb_common::message_proto::{terminal_response, CloseTerminal, OpenTerminal};

    pub fn apri(id: i32) -> TerminalAction {
        let mut a = TerminalAction::new();
        a.set_open(OpenTerminal {
            terminal_id: id,
            ..Default::default()
        });
        a
    }

    pub fn manda(id: i32, d: &[u8]) -> TerminalAction {
        let mut a = TerminalAction::new();
        a.set_data(TerminalData {
            terminal_id: id,
            data: d.to_vec().into(),
            ..Default::default()
        });
        a
    }

    /// stdout, stderr e il codice, aspettando al massimo 10 secondi.
    pub fn esito(e: &mut Esecuzione) -> (Vec<u8>, Vec<u8>, Option<i32>) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let inizio = Instant::now();
        while inizio.elapsed() < Duration::from_secs(10) {
            for r in e.leggi() {
                match r.union {
                    Some(terminal_response::Union::Data(d)) if d.terminal_id == 1 => {
                        out.extend_from_slice(&d.data)
                    }
                    Some(terminal_response::Union::Data(d))
                        if d.terminal_id == TERMINAL_ID_STDERR =>
                    {
                        err.extend_from_slice(&d.data)
                    }
                    Some(terminal_response::Union::Closed(c)) => {
                        return (out, err, Some(c.exit_code))
                    }
                    altro => panic!("risposta inattesa: {:?}", altro),
                }
            }
            thread::sleep(Duration::from_millis(20));
        }
        (out, err, None)
    }

    #[test]
    fn script_in_piu_blocchi_con_flussi_separati_e_codice() {
        let mut e = Esecuzione::nuova(Some(b"echo fuori; echo dentro >&2; exit 7\n".to_vec()));
        let r = e.azione(&apri(1), None).expect("aperto");
        assert!(
            matches!(r.union, Some(terminal_response::Union::Opened(ref o)) if o.success && o.message == APERTO)
        );
        assert!(e.azione(&manda(1, b"echo fuori; ec"), None).is_none());
        assert!(e
            .azione(&manda(1, b"ho dentro >&2; exit 7\n"), None)
            .is_none());
        assert!(
            e.leggi().is_empty(),
            "non parte prima della fine dello script"
        );
        assert!(e.azione(&manda(1, b""), None).is_none());
        assert_eq!(
            esito(&mut e),
            (b"fuori\n".to_vec(), b"dentro\n".to_vec(), Some(7))
        );
    }

    #[test]
    fn oltre_i_limiti_rifiutato_senza_esecuzione() {
        let segno = std::env::temp_dir().join(format!("remotek-exec-{}", std::process::id()));
        let mut e = Esecuzione::nuova(None);
        e.azione(&apri(1), None);
        let primo = format!("touch {}\n", segno.display());
        assert!(e.azione(&manda(1, primo.as_bytes()), None).is_none());
        let rifiuto = |e: &mut Esecuzione| {
            let r = e.leggi();
            assert!(
                matches!(&r[..], [TerminalResponse { union: Some(terminal_response::Union::Error(x)), .. }] if x.message == TROPPO_GRANDE),
                "il rifiuto esce da leggi, una volta"
            );
        };
        assert!(e
            .azione(&manda(1, &vec![b'#'; MASSIMO_SCRIPT]), None)
            .is_none());
        rifiuto(&mut e);
        assert!(e.azione(&manda(1, b""), None).is_none());
        thread::sleep(Duration::from_millis(200));
        assert!(e.leggi().is_empty());
        assert!(!segno.exists());
        let mut righe = Esecuzione::nuova(None);
        righe.azione(&apri(1), None);
        let troppe = format!("{}touch {}\n", "\n".repeat(2000), segno.display());
        righe.azione(&manda(1, troppe.as_bytes()), None);
        assert!(righe.azione(&manda(1, b""), None).is_none());
        rifiuto(&mut righe);
        thread::sleep(Duration::from_millis(200));
        assert!(righe.leggi().is_empty() && !segno.exists());
        let mut s = Vec::new();
        assert_eq!(aggiungi(&mut s, &vec![0; MASSIMO_SCRIPT]), Blocco::Continua);
        assert_eq!(aggiungi(&mut s, b"x"), Blocco::TroppoGrande);
    }

    #[test]
    fn parte_solo_lo_script_mostrato_al_cliente() {
        let segno = std::env::temp_dir().join(format!("remotek-mostrato-{}", std::process::id()));
        let script = format!("touch {}\nexit 4\n", segno.display()).into_bytes();
        let mut diverso = script.clone();
        diverso[0] = b'T';
        let rifiutato = |mostrato: Option<Vec<u8>>, mandato: &[u8]| {
            let mut e = Esecuzione::nuova(mostrato);
            e.azione(&apri(1), None);
            assert!(e.azione(&manda(1, mandato), None).is_none());
            assert!(e.azione(&manda(1, b""), None).is_none());
            let r = e.leggi();
            assert!(
                matches!(&r[..], [TerminalResponse { union: Some(terminal_response::Union::Error(x)), .. }] if x.message == NON_MOSTRATO && x.terminal_id == 1),
                "il rifiuto esce da leggi, una volta: {:?}",
                r
            );
            thread::sleep(Duration::from_millis(200));
            assert!(e.leggi().is_empty());
        };
        // Diverso di un byte, o senza script mostrato: non parte.
        rifiutato(Some(script.clone()), &diverso);
        rifiutato(None, &script);
        assert!(!segno.exists(), "nessuno script partito");
        // Uguale: parte.
        let mut e = Esecuzione::nuova(Some(script.clone()));
        e.azione(&apri(1), None);
        e.azione(&manda(1, &script), None);
        e.azione(&manda(1, b""), None);
        assert_eq!(esito(&mut e).2, Some(4));
        assert!(segno.exists());
        std::fs::remove_file(&segno).ok();
    }

    #[test]
    fn chiusura_uccide_anche_un_figlio() {
        let mut e = Esecuzione::nuova(Some(b"sleep 300 & echo $!; wait\n".to_vec()));
        e.azione(&apri(1), None);
        e.azione(&manda(1, b"sleep 300 & echo $!; wait\n"), None);
        e.azione(&manda(1, b""), None);
        let mut pid = Vec::new();
        let inizio = Instant::now();
        while !pid.ends_with(b"\n") && inizio.elapsed() < Duration::from_secs(10) {
            for r in e.leggi() {
                if let Some(terminal_response::Union::Data(d)) = r.union {
                    pid.extend_from_slice(&d.data);
                }
            }
            thread::sleep(Duration::from_millis(20));
        }
        let pid: i32 = String::from_utf8_lossy(&pid)
            .trim()
            .parse()
            .expect("pid del figlio");
        let mut chiudi = TerminalAction::new();
        chiudi.set_close(CloseTerminal {
            terminal_id: 1,
            ..Default::default()
        });
        let r = e.azione(&chiudi, None).expect("chiuso");
        assert!(
            matches!(r.union, Some(terminal_response::Union::Closed(ref c)) if c.exit_code == -1)
        );
        thread::sleep(Duration::from_millis(100));
        // Il figlio e' stato raccolto da init o e' uno zombie: non gira piu'.
        let stato = std::fs::read_to_string(format!("/proc/{}/stat", pid)).unwrap_or_default();
        assert!(
            stato.is_empty() || stato.contains(") Z "),
            "figlio vivo: {}",
            stato
        );
    }

    #[test]
    fn decodifica_blocchi_spezzati_tipo_sconosciuto_e_lunghezza() {
        let tipi = [MSG_TYPE_DATA, MSG_TYPE_STDERR];
        let mut tutto = codifica(MSG_TYPE_DATA, b"ciao");
        tutto.extend(codifica(MSG_TYPE_STDERR, b""));
        let mut d = Decodifica::default();
        let mut visti = Vec::new();
        for byte in tutto.chunks(3) {
            visti.extend(d.aggiungi(byte, &tipi).expect("valido"));
        }
        assert_eq!(
            visti,
            vec![(MSG_TYPE_DATA, b"ciao".to_vec()), (MSG_TYPE_STDERR, vec![])]
        );
        let err = Decodifica::default().aggiungi(&codifica(0x02, b"xy"), &tipi);
        assert!(err.is_err());
        let mut lungo = vec![MSG_TYPE_DATA];
        lungo.extend_from_slice(&(MASSIMO_SCRIPT as u32 + 1).to_le_bytes());
        assert!(Decodifica::default().aggiungi(&lungo, &tipi).is_err());
    }

    #[test]
    fn ricevi_script_solo_con_la_fine() {
        let mut m = codifica(MSG_TYPE_DATA, b"echo ");
        m.extend(codifica(MSG_TYPE_DATA, b"ok"));
        assert_eq!(
            ricevi_script(&mut m.as_slice()),
            None,
            "pipe chiusa prima della fine"
        );
        m.extend(codifica(MSG_TYPE_FINE_SCRIPT, b""));
        assert_eq!(ricevi_script(&mut m.as_slice()), Some(b"echo ok".to_vec()));
        let mut grande = codifica(MSG_TYPE_DATA, &vec![b'#'; MASSIMO_SCRIPT]);
        grande.extend(codifica(MSG_TYPE_DATA, b"x"));
        grande.extend(codifica(MSG_TYPE_FINE_SCRIPT, b""));
        assert_eq!(ricevi_script(&mut grande.as_slice()), None);
        let mut righe = codifica(MSG_TYPE_DATA, &[b'\n'; 2001]);
        righe.extend(codifica(MSG_TYPE_FINE_SCRIPT, b""));
        assert_eq!(
            ricevi_script(&mut righe.as_slice()),
            None,
            "oltre 2000 righe"
        );
    }

    #[test]
    fn helper_manda_i_due_flussi_ed_esce_col_codice() {
        let mut m = codifica(MSG_TYPE_DATA, b"echo o; echo e >&2; exit 4\n");
        m.extend(codifica(MSG_TYPE_FINE_SCRIPT, b""));
        let mut uscita = Vec::new();
        assert_eq!(helper(m.as_slice(), &mut uscita), 4);
        let mut d = Decodifica::default();
        let mut visti = d
            .aggiungi(&uscita, &[MSG_TYPE_DATA, MSG_TYPE_STDERR, MSG_TYPE_PARTITO])
            .expect("valido");
        assert_eq!(
            visti.first(),
            Some(&(MSG_TYPE_PARTITO, vec![])),
            "\"partito\" e' il primo messaggio"
        );
        let mut visti = visti.split_off(1);
        visti.sort();
        assert_eq!(
            visti,
            vec![
                (MSG_TYPE_DATA, b"o\n".to_vec()),
                (MSG_TYPE_STDERR, b"e\n".to_vec())
            ]
        );
        let mut niente = Vec::new();
        let troncato = codifica(MSG_TYPE_DATA, b"touch /nonesiste\n");
        assert_eq!(helper(troncato.as_slice(), &mut niente), HELPER_NON_PARTITO);
        assert!(niente.is_empty(), "senza script neanche \"partito\"");
    }

    #[test]
    fn il_codice_dello_script_passa_da_codice_dello_script() {
        use crate::remotek::uscita_terminale::CODICE_SCRIPT_SOSTITUITO;
        assert_eq!(
            codice_da(Some(HELPER_NON_PARTITO), None),
            CODICE_SCRIPT_SOSTITUITO
        );
        assert_eq!(codice_da(Some(3), None), 3);
        assert_eq!(codice_da(None, None), -1);
        assert_eq!(codice_da(None, Some(9)), 137);
    }

    /// Una pipe d'uscita che il servizio non legge piu'.
    struct PipeRotta;

    impl Write for PipeRotta {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::ErrorKind::BrokenPipe.into())
        }
        fn flush(&mut self) -> io::Result<()> {
            Err(io::ErrorKind::BrokenPipe.into())
        }
    }

    #[test]
    fn partito_arriva_prima_dello_script() {
        let cartella =
            std::env::temp_dir().join(format!("remotek-exec-partito-{}", std::process::id()));
        std::fs::create_dir_all(&cartella).expect("cartella temporanea");
        let segno = cartella.join("segno");
        let mut m = codifica(
            MSG_TYPE_DATA,
            format!("touch {}\n", segno.display()).as_bytes(),
        );
        m.extend(codifica(MSG_TYPE_FINE_SCRIPT, b""));
        // "partito" non si scrive: lo script non arriva al processo.
        assert_eq!(helper(m.as_slice(), PipeRotta), HELPER_NON_PARTITO);
        assert!(!segno.exists(), "lo script e' girato senza \"partito\"");
        // Controprova: con la pipe che accetta, lo stesso script gira.
        assert_eq!(helper(m.as_slice(), Vec::new()), 0);
        assert!(segno.exists());
        std::fs::remove_dir_all(&cartella).expect("pulizia");
    }

    #[test]
    fn il_servizio_accetta_partito_solo_come_primo_messaggio() {
        let leggi = |pipe: Vec<u8>| {
            let (tx, rx) = sync_channel(64);
            leggi_helper(pipe.as_slice(), tx);
            rx.try_iter().collect::<Vec<_>>()
        };
        let mut primo = codifica(MSG_TYPE_PARTITO, &[]);
        primo.extend(codifica(MSG_TYPE_DATA, b"a"));
        primo.extend(codifica(MSG_TYPE_STDERR, b"b"));
        primo.extend(codifica(MSG_TYPE_DATA, b"c"));
        assert_eq!(
            leggi(primo),
            vec![
                Uscita::Partito,
                Uscita::Stdout(b"a".to_vec()),
                Uscita::Stderr(b"b".to_vec()),
                Uscita::Stdout(b"c".to_vec())
            ]
        );
        let mut dopo = codifica(MSG_TYPE_DATA, b"a");
        dopo.extend(codifica(MSG_TYPE_PARTITO, &[]));
        dopo.extend(codifica(MSG_TYPE_DATA, b"b"));
        assert_eq!(
            leggi(dopo),
            vec![Uscita::Stdout(b"a".to_vec()), Uscita::Stdout(b"b".to_vec())],
            "\"partito\" dopo i dati si scarta"
        );
    }
}
