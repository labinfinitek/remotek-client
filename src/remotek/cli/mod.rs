//! `remotek-cli`: il client a riga di comando con cui l'agente AI di Infinitek
//! usa il terminale dei PC dei clienti (ADR-0021). Entra solo se il cliente
//! accetta la finestra: nessuna password del PC, in nessun modo.

mod account;
mod terminale;
mod tunnel;

use std::io::Write;

const USO: &str = "uso:
  remotek-cli login <utente>   la password dell'account dell'agente si legge da stdin, una riga
  remotek-cli logout
  remotek-cli whoami
  remotek-cli terminal <ID> [--amministratore] [--attesa <secondi>] [--righe <n>] [--colonne <n>]
                               stdin va al terminale del PC, il terminale su stdout;
                               prima controlla l'accesso sull'API: scaduto o revocato, toglie
                               le credenziali (rifare login); --amministratore chiede all'API
                               un lasciapassare per quel PC (ADR-0028) e lo manda al PC;
                               si entra solo se il cliente accetta (attesa di default 120 s);
                               righe e colonne da 1 a 65535 (default 24 e 80);
                               esce col codice del terminale (fuori da 0-255: 255); 0 anche
                               quando il PC non conosce il codice della shell; 125 se il
                               terminale non e' partito sul PC (lo dice anche su stderr);
                               126 se il PC non dichiara la trascrizione e la vista del
                               terminale (va aggiornato; lo dice anche su stderr): sul PC
                               non si apre niente;
                               la fine di stdin chiude subito il terminale e la shell: per il
                               codice della shell si manda `exit` e si tiene stdin aperto
  remotek-cli exec <ID> [--amministratore] [--attesa <secondi>]
                               esegue sul PC lo script letto da stdin (tutto, prima di
                               collegarsi; al massimo 32 KiB e 2000 righe, altrimenti esce
                               con 2) senza pseudo-console: PowerShell su Windows, sh su
                               Linux; chiede sempre all'API un lasciapassare di exec, da
                               utente o con --amministratore, e il cliente deve accettare;
                               stdout e stderr del PC su stdout e stderr; esce col codice
                               dello script, con le regole di terminal (fuori da 0-255: 255;
                               125 se non partito, 126 se il PC non dichiara, anche quando
                               non mostra lo script prima della scelta: va aggiornato);
                               lo script va al PC col login: il cliente lo vede intero
                               nella finestra della richiesta, prima di accettare, e il PC
                               esegue solo quello: niente segreti nel testo dello script
  remotek-cli tunnel <ID> <locale>:<porta> [<locale>:<porta> ...] [--durata <secondi>] [--attesa <secondi>]
                               ogni connessione TCP a 127.0.0.1:<locale> di questa macchina va a
                               127.0.0.1:<porta> del PC, tutte in una sola connessione (ADR-0036);
                               da 1 a 8 porte da 1 a 65535, locali diverse e porte diverse;
                               chiede all'API un lasciapassare di tunnel da utente per quelle
                               porte del PC (durata da 60 a 28800 secondi, 3600 senza
                               --durata) dopo aver preso le porte locali; il cliente deve
                               accettare (attesa di default 120 s), tranne sul server non
                               presidiato che lascia entrare il tunnel verso quelle porte
                               senza clic; una riga su stderr per
                               ogni porta in ascolto e alla chiusura (motivo, flussi, byte);
                               resta in primo piano fino a SIGINT o SIGTERM (Ctrl+C su
                               Windows), alla chiusura del PC o alla fine della durata, e
                               non si ricollega; esce con 0 se l'ha chiuso l'agente o e'
                               finita la durata, 1 per errori di rete o dell'API e quando
                               il PC chiude per altri motivi (il cliente, un'ora senza
                               traffico), 2 per argomenti sbagliati, 126 se il PC non
                               dichiara il tunnel (va aggiornato; lo dice anche su stderr);
                               un PC fino al remotek-1.4.9-8 non sa del tunnel: dopo il clic
                               del cliente rifiuta il login e il CLI esce con 1, col
                               messaggio del PC
il CLI ha una cartella di configurazione sua (account, ID e chiavi), separata da quella
del client grafico dello stesso utente;
su Windows stdin, stdout e, con exec, stderr vanno rediretti (pipe o file): su una
console i byte non UTF-8 sono rifiutati";

/// Uscita per un errore dell'API, di rete o di configurazione.
pub(crate) const USCITA_ERRORE: i32 = 1;
/// Uscita per argomenti sbagliati.
pub(crate) const USCITA_USO: i32 = 2;
/// Uscita quando il terminale non e' partito sul PC (l'helper e' uscito prima
/// della shell): 125, come `env` e i lanciatori quando il comando non parte.
/// Una shell che esce da se' con 125 si distingue solo da stderr.
pub(crate) const USCITA_NON_PARTITO: i32 = 125;
/// Uscita quando il PC non dichiara la trascrizione e la vista del terminale
/// (ADR-0034, src/remotek/dichiarazione.rs): ne' terminale ne' exec si aprono;
/// con exec anche quando non dichiara lo script nella scelta (ADR-0035).
/// 126, il codice dei comandi che non si possono eseguire. Una shell o uno
/// script che esce da se' con 126 si distingue solo da stderr, come per il 125.
pub(crate) const USCITA_NON_DICHIARA: i32 = 126;

#[derive(Debug, PartialEq)]
enum Comando {
    Login(String),
    Logout,
    Whoami,
    Terminale {
        id: String,
        attesa: u64,
        righe: u32,
        colonne: u32,
        amministratore: bool,
    },
    Exec {
        id: String,
        attesa: u64,
        amministratore: bool,
    },
    Tunnel {
        id: String,
        porte: Vec<tunnel::Porta>,
        durata: Option<u32>,
        attesa: u64,
    },
    Aiuto,
}

/// L'ID del PC: niente `/r`, `@server` o altro che il client interpreta.
fn controlla_id(id: &str) -> Result<(), String> {
    let valido = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    if id.is_empty() || id.starts_with('-') || !id.chars().all(valido) {
        return Err("ID del PC non valido".to_owned());
    }
    Ok(())
}

/// `tunnel <ID> <locale>:<porta> ... [--durata <s>] [--attesa <s>]`.
fn leggi_tunnel(id: &str, resto: &[&str]) -> Result<Comando, String> {
    controlla_id(id)?;
    let (mut durata, mut attesa, mut porte) = (None, 120u64, Vec::new());
    let mut parole = resto.iter();
    while let Some(parola) = parole.next() {
        match *parola {
            "--durata" => durata = Some(tunnel::leggi_durata(parole.next().unwrap_or(&""))?),
            "--attesa" => {
                attesa = parole
                    .next()
                    .and_then(|v| v.parse::<u32>().ok())
                    .filter(|n| *n > 0)
                    .ok_or("argomenti non validi")?
                    .into()
            }
            p => porte.push(p),
        }
    }
    Ok(Comando::Tunnel {
        id: id.to_owned(),
        porte: tunnel::leggi_porte(&porte)?,
        durata,
        attesa,
    })
}

/// `terminal <ID>` con le tre opzioni numeriche e `--amministratore`; nessuna
/// porta una password.
fn leggi_terminale(id: &str, opzioni: &[&str]) -> Result<Comando, String> {
    controlla_id(id)?;
    let (mut attesa, mut righe, mut colonne) = (
        120,
        terminale::RIGHE_PREDEFINITE,
        terminale::COLONNE_PREDEFINITE,
    );
    let mut amministratore = false;
    let mut resto = opzioni.iter();
    while let Some(opzione) = resto.next() {
        if *opzione == "--amministratore" {
            amministratore = true;
            continue;
        }
        let valore = resto
            .next()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|n| *n > 0);
        // Il PC converte righe e colonne in u16 (src/server/terminal_service.rs).
        let dimensione = |n: u32| n <= u32::from(u16::MAX);
        match (*opzione, valore) {
            ("--attesa", Some(n)) => attesa = n,
            ("--righe", Some(n)) if dimensione(n) => righe = n,
            ("--colonne", Some(n)) if dimensione(n) => colonne = n,
            _ => return Err("argomenti non validi".to_owned()),
        }
    }
    Ok(Comando::Terminale {
        id: id.to_owned(),
        attesa: attesa.into(),
        righe,
        colonne,
        amministratore,
    })
}

/// Gli argomenti si leggono a mano: nessuna opzione e' ammessa, quindi nemmeno
/// una che porti una password.
fn leggi_argomenti(args: &[String]) -> Result<Comando, String> {
    let parole: Vec<&str> = args.iter().map(String::as_str).collect();
    match parole.as_slice() {
        ["-h"] | ["--help"] | ["help"] => Ok(Comando::Aiuto),
        ["login", utente] if !utente.starts_with('-') && !utente.is_empty() => {
            Ok(Comando::Login(utente.to_string()))
        }
        ["logout"] => Ok(Comando::Logout),
        ["whoami"] => Ok(Comando::Whoami),
        ["terminal", id, opzioni @ ..] => leggi_terminale(id, opzioni),
        // Le opzioni di terminal, senza righe e colonne.
        ["exec", id, opzioni @ ..]
            if !opzioni.iter().any(|o| *o == "--righe" || *o == "--colonne") =>
        {
            match leggi_terminale(id, opzioni)? {
                Comando::Terminale {
                    id,
                    attesa,
                    amministratore,
                    ..
                } => Ok(Comando::Exec {
                    id,
                    attesa,
                    amministratore,
                }),
                _ => Err("argomenti non validi".to_owned()),
            }
        }
        ["tunnel", id, resto @ ..] => leggi_tunnel(id, resto),
        [] => Err("manca il comando".to_owned()),
        // Gli argomenti non si ripetono: potrebbero contenere una password.
        _ => Err("argomenti non validi".to_owned()),
    }
}

/// L'unico runtime del CLI, per le richieste all'API. Il terminale non lo usa:
/// `io_loop` gira nel suo thread come in src/flutter.rs (`session_start_`).
fn nel_runtime<T, F: std::future::Future<Output = Result<T, String>>>(
    futuro: F,
) -> Result<T, String> {
    hbb_common::tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("runtime: {e}"))?
        .block_on(futuro)
}

/// Il suffisso del nome dell'applicazione per il CLI: la cartella di
/// configurazione (`Config::path`, file `<nome>*.toml`, rubrica, PC) e' quella
/// di `<APP_NAME>-cli`, non quella del client grafico.
const SUFFISSO_CLI: &str = "-cli";

fn nome_cli(app: &str) -> String {
    if app.ends_with(SUFFISSO_CLI) {
        app.to_owned()
    } else {
        format!("{app}{SUFFISSO_CLI}")
    }
}

/// L'inverso di `nome_cli`: il nome del client sul PC, per i messaggi
/// all'agente. Nel processo del CLI `APP_NAME` vale `<nome>-cli`.
fn nome_client(app: &str) -> &str {
    app.strip_suffix(SUFFISSO_CLI).unwrap_or(app)
}

/// Il marchio del client (APP_NAME senza `SUFFISSO_CLI`).
pub(super) fn marchio() -> String {
    nome_client(&crate::get_app_name()).to_owned()
}

/// Da chiamare prima di ogni accesso alla configurazione: `Config` e
/// `LocalConfig` si caricano al primo uso, dal file di `APP_NAME` di quel
/// momento, e restano in memoria. Un `login` del CLI cosi' non tocca account,
/// ID e chiavi del client.
fn usa_configurazione_del_cli() {
    let mut nome = hbb_common::config::APP_NAME.write().unwrap();
    *nome = nome_cli(&nome);
}

/// Punto d'ingresso del binario: restituisce il codice d'uscita.
pub fn main() -> i32 {
    avvia(std::env::args_os().skip(1))
}

fn avvia(argomenti: impl Iterator<Item = std::ffi::OsString>) -> i32 {
    // Prima di tutto il resto, configurazione compresa.
    usa_configurazione_del_cli();
    // ADR-0021, regola 3: mai il modo amministratore di upstream; da ADR-0028
    // l'amministratore si chiede solo col lasciapassare.
    std::env::remove_var("IS_TERMINAL_ADMIN");
    // `args()` andrebbe in panic, stampandolo, su un argomento non UTF-8.
    let args: Result<Vec<String>, _> = argomenti.map(|a| a.into_string()).collect();
    let comando = args
        .map_err(|_| "argomenti non validi".to_owned())
        .and_then(|a| leggi_argomenti(&a));
    let esito = match comando {
        Ok(Comando::Aiuto) => {
            println!("{USO}");
            return 0;
        }
        Ok(Comando::Login(utente)) => {
            account::leggi_password().and_then(|pw| nel_runtime(account::login(&utente, &pw)))
        }
        Ok(Comando::Logout) => nel_runtime(account::logout()),
        Ok(Comando::Whoami) => account::whoami(),
        Ok(Comando::Terminale {
            id,
            attesa,
            righe,
            colonne,
            amministratore,
        }) => {
            let esito = avvia_terminale(
                // Il cliente deve vedere "Agente AI per <tecnico>", non l'utente Linux.
                || account::whoami().and_then(|_| nel_runtime(account::verifica_accesso())),
                || {
                    // Dopo i controlli locali del terminale: vale 60 secondi.
                    let chiedi =
                        amministratore.then_some(|| nel_runtime(account::lasciapassare(&id)));
                    terminale::terminale(&id, attesa, righe, colonne, chiedi)
                },
            );
            return match esito {
                Ok(codice) => codice,
                Err(e) => {
                    eprintln!("remotek-cli: {e}");
                    USCITA_ERRORE
                }
            };
        }
        Ok(Comando::Exec {
            id,
            attesa,
            amministratore,
        }) => {
            // Tutto lo script prima di collegarsi.
            let script = match terminale::leggi_script(std::io::stdin().lock()) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("remotek-cli: {}", e.messaggio());
                    return e.codice();
                }
            };
            let esito = avvia_terminale(
                || account::whoami().and_then(|_| nel_runtime(account::verifica_accesso())),
                || {
                    terminale::exec(&id, attesa, amministratore, script, || {
                        nel_runtime(account::lasciapassare_exec(&id, amministratore))
                    })
                },
            );
            return match esito {
                Ok(codice) => codice,
                Err(e) => {
                    eprintln!("remotek-cli: {e}");
                    USCITA_ERRORE
                }
            };
        }
        Ok(Comando::Tunnel {
            id,
            porte,
            durata,
            attesa,
        }) => {
            let esito = avvia_terminale(
                || account::whoami().and_then(|_| nel_runtime(account::verifica_accesso())),
                || {
                    tunnel::tunnel(&id, &porte, attesa, |remote| {
                        nel_runtime(account::lasciapassare_tunnel(&id, remote, durata))
                    })
                },
            );
            return match esito {
                Ok(codice) => codice,
                Err(e) => {
                    eprintln!("remotek-cli: {e}");
                    USCITA_ERRORE
                }
            };
        }
        Err(e) => {
            eprintln!("remotek-cli: {e}\n{USO}");
            return USCITA_USO;
        }
    };
    match esito {
        Ok(testo) => {
            let mut out = std::io::stdout().lock();
            if let Err(e) = writeln!(out, "{testo}") {
                eprintln!("remotek-cli: stdout: {e}");
                return USCITA_ERRORE;
            }
            0
        }
        Err(e) => {
            eprintln!("remotek-cli: {e}");
            USCITA_ERRORE
        }
    }
}

/// Il terminale si apre solo dopo che l'accesso e' verificato; il
/// lasciapassare lo chiede `collega` (src/remotek/cli/terminale.rs,
/// `prepara`).
fn avvia_terminale(
    verifica: impl FnOnce() -> Result<(), String>,
    collega: impl FnOnce() -> Result<i32, String>,
) -> Result<i32, String> {
    verifica()?;
    collega()
}

/// I test che toccano l'ambiente o le opzioni locali non girano in parallelo:
/// `initialize` legge IS_TERMINAL_ADMIN, il CLI token e `user_info`.
#[cfg(test)]
static AMBIENTE: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    fn leggi(a: &[&str]) -> Result<Comando, String> {
        leggi_argomenti(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn comandi_validi() {
        assert_eq!(
            leggi(&["login", "agente1"]),
            Ok(Comando::Login("agente1".into()))
        );
        assert_eq!(leggi(&["logout"]), Ok(Comando::Logout));
        assert_eq!(leggi(&["whoami"]), Ok(Comando::Whoami));
        assert_eq!(leggi(&["--help"]), Ok(Comando::Aiuto));
        let t = |attesa, righe, colonne| Comando::Terminale {
            id: "123456789".into(),
            attesa,
            righe,
            colonne,
            amministratore: false,
        };
        assert_eq!(leggi(&["terminal", "123456789"]), Ok(t(120, 24, 80)));
        assert_eq!(
            leggi(&[
                "terminal",
                "123456789",
                "--colonne",
                "132",
                "--attesa",
                "30"
            ]),
            Ok(t(30, 24, 132))
        );
        assert_eq!(
            leggi(&["terminal", "123456789", "--righe", "65535"]),
            Ok(t(120, 65535, 80))
        );
        assert_eq!(
            leggi(&[
                "terminal",
                "123456789",
                "--amministratore",
                "--attesa",
                "30"
            ]),
            Ok(Comando::Terminale {
                id: "123456789".into(),
                attesa: 30,
                righe: 24,
                colonne: 80,
                amministratore: true,
            })
        );
    }

    #[test]
    fn il_comando_tunnel() {
        let porta = |locale, remota| tunnel::Porta { locale, remota };
        assert_eq!(
            leggi(&["tunnel", "123456789", "1433:1433"]),
            Ok(Comando::Tunnel {
                id: "123456789".into(),
                porte: vec![porta(1433, 1433)],
                durata: None,
                attesa: 120,
            })
        );
        assert_eq!(
            leggi(&["tunnel", "123456789", "15433:1433", "--durata", "600", "18080:8080", "--attesa", "30"]),
            Ok(Comando::Tunnel {
                id: "123456789".into(),
                porte: vec![porta(15433, 1433), porta(18080, 8080)],
                durata: Some(600),
                attesa: 30,
            })
        );
        for sbagliato in [
            &["tunnel", "123456789"][..],
            &["tunnel", "123456789", "0:1433"],
            &["tunnel", "123456789", "1433:70000"],
            &["tunnel", "123456789", "1433:1433", "2433:1433"],
            &["tunnel", "123456789", "1433:1433", "--durata", "30"],
            &["tunnel", "123456789", "1433:1433", "--durata"],
            &["tunnel", "123456789", "1433:1433", "--attesa", "0"],
            &["tunnel", "123456789", "1433:1433", "--amministratore"],
            &["tunnel", "12/r", "1433:1433"],
            &["tunnel", "-1", "1433:1433"],
        ] {
            assert!(leggi(sbagliato).is_err(), "{sbagliato:?}");
        }
        let nove: Vec<String> = (1..=9).map(|i| format!("{}:{}", 2000 + i, 3000 + i)).collect();
        let mut parole = vec!["tunnel", "123456789"];
        parole.extend(nove.iter().map(String::as_str));
        assert!(leggi(&parole).is_err());
        parole.pop();
        assert!(leggi(&parole).is_ok());
    }

    #[test]
    fn senza_accesso_nessuna_connessione() {
        let collega = || -> Result<i32, String> { panic!("collegato") };
        assert_eq!(
            avvia_terminale(|| Err("scaduto".into()), collega),
            Err("scaduto".into())
        );
        assert_eq!(avvia_terminale(|| Ok(()), || Ok(3)), Ok(3));
    }

    #[test]
    fn cartella_del_cli_diversa_da_quella_del_client() {
        let app = hbb_common::config::APP_NAME.read().unwrap().clone();
        assert_ne!(nome_cli(&app), app);
        // Il nome viene da APP_NAME: niente "Remotek" cablato.
        assert_eq!(nome_cli("Prova"), "Prova-cli");
        assert!(!nome_cli("Prova").contains("Remotek"));
        // Chiamata due volte, la cartella non cambia.
        assert_eq!(nome_cli(&nome_cli("Prova")), "Prova-cli");
    }

    /// Il test rilancia se stesso in un processo nuovo, con questa variabile e
    /// una HOME vuota: la configurazione in memoria non e' ancora caricata.
    #[cfg(target_os = "linux")]
    const FIGLIO: &str = "REMOTEK_CLI_TEST_FIGLIO";

    /// Solo Linux: la cartella segue HOME (su Windows la decide il sistema).
    #[cfg(target_os = "linux")]
    #[test]
    fn il_cli_usa_la_sua_cartella_dal_primo_accesso() {
        if std::env::var_os(FIGLIO).is_some() {
            return nel_figlio();
        }
        let casa = std::env::temp_dir().join(format!("remotek-cli-test-{}", std::process::id()));
        std::fs::create_dir_all(&casa).unwrap();
        let nome = module_path!().split_once("::").unwrap().1;
        let uscita = std::process::Command::new(std::env::current_exe().unwrap())
            .args([&format!(
                "{nome}::il_cli_usa_la_sua_cartella_dal_primo_accesso"
            )])
            .args(["--exact", "--nocapture", "--test-threads=1"])
            .env(FIGLIO, "1")
            .env("HOME", &casa)
            .env_remove("XDG_CONFIG_HOME")
            .output()
            .unwrap();
        std::fs::remove_dir_all(&casa).ok();
        let testo = String::from_utf8_lossy(&uscita.stdout);
        let errori = String::from_utf8_lossy(&uscita.stderr);
        assert!(uscita.status.success(), "{testo}\n{errori}");
        assert!(testo.contains("1 passed"), "{testo}");
    }

    /// Nel processo figlio: il client ha gia' account e ID; il CLI non li vede
    /// e scrive solo nella sua cartella.
    #[cfg(target_os = "linux")]
    fn nel_figlio() {
        use hbb_common::config::{Config, LocalConfig};
        let app = hbb_common::config::APP_NAME.read().unwrap().clone();
        // `Config::path` calcola il percorso senza caricare la configurazione.
        let client = Config::path("");
        std::fs::create_dir_all(&client).unwrap();
        let file_client = client.join(format!("{app}_local.toml"));
        let info = r#"{"name":"tecnico","display_name":"Agente AI per Mario"}"#;
        std::fs::write(
            &file_client,
            format!("[options]\naccess_token = 'token-grafico'\nuser_info = '{info}'\n"),
        )
        .unwrap();
        std::fs::write(client.join(format!("{app}.toml")), "id = '111222333'\n").unwrap();
        // Col codice di prima `whoami` leggeva l'account del client e usciva con 0.
        assert_eq!(
            avvia(["whoami"].map(std::ffi::OsString::from).into_iter()),
            USCITA_ERRORE
        );
        // Qui APP_NAME vale `<app>-cli`: i consigli all'agente nominano il
        // client sul PC, non il CLI (compito 61).
        let evento = terminale::evento_da_msgbox(
            "error",
            "Login Error",
            crate::remotek::token_terminale::RIFIUTO_NIENTE_AMMINISTRATORE,
            true,
        );
        let consiglio = match evento {
            Some(terminale::Evento::Errore(c)) => c,
            altro => panic!("{altro:?}"),
        };
        assert!(
            consiglio.contains(&format!("fai installare {app}")),
            "{consiglio}"
        );
        assert!(!consiglio.contains(SUFFISSO_CLI), "{consiglio}");
        let cli = Config::path("");
        assert_ne!(cli, client);
        assert!(cli
            .to_string_lossy()
            .to_lowercase()
            .contains(&nome_cli(&app).to_lowercase()));
        assert_eq!(LocalConfig::get_option("access_token"), "");
        assert_ne!(Config::get_id(), "111222333");
        LocalConfig::set_option("access_token".into(), "token-agente".into());
        let scritto = Config::path(format!("{}_local.toml", nome_cli(&app)));
        assert!(std::fs::read_to_string(scritto)
            .unwrap()
            .contains("token-agente"));
        let rimasto = std::fs::read_to_string(&file_client).unwrap();
        assert!(rimasto.contains("token-grafico") && !rimasto.contains("token-agente"));
    }

    #[test]
    fn argomenti_di_exec() {
        assert_eq!(
            leggi(&["exec", "123456789"]),
            Ok(Comando::Exec {
                id: "123456789".into(),
                attesa: 120,
                amministratore: false
            })
        );
        assert_eq!(
            leggi(&["exec", "123456789", "--amministratore", "--attesa", "30"]),
            Ok(Comando::Exec {
                id: "123456789".into(),
                attesa: 30,
                amministratore: true
            })
        );
    }

    #[test]
    fn nessuna_opzione_accetta_una_password() {
        for a in [
            &["login", "agente1", "segreta"][..],
            &["login", "agente1", "--password", "segreta"],
            &["login", "--password=segreta", "agente1"],
            &["login", "-p", "segreta"],
            &["--password", "segreta", "whoami"],
            &["whoami", "--password", "segreta"],
            &["logout", "segreta"],
            &["terminal", "123456789", "--password", "segreta"],
            &["terminal", "123456789", "segreta"],
            &["terminal", "--attesa", "10"],
            &["terminal", "123456789", "--attesa"],
            &["terminal", "123456789", "--attesa", "-1"],
            &["terminal", "123456789", "--righe", "65536"],
            &["terminal", "123456789", "--colonne", "4294967295"],
            &["terminal", "123456789", "--amministratore", "segreta"],
            &["terminal", "123456789", "--amministratore=segreta"],
            &["terminal", "123456789/r"],
            &["terminal", "123456789@server"],
            &["exec", "123456789", "--password", "segreta"],
            &["exec", "123456789", "segreta"],
            &["exec", "123456789", "--righe", "10"],
            &["exec", "123456789", "--colonne", "10"],
            &["exec", "123456789", "--amministratore=segreta"],
            &["exec", "123456789/r"],
            &["exec"],
            &[],
        ] {
            assert!(leggi(a).is_err(), "{a:?}");
        }
    }
}
