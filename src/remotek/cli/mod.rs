//! `remotek-cli`: il client a riga di comando con cui l'agente AI di Infinitek
//! usa il terminale dei PC dei clienti (ADR-0021). Entra solo se il cliente
//! accetta la finestra: nessuna password del PC, in nessun modo.

mod account;
mod terminale;

use std::io::Write;

const USO: &str = "uso:
  remotek-cli login <utente>   la password dell'account dell'agente si legge da stdin, una riga
  remotek-cli logout
  remotek-cli whoami
  remotek-cli terminal <ID> [--attesa <secondi>] [--righe <n>] [--colonne <n>]
                               stdin va al terminale del PC, il terminale su stdout;
                               si entra solo se il cliente accetta (attesa di default 120 s);
                               righe e colonne da 1 a 65535 (default 24 e 80);
                               esce col codice del terminale (fuori da 0-255: 255); 0 anche
                               quando il PC non conosce il codice della shell;
                               la fine di stdin chiude subito il terminale e la shell: per il
                               codice della shell si manda `exit` e si tiene stdin aperto";

/// Uscita per un errore dell'API, di rete o di configurazione.
pub(crate) const USCITA_ERRORE: i32 = 1;
/// Uscita per argomenti sbagliati.
pub(crate) const USCITA_USO: i32 = 2;

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
    },
    Aiuto,
}

/// `terminal <ID>` con le sole tre opzioni numeriche; nessuna porta una password.
fn leggi_terminale(id: &str, opzioni: &[&str]) -> Result<Comando, String> {
    // Solo l'ID: niente `/r`, `@server` o altro che il client interpreta.
    let valido = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    if id.is_empty() || id.starts_with('-') || !id.chars().all(valido) {
        return Err("ID del PC non valido".to_owned());
    }
    let (mut attesa, mut righe, mut colonne) = (120, 24, 80);
    let mut resto = opzioni.chunks(2);
    while let Some(coppia) = resto.next() {
        let valore = |v: Option<&&str>| v.and_then(|v| v.parse::<u32>().ok()).filter(|n| *n > 0);
        // Il PC converte righe e colonne in u16 (src/server/terminal_service.rs).
        let dimensione = |n: u32| n <= u32::from(u16::MAX);
        match (coppia.first(), valore(coppia.get(1))) {
            (Some(&"--attesa"), Some(n)) => attesa = n,
            (Some(&"--righe"), Some(n)) if dimensione(n) => righe = n,
            (Some(&"--colonne"), Some(n)) if dimensione(n) => colonne = n,
            _ => return Err("argomenti non validi".to_owned()),
        }
    }
    Ok(Comando::Terminale {
        id: id.to_owned(),
        attesa: attesa.into(),
        righe,
        colonne,
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
        [] => Err("manca il comando".to_owned()),
        // Gli argomenti non si ripetono: potrebbero contenere una password.
        _ => Err("argomenti non validi".to_owned()),
    }
}

/// L'unico runtime del CLI, per le richieste all'API. Il terminale non lo usa:
/// `io_loop` gira nel suo thread come in src/flutter.rs (`session_start_`).
fn nel_runtime<F: std::future::Future<Output = Result<String, String>>>(
    futuro: F,
) -> Result<String, String> {
    hbb_common::tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("runtime: {e}"))?
        .block_on(futuro)
}

/// Punto d'ingresso del binario: restituisce il codice d'uscita.
pub fn main() -> i32 {
    // ADR-0021, regola 3: mai il terminale come amministratore.
    std::env::remove_var("IS_TERMINAL_ADMIN");
    // `args()` andrebbe in panic, stampandolo, su un argomento non UTF-8.
    let args: Result<Vec<String>, _> = std::env::args_os()
        .skip(1)
        .map(|a| a.into_string())
        .collect();
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
        }) => {
            // Il cliente deve vedere "Agente AI per <tecnico>", non l'utente Linux.
            let esito =
                account::whoami().and_then(|_| terminale::terminale(&id, attesa, righe, colonne));
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
            &["terminal", "123456789/r"],
            &["terminal", "123456789@server"],
            &[],
        ] {
            assert!(leggi(a).is_err(), "{a:?}");
        }
    }
}
