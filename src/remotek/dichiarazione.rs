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
//! tutte e due (src/remotek/cli/terminale.rs).

use serde_json::{Map, Value};

/// Il PC trascrive ogni sessione terminale (ADR-0021 regola 5).
pub const CHIAVE_TRASCRIZIONE: &str = "remotek_trascrizione";
/// Il PC mostra la sessione terminale al cliente nel CM (ADR-0021 regola 6).
pub const CHIAVE_VISTA_CM: &str = "remotek_vista_cm";

/// Le chiavi che servono per aprire il terminale.
const CHIAVI_DEL_TERMINALE: [&str; 2] = [CHIAVE_TRASCRIZIONE, CHIAVE_VISTA_CM];

/// Aggiunge a `platform_additions` (src/server/connection.rs) le due chiavi,
/// a `true`. Le altre chiavi restano come sono.
pub fn aggiungi(m: &mut Map<String, Value>) {
    for chiave in CHIAVI_DEL_TERMINALE {
        m.insert(chiave.to_owned(), Value::Bool(true));
    }
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
        assert_eq!(platform_additions.len(), 4);
        assert_eq!(platform_additions["is_installed"], json!(true));
        assert_eq!(platform_additions["support_view_camera"], json!(true));
        // Serializzata come in connection.rs.
        let testo = serde_json::to_string(&platform_additions).unwrap_or("".into());
        assert!(dichiara_il_terminale(&testo));
    }
}
