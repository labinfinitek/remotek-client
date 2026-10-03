//! `remotek-cli`: il client a riga di comando con cui l'agente AI di Infinitek
//! usa il terminale dei PC dei clienti (ADR-0021). Entra solo se il cliente
//! accetta la finestra: nessuna password del PC, in nessun modo.

mod account;

use std::io::Write;

const USO: &str = "uso:
  remotek-cli login <utente>   la password dell'account dell'agente si legge da stdin, una riga
  remotek-cli logout
  remotek-cli whoami";

/// Uscita per un errore dell'API, di rete o di configurazione.
pub(crate) const USCITA_ERRORE: i32 = 1;
/// Uscita per argomenti sbagliati.
pub(crate) const USCITA_USO: i32 = 2;

#[derive(Debug, PartialEq)]
enum Comando {
    Login(String),
    Logout,
    Whoami,
    Aiuto,
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
        [] => Err("manca il comando".to_owned()),
        // Gli argomenti non si ripetono: potrebbero contenere una password.
        _ => Err("argomenti non validi".to_owned()),
    }
}

/// Punto d'ingresso del binario: restituisce il codice d'uscita.
pub fn main() -> i32 {
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
        Ok(Comando::Login(utente)) => account::login(&utente),
        Ok(Comando::Logout) => account::logout(),
        Ok(Comando::Whoami) => account::whoami(),
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
            &[],
        ] {
            assert!(leggi(a).is_err(), "{a:?}");
        }
    }
}
