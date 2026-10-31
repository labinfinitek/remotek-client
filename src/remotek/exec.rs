//! Exec (ADR-0033 passo 2): il terminale senza pseudo-console. L'agente manda
//! uno script intero e riceve stdout, stderr e codice d'uscita separati.
//!
//! La sessione e' di exec quando il lasciapassare della connessione e' di exec
//! (`lasciapassare::exec`); consenso, lasciapassare, trascrizione e identita'
//! sono quelli del terminale: questo modulo non sceglie l'utente, esegue
//! lo script nel processo del servizio del terminale, che gira gia' come
//! l'utente deciso da `fill_terminal_user_token` (src/server/connection.rs).
//!
//! Il contratto col CLI:
//! - all'`OpenTerminal` il PC risponde `TerminalOpened` con `success` e
//!   `message` = [`APERTO`], senza aprire una pseudo-console;
//! - lo script arriva nei `TerminalData` del `terminal_id` aperto, fino a un
//!   `TerminalData` vuoto, che e' la fine dello script. Il client Flutter non
//!   manda mai dati vuoti (`send_terminal_input` di src/ui_session_interface.rs
//!   porta l'input di xterm, mai vuoto), e una sessione di exec esiste solo
//!   col lasciapassare di exec, che chiede solo `remotek-cli`;
//! - oltre [`MASSIMO_SCRIPT`] byte: `TerminalError` con [`TROPPO_GRANDE`],
//!   la sessione si chiude e non si esegue niente;
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

use super::uscita_terminale::{risposta_chiusura, HELPER_NON_PARTITO};
use hbb_common::log;
use hbb_common::message_proto::{
    terminal_action, TerminalAction, TerminalData, TerminalError, TerminalOpened, TerminalResponse,
};
use std::{
    io::{self, Read, Write},
    process::{Child, Command, ExitStatus, Stdio},
    sync::mpsc::{sync_channel, Receiver, SyncSender, TryRecvError},
    thread,
    time::{Duration, Instant},
};

/// Il `message` del `TerminalOpened` di una sessione di exec.
pub const APERTO: &str = "remotek-exec";
/// Il `terminal_id` di stderr. Il client Flutter numera i terminali da 1 in
/// su (`_nextTerminalId` di flutter/lib/desktop/pages/terminal_tab_page.dart).
pub const TERMINAL_ID_STDERR: i32 = -2;
/// Lo script piu' grande che si accetta: 1 MiB.
pub const MASSIMO_SCRIPT: usize = 1024 * 1024;
/// Il messaggio del `TerminalError` per uno script oltre [`MASSIMO_SCRIPT`].
pub const TROPPO_GRANDE: &str = "lo script supera 1 MiB: non eseguito";
/// Finito lo script, quanto si aspetta ancora l'output gia' scritto.
const ATTESA_OUTPUT: Duration = Duration::from_secs(2);

/// Il comando fisso di PowerShell: UTF-8 in ingresso e in uscita, poi tutto
/// lo stdin come un solo scriptblock (`exit N` da' il codice N), senza file
/// .ps1 e quindi senza ExecutionPolicy.
#[cfg(windows)]
const COMANDO_POWERSHELL: &str = "$u = New-Object System.Text.UTF8Encoding $false; \
[Console]::InputEncoding = $u; [Console]::OutputEncoding = $u; $OutputEncoding = $u; \
& ([scriptblock]::Create([Console]::In.ReadToEnd()))";

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

/// Il codice d'uscita; ucciso da un segnale: 128 + il segnale, come le shell.
fn codice(stato: ExitStatus) -> i32 {
    #[cfg(unix)]
    if let Some(s) = std::os::unix::process::ExitStatusExt::signal(&stato) {
        return 128 + s;
    }
    stato.code().unwrap_or(-1)
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

enum Uscita {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
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
    child: Child,
    albero: Albero,
    uscita: Receiver<Uscita>,
    /// Il codice, e da quando, una volta finito lo script.
    finito: Option<(i32, Instant)>,
}

impl Processo {
    fn avvia(script: Vec<u8>) -> io::Result<Self> {
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
        let (Some(mut stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            return Err(io::Error::other("pipe del processo mancanti"));
        };
        let (tx, uscita) = sync_channel(64);
        lettore(stdout, tx.clone(), false);
        lettore(stderr, tx, true);
        // In un thread: uno script che non legge lo stdin non blocca il
        // servizio. Lo stdin si chiude a fine thread.
        thread::spawn(move || {
            if let Err(e) = stdin.write_all(&script) {
                log::warn!("exec: scrittura dello script non riuscita: {}", e);
            }
        });
        Ok(Self {
            child,
            albero,
            uscita,
            finito: None,
        })
    }

    /// Lo script e' finito, senza raccogliere il processo (vedi [`Albero`]).
    #[cfg(not(windows))]
    fn uscito(&mut self) -> bool {
        use hbb_common::libc;
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let flag = libc::WEXITED | libc::WNOHANG | libc::WNOWAIT;
        let r = unsafe { libc::waitid(libc::P_PID, self.child.id(), &mut info, flag) };
        r == 0 && unsafe { info.si_pid() } != 0
    }

    #[cfg(windows)]
    fn uscito(&mut self) -> bool {
        // Su Windows l'handle del processo resta aperto: niente riuso.
        matches!(self.child.try_wait(), Ok(Some(_)))
    }

    /// Uccide i figli rimasti e raccoglie il processo.
    fn chiudi(&mut self) -> i32 {
        self.albero.uccidi();
        match self.child.wait() {
            Ok(s) => codice(s),
            Err(e) => {
                log::warn!("exec: attesa del processo non riuscita: {}", e);
                -1
            }
        }
    }
}

impl Drop for Processo {
    fn drop(&mut self) {
        if self.finito.is_none() {
            self.chiudi();
        }
    }
}

/// Una sessione di exec nel servizio del terminale: un solo script.
pub struct Esecuzione {
    /// Il servizio ha il token dell'utente della console (Windows
    /// installato): lo script andra' nell'helper (MR 3b); fino ad allora non
    /// parte, mai col token del servizio.
    con_helper: bool,
    terminal_id: Option<i32>,
    script: Vec<u8>,
    processo: Option<Processo>,
    chiusa: bool,
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
    pub fn nuova(con_helper: bool) -> Self {
        Self {
            con_helper,
            terminal_id: None,
            script: Vec::new(),
            processo: None,
            chiusa: false,
        }
    }

    pub fn azione(&mut self, azione: &TerminalAction) -> Option<TerminalResponse> {
        match &azione.union {
            Some(terminal_action::Union::Open(o)) => {
                if self.terminal_id.is_some() {
                    return Some(errore(
                        o.terminal_id,
                        "exec: un solo script per connessione",
                    ));
                }
                self.terminal_id = Some(o.terminal_id);
                if self.con_helper {
                    log::warn!("exec: con l'helper non ancora: non parte");
                    self.chiusa = true;
                    return Some(risposta_chiusura(o.terminal_id, HELPER_NON_PARTITO));
                }
                let mut r = TerminalResponse::new();
                r.set_opened(TerminalOpened {
                    terminal_id: o.terminal_id,
                    success: true,
                    message: APERTO.to_owned(),
                    ..Default::default()
                });
                Some(r)
            }
            Some(terminal_action::Union::Data(d)) => {
                if self.terminal_id != Some(d.terminal_id) || self.processo.is_some() || self.chiusa
                {
                    log::warn!("exec: dati ignorati per il terminale {}", d.terminal_id);
                    return None;
                }
                match aggiungi(&mut self.script, &d.data) {
                    Blocco::Continua => None,
                    Blocco::TroppoGrande => {
                        self.chiusa = true;
                        self.script = Vec::new();
                        Some(errore(d.terminal_id, TROPPO_GRANDE))
                    }
                    Blocco::Completo => match Processo::avvia(std::mem::take(&mut self.script)) {
                        Ok(p) => {
                            self.processo = Some(p);
                            None
                        }
                        Err(e) => {
                            log::warn!("exec: lo script non e' partito: {}", e);
                            self.chiusa = true;
                            Some(risposta_chiusura(d.terminal_id, HELPER_NON_PARTITO))
                        }
                    },
                }
            }
            Some(terminal_action::Union::Close(c)) if self.terminal_id == Some(c.terminal_id) => {
                if self.chiusa {
                    return None;
                }
                self.chiusa = true;
                // -1: chiusura forzata, come il terminale (`handle_close`).
                // Il Drop del processo uccide lo script e i figli.
                self.processo = None;
                Some(risposta_chiusura(c.terminal_id, -1))
            }
            _ => None,
        }
    }

    /// Le risposte pronte: l'output, e alla fine la chiusura col codice.
    pub fn leggi(&mut self) -> Vec<TerminalResponse> {
        let (Some(id), Some(p)) = (self.terminal_id, self.processo.as_mut()) else {
            return Vec::new();
        };
        let mut risposte = Vec::new();
        let mut lettori_finiti = false;
        loop {
            match p.uscita.try_recv() {
                Ok(Uscita::Stdout(d)) => risposte.push(dati(id, d)),
                Ok(Uscita::Stderr(d)) => risposte.push(dati(TERMINAL_ID_STDERR, d)),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    lettori_finiti = true;
                    break;
                }
            }
        }
        if p.finito.is_none() && p.uscito() {
            p.finito = Some((p.chiudi(), Instant::now()));
        }
        if let Some((codice, da)) = p.finito {
            if lettori_finiti || da.elapsed() > ATTESA_OUTPUT {
                self.processo = None;
                self.chiusa = true;
                risposte.push(risposta_chiusura(id, codice));
            }
        }
        risposte
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
        let mut e = Esecuzione::nuova(false);
        let r = e.azione(&apri(1)).expect("aperto");
        assert!(
            matches!(r.union, Some(terminal_response::Union::Opened(ref o)) if o.success && o.message == APERTO)
        );
        assert!(e.azione(&manda(1, b"echo fuori; ec")).is_none());
        assert!(e.azione(&manda(1, b"ho dentro >&2; exit 7\n")).is_none());
        assert!(
            e.leggi().is_empty(),
            "non parte prima della fine dello script"
        );
        assert!(e.azione(&manda(1, b"")).is_none());
        assert_eq!(
            esito(&mut e),
            (b"fuori\n".to_vec(), b"dentro\n".to_vec(), Some(7))
        );
    }

    #[test]
    fn oltre_un_mib_rifiutato_senza_esecuzione() {
        let segno = std::env::temp_dir().join(format!("remotek-exec-{}", std::process::id()));
        let mut e = Esecuzione::nuova(false);
        e.azione(&apri(1));
        let primo = format!("touch {}\n", segno.display());
        assert!(e.azione(&manda(1, primo.as_bytes())).is_none());
        let r = e
            .azione(&manda(1, &vec![b'#'; MASSIMO_SCRIPT]))
            .expect("errore");
        assert!(
            matches!(r.union, Some(terminal_response::Union::Error(ref x)) if x.message == TROPPO_GRANDE)
        );
        assert!(e.azione(&manda(1, b"")).is_none());
        thread::sleep(Duration::from_millis(200));
        assert!(e.leggi().is_empty());
        assert!(!segno.exists());
        let mut s = Vec::new();
        assert_eq!(aggiungi(&mut s, &vec![0; MASSIMO_SCRIPT]), Blocco::Continua);
        assert_eq!(aggiungi(&mut s, b"x"), Blocco::TroppoGrande);
    }

    #[test]
    fn chiusura_uccide_anche_un_figlio() {
        let mut e = Esecuzione::nuova(false);
        e.azione(&apri(1));
        e.azione(&manda(1, b"sleep 300 & echo $!; wait\n"));
        e.azione(&manda(1, b""));
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
        let r = e.azione(&chiudi).expect("chiuso");
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
}
