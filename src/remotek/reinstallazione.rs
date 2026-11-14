//! Il `custom.txt` resta quando si aggiorna dalla finestra.
//!
//! Collaudo del `remotek-1.4.9-10` (2026-11-08, osservazione 1): con
//! "Aggiorna" della home il `custom.txt` della cartella di installazione
//! spariva e il server non presidiato tornava a chiedere il clic. Il pulsante
//! porta alla pagina di installazione (`update_me` di `ui_interface.rs` e'
//! `goto_install`), e `install_me` comincia con la disinstallazione, che fa
//! `rd /s /q` della cartella. Lo staging di upstream
//! (`handle_custom_client_staging_dir_before_update`) vale solo per
//! `update_to`, cioe' l'aggiornamento automatico, che Remotek non fa.
//!
//! Qui due pezzi dello script di `install_me`: prima della disinstallazione
//! il `custom.txt` della vecchia cartella va accanto a lei
//! (`<cartella>.custom.txt`, stessi permessi del padre: `C:\Program Files`
//! per l'installazione normale), dopo la copia del programma torna nella
//! cartella nuova. Si porta solo il file che c'era: una copia rimasta da un
//! giro precedente si cancella prima. Il file resta firmato e il servizio lo
//! verifica come sempre (`read_custom_client`).

/// Righe da mettere prima della disinstallazione. `vecchia` e' la cartella
/// dell'installazione presente (`get_install_info`); vuota, niente.
pub fn salva(vecchia: &str) -> String {
    let Some(vecchia) = cartella(vecchia) else {
        return String::new();
    };
    let copia = copia(vecchia);
    format!(
        "
chcp 65001
if exist \"{copia}\" del /f /q \"{copia}\"
if exist \"{vecchia}\\custom.txt\" copy /y \"{vecchia}\\custom.txt\" \"{copia}\"
"
    )
}

/// Righe da mettere dopo la copia del programma nella cartella `nuova`.
pub fn rimetti(vecchia: &str, nuova: &str) -> String {
    let (Some(vecchia), Some(nuova)) = (cartella(vecchia), cartella(nuova)) else {
        return String::new();
    };
    let copia = copia(vecchia);
    format!(
        "
if exist \"{copia}\" move /y \"{copia}\" \"{nuova}\\custom.txt\"
"
    )
}

fn cartella(percorso: &str) -> Option<&str> {
    let p = percorso.trim().trim_end_matches('\\');
    // Un percorso con le virgolette romperebbe lo script: meglio niente.
    if p.is_empty() || p.contains('"') {
        None
    } else {
        Some(p)
    }
}

fn copia(vecchia: &str) -> String {
    format!("{vecchia}.custom.txt")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn salva_cancella_la_copia_vecchia_e_poi_copia() {
        let s = salva("C:\\Program Files\\Remotek\\");
        let del = s
            .find("del /f /q \"C:\\Program Files\\Remotek.custom.txt\"")
            .unwrap();
        let cp = s
            .find("copy /y \"C:\\Program Files\\Remotek\\custom.txt\" \"C:\\Program Files\\Remotek.custom.txt\"")
            .unwrap();
        assert!(del < cp);
    }

    #[test]
    fn rimetti_porta_nella_cartella_nuova() {
        let s = rimetti("C:\\Program Files\\Remotek", "D:\\Programmi\\Remotek\\");
        assert!(s.contains(
            "if exist \"C:\\Program Files\\Remotek.custom.txt\" move /y \"C:\\Program Files\\Remotek.custom.txt\" \"D:\\Programmi\\Remotek\\custom.txt\""
        ));
    }

    #[test]
    fn senza_cartella_niente() {
        assert_eq!(salva(""), "");
        assert_eq!(salva("  \\ "), "");
        assert_eq!(rimetti("", "C:\\Program Files\\Remotek"), "");
        assert_eq!(rimetti("C:\\Program Files\\Remotek", ""), "");
    }

    #[test]
    fn virgolette_niente() {
        assert_eq!(salva("C:\\a\" & del x"), "");
        assert_eq!(rimetti("C:\\a", "C:\\b\""), "");
    }
}
