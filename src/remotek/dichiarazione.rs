//! Il PC dichiara al controllante le funzioni Remotek del terminale (ADR-0034).
//!
//! Ogni build manda la stessa `version` (1.4.9) nella `PeerInfo`, quindi il
//! controllante non puo' sapere dalla versione se il PC trascrive la sessione
//! terminale (ADR-0021 regola 5, src/remotek/trascrizione.rs) e la mostra al
//! cliente nel connection manager (regola 6, src/remotek/vista_cm.rs). Il PC
//! lo dichiara in `PeerInfo.platform_additions`, la mappa JSON che upstream
//! estende gia' con chiavi booleane sue (`is_installed`,
//! `support_view_camera`). Ogni chiave dichiara una funzione che il build ha:
//! un build senza quella funzione non deve mettere la chiave.
//!
//! `remotek-cli` non apre ne' terminale ne' exec su un PC che non le dichiara
//! tutte e due (src/remotek/cli/terminale.rs). Per exec vuole anche
//! [`CHIAVE_SCRIPT_NELLA_SCELTA`] (ADR-0035): il PC mostra lo script al
//! cliente nella finestra della richiesta, prima della scelta. Non apre il
//! tunnel su un PC che non dichiara [`CHIAVE_TUNNEL`] (ADR-0036,
//! src/remotek/cli/tunnel.rs), ne' verso un indirizzo diverso da `127.0.0.1`
//! su un PC che non dichiara [`CHIAVE_TUNNEL_DESTINAZIONI`] (ADR-0037).

use serde_json::{Map, Value};

/// Il PC trascrive ogni sessione terminale (ADR-0021 regola 5).
pub const CHIAVE_TRASCRIZIONE: &str = "remotek_trascrizione";
/// Il PC mostra la sessione terminale al cliente nel CM (ADR-0021 regola 6).
pub const CHIAVE_VISTA_CM: &str = "remotek_vista_cm";

/// Il PC mostra lo script di exec nella finestra della richiesta, prima
/// della scelta del cliente, ed esegue solo quello (ADR-0035).
pub const CHIAVE_SCRIPT_NELLA_SCELTA: &str = "remotek_script_nella_scelta";

/// Il PC serve il tunnel dell'agente coi flussi (src/remotek/tunnel.rs).
pub const CHIAVE_TUNNEL: &str = "remotek_tunnel";

/// Il PC apre il tunnel anche verso le destinazioni del `custom.txt` firmato
/// (src/remotek/destinazioni.rs, ADR-0037).
pub const CHIAVE_TUNNEL_DESTINAZIONI: &str = "remotek_tunnel_destinazioni";

/// Le chiavi che servono per aprire il terminale.
const CHIAVI_DEL_TERMINALE: [&str; 2] = [CHIAVE_TRASCRIZIONE, CHIAVE_VISTA_CM];

/// Aggiunge a `platform_additions` (src/server/connection.rs) le chiavi del
/// terminale, quella dello script nella scelta e quelle del tunnel, a `true`.
/// Le altre chiavi restano come sono.
pub fn aggiungi(m: &mut Map<String, Value>) {
    for chiave in CHIAVI_DEL_TERMINALE {
        m.insert(chiave.to_owned(), Value::Bool(true));
    }
    m.insert(CHIAVE_SCRIPT_NELLA_SCELTA.to_owned(), Value::Bool(true));
    m.insert(CHIAVE_TUNNEL.to_owned(), Value::Bool(true));
    m.insert(CHIAVE_TUNNEL_DESTINAZIONI.to_owned(), Value::Bool(true));
}

/// Vera solo se `platform_additions` e' una mappa JSON con
/// [`CHIAVE_TUNNEL_DESTINAZIONI`] al booleano `true`.
pub fn dichiara_le_destinazioni(platform_additions: &str) -> bool {
    serde_json::from_str::<Map<String, Value>>(platform_additions)
        .is_ok_and(|m| m.get(CHIAVE_TUNNEL_DESTINAZIONI) == Some(&Value::Bool(true)))
}

/// Vera solo se `platform_additions` e' una mappa JSON con [`CHIAVE_TUNNEL`]
/// al booleano `true`.
pub fn dichiara_il_tunnel(platform_additions: &str) -> bool {
    serde_json::from_str::<Map<String, Value>>(platform_additions)
        .is_ok_and(|m| m.get(CHIAVE_TUNNEL) == Some(&Value::Bool(true)))
}

/// Vera solo se `platform_additions` e' una mappa JSON con tutte e due le
/// chiavi al booleano `true`. Falsa con JSON non valido, testo vuoto, una
/// chiave mancante, `false` o un valore non booleano (anche `"true"`).
pub fn dichiara_il_terminale(platform_additions: &str) -> bool {
    match serde_json::from_str::<Map<String, Value>>(platform_additions) {
        Ok(m) => CHIAVI_DEL_TERMINALE
            .iter()
            .all(|chiave| m.get(*chiave) == Some(&Value::Bool(true))),
        Err(_) => false,
    }
}

/// Vera solo se `platform_additions` e' una mappa JSON con
/// [`CHIAVE_SCRIPT_NELLA_SCELTA`] al booleano `true`; falsa come
/// [`dichiara_il_terminale`] in ogni altro caso.
pub fn dichiara_lo_script_nella_scelta(platform_additions: &str) -> bool {
    match serde_json::from_str::<Map<String, Value>>(platform_additions) {
        Ok(m) => m.get(CHIAVE_SCRIPT_NELLA_SCELTA) == Some(&Value::Bool(true)),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn vera_solo_con_tutte_e_due_le_chiavi_a_true() {
        assert!(dichiara_il_terminale(
            r#"{"remotek_trascrizione":true,"remotek_vista_cm":true}"#
        ));
        assert!(!dichiara_il_terminale(r#"{"remotek_trascrizione":true}"#));
        assert!(!dichiara_il_terminale(r#"{"remotek_vista_cm":true}"#));
        assert!(!dichiara_il_terminale(r#"{"is_installed":true}"#));
        assert!(!dichiara_il_terminale("{}"));
        assert!(!dichiara_il_terminale(
            r#"{"remotek_trascrizione":true,"remotek_vista_cm":false}"#
        ));
        assert!(!dichiara_il_terminale(
            r#"{"remotek_trascrizione":"true","remotek_vista_cm":true}"#
        ));
        assert!(!dichiara_il_terminale(
            r#"{"remotek_trascrizione":true,"remotek_vista_cm":1}"#
        ));
        assert!(!dichiara_il_terminale(r#"{"remotek_trascrizione":true,"#));
        assert!(!dichiara_il_terminale(r#"["remotek_trascrizione"]"#));
        assert!(!dichiara_il_terminale(""));
    }

    #[test]
    fn aggiungi_tiene_le_altre_chiavi() {
        let mut platform_additions = Map::new();
        platform_additions.insert("is_installed".into(), json!(true));
        platform_additions.insert("support_view_camera".into(), json!(true));
        aggiungi(&mut platform_additions);
        assert_eq!(platform_additions.len(), 7);
        assert_eq!(platform_additions[CHIAVE_SCRIPT_NELLA_SCELTA], json!(true));
        assert_eq!(platform_additions["is_installed"], json!(true));
        assert_eq!(platform_additions["support_view_camera"], json!(true));
        // Serializzata come in connection.rs.
        let testo = serde_json::to_string(&platform_additions).unwrap_or("".into());
        assert!(dichiara_il_terminale(&testo));
        assert!(dichiara_lo_script_nella_scelta(&testo));
        assert!(dichiara_il_tunnel(&testo));
        assert!(dichiara_le_destinazioni(&testo));
    }

    #[test]
    fn il_tunnel_solo_con_la_sua_chiave_a_true() {
        assert!(dichiara_il_tunnel(r#"{"remotek_tunnel":true}"#));
        // Un PC del -8 dichiara il terminale ma non il tunnel.
        assert!(!dichiara_il_tunnel(
            r#"{"remotek_trascrizione":true,"remotek_vista_cm":true}"#
        ));
        assert!(!dichiara_il_tunnel(r#"{"remotek_tunnel":"true"}"#));
        assert!(!dichiara_il_tunnel(r#"{"remotek_tunnel":false}"#));
        assert!(!dichiara_il_tunnel(""));
    }

    #[test]
    fn le_destinazioni_solo_con_la_loro_chiave_a_true() {
        assert!(dichiara_le_destinazioni(
            r#"{"remotek_tunnel":true,"remotek_tunnel_destinazioni":true}"#
        ));
        // Il PC della #71: il tunnel, non le destinazioni.
        assert!(!dichiara_le_destinazioni(r#"{"remotek_tunnel":true}"#));
        assert!(!dichiara_le_destinazioni(
            r#"{"remotek_tunnel_destinazioni":"true"}"#
        ));
        assert!(!dichiara_le_destinazioni(
            r#"{"remotek_tunnel_destinazioni":false}"#
        ));
        assert!(!dichiara_le_destinazioni(""));
    }

    #[test]
    fn lo_script_nella_scelta_solo_col_booleano_true() {
        assert!(dichiara_lo_script_nella_scelta(
            r#"{"remotek_script_nella_scelta":true}"#
        ));
        // Il PC del -7 e del -8: le chiavi del terminale, non questa.
        let vecchio = r#"{"remotek_trascrizione":true,"remotek_vista_cm":true}"#;
        assert!(dichiara_il_terminale(vecchio));
        assert!(!dichiara_lo_script_nella_scelta(vecchio));
        for no in [
            r#"{"remotek_script_nella_scelta":false}"#,
            r#"{"remotek_script_nella_scelta":"true"}"#,
            r#"["remotek_script_nella_scelta"]"#,
            "",
        ] {
            assert!(!dichiara_lo_script_nella_scelta(no), "{no}");
        }
    }
}
