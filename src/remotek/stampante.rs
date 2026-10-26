//! La stampante remota di upstream, tolta da Remotek.
//!
//! Collaudo di `remotek-1.4.9-4` e `-5` sulla 109, caso 26 (server non
//! presidiato): col `custom.txt` firmato il servizio `--server` usciva dopo
//! circa 150 ms col codice 3 e ripartiva in ciclo, e l'uscita veniva da un
//! thread di `printer_driver_adapter.dll`. Quella DLL e' un binario gia'
//! compilato che il build scaricava dalle release di rustdesk/hbb_common, non
//! dai nostri sorgenti: non la possiamo correggere ne' verificare. Il PC non
//! la carica mai, anche se il file e' rimasto nella cartella di installazione
//! (XCOPY non cancella i file della versione precedente), e nessuna
//! installazione o aggiornamento mette piu' la stampante.

/// Il motivo, nel log del servizio al posto dell'avvio della stampante.
pub const MOTIVO: &str = "stampante remota spenta: printer_driver_adapter.dll non e' nostro \
     e faceva uscire --server (collaudo, caso 26)";

/// Se il servizio della stampante parte (`src/server.rs`, `new`): mai. E'
/// lui che carica `printer_driver_adapter.dll` (`printer_service::init`).
pub fn servizio() -> bool {
    false
}

/// Se l'installazione mette la stampante, chiunque la chieda (pagina di
/// installazione, `--silent-install`, registro a 1): mai. Senza, su Windows
/// 10 e seguenti `install_me` lancia `--uninstall-remote-printer`, che toglie
/// quella installata da una versione precedente.
pub fn installa(_richiesta: bool) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn il_servizio_non_parte() {
        assert!(!servizio());
    }

    #[test]
    fn nessuna_installazione_mette_la_stampante() {
        assert!(!installa(true));
        assert!(!installa(false));
    }
}
