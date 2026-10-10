//! Firma delle richieste del PC all'API (ADR-0023). Il contratto e' il README
//! di remotek-api, sezione "Firma del dispositivo": l'uuid che lega l'ID al PC
//! va a hbbs in chiaro, quindi il PC firma sysinfo, heartbeat e audit con la
//! sua chiave Ed25519, la stessa di `RegisterPk`.

use hbb_common::{bail, config::Config, log, sodiumoxide::crypto::sign, ResultType};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const VERSIONE: &str = "remotek-api-v1";
const INTESTAZIONE: &str = "X-Remotek-Firma";

/// L'intestazione di firma di una POST a `url` con questo `corpo`, nel formato
/// "Nome: valore" che `crate::post_request` passa alla richiesta. `corpo`
/// dev'essere esattamente la stringa mandata.
///
/// La firma Ed25519 e' deterministica: due richieste identiche nello stesso
/// secondo hanno la stessa firma e l'API rifiuta la seconda come ripetuta.
/// L'heartbeat parte ogni 3 o 15 secondi con corpi diversi; gli audit dei
/// file, che possono ripetersi uguali (lo stesso file mandato due volte),
/// portano il campo `ms` di [`ora_ms`]. Un PC con l'orologio sbagliato di
/// piu' di 5 minuti e' rifiutato.
///
/// Se la firma non si puo' fare (chiave del PC illeggibile, orologio prima del
/// 1970) lo scrive nel log e restituisce "": la richiesta parte senza firma e
/// decide l'API, che la rifiuta se il PC ha una chiave registrata.
pub fn intestazione(url: &str, corpo: &str) -> String {
    match firma_ora(url, corpo) {
        Ok(valore) => riga(&valore),
        Err(e) => {
            log::error!(
                "firma della richiesta a {} non riuscita: {}",
                percorso(url),
                e
            );
            String::new()
        }
    }
}

/// I millisecondi UNIX, strettamente crescenti nel processo: il campo `ms`
/// degli audit dei file, che li distingue anche nello stesso millisecondo.
/// L'API ignora i campi che non conosce. Con l'orologio prima del 1970 parte
/// da 0 e cresce lo stesso.
pub fn ora_ms() -> u64 {
    static ULTIMA: AtomicU64 = AtomicU64::new(0);
    let ora = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    let prossima = |ultima: u64| ora.max(ultima.saturating_add(1));
    // La chiusura restituisce sempre Some: Ok e Err portano lo stesso valore.
    match ULTIMA.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |u| Some(prossima(u))) {
        Ok(ultima) | Err(ultima) => prossima(ultima),
    }
}

/// "X-Remotek-Firma: <valore>", la riga che `crate::post_request` divide su ": ".
fn riga(valore: &str) -> String {
    format!("{}: {}", INTESTAZIONE, valore)
}

/// La chiave pubblica Ed25519 del PC in base64 standard, il campo `pk` del
/// sysinfo.
pub fn chiave_pubblica() -> String {
    crate::encode64(Config::get_key_pair().1)
}

fn firma_ora(url: &str, corpo: &str) -> ResultType<String> {
    let (sk, _) = Config::get_key_pair();
    let Some(sk) = sign::SecretKey::from_slice(&sk) else {
        bail!(
            "chiave del PC di {} byte invece di {}",
            sk.len(),
            sign::SECRETKEYBYTES
        );
    };
    let ts = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    Ok(valore("POST", percorso(url), ts, corpo.as_bytes(), &sk))
}

/// `<ts>.<firma>`: firma in base64 standard del messaggio di cinque righe del
/// contratto.
fn valore(metodo: &str, percorso: &str, ts: u64, corpo: &[u8], sk: &sign::SecretKey) -> String {
    let messaggio = format!(
        "{}\n{}\n{}\n{}\n{}",
        VERSIONE,
        metodo.to_uppercase(),
        percorso,
        ts,
        hex::encode(Sha256::digest(corpo))
    );
    let firma = sign::sign_detached(messaggio.as_bytes(), sk);
    format!("{}.{}", ts, crate::encode64(firma.to_bytes()))
}

/// Il percorso dell'URL, senza schema, host, query e frammento. Con un'API
/// dietro un prefisso (`https://host/prefisso`) il prefisso resta nel
/// percorso firmato, mentre il README vuole il percorso che arriva all'API:
/// dietro un reverse proxy che toglie il prefisso la firma non torna e l'API
/// rifiuta le richieste del PC. L'API va servita senza prefisso, o con un
/// proxy che lo lascia.
fn percorso(url: &str) -> &str {
    let dopo_host = match url.find("://") {
        Some(i) => &url[i + 3..],
        None => url,
    };
    let p = match dopo_host.find(|c| c == '/' || c == '?' || c == '#') {
        Some(inizio) if dopo_host[inizio..].starts_with('/') => &dopo_host[inizio..],
        _ => return "/",
    };
    match p.find(|c| c == '?' || c == '#') {
        Some(fine) => &p[..fine],
        None => p,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chiavi_di_prova() -> (sign::PublicKey, sign::SecretKey) {
        let mut seme = [0u8; 32];
        for (i, b) in seme.iter_mut().enumerate() {
            *b = i as u8;
        }
        sign::keypair_from_seed(&sign::Seed(seme))
    }

    #[test]
    fn vettore_del_readme() {
        let (pk, sk) = chiavi_di_prova();
        assert_eq!(
            crate::encode64(pk.0),
            "A6EHv/POEL4dcN0Y50vAmWfk1jCbpQ1fHdyGZBJVMbg="
        );
        let corpo = r#"{"id":"999000111","uuid":"dXVpZA=="}"#;
        assert_eq!(
            hex::encode(Sha256::digest(corpo.as_bytes())),
            "8ce852e69dbc62f20c29f8fcfb77e1a1922d73c7a185de173bb31139f4089d10"
        );
        let v = valore("POST", "/api/heartbeat", 1791000000, corpo.as_bytes(), &sk);
        assert_eq!(
            v,
            "1791000000.otUjpW4BdAQnl0TuN1E/GmD3LAx38zzpYTczckv70y3hYSIVtkA7urCTqftTR9BXPMoDc//+nyxItL73nbrHCw=="
        );
        // La riga intera del README: nome e separatore compresi.
        assert_eq!(
            riga(&v),
            "X-Remotek-Firma: 1791000000.otUjpW4BdAQnl0TuN1E/GmD3LAx38zzpYTczckv70y3hYSIVtkA7urCTqftTR9BXPMoDc//+nyxItL73nbrHCw=="
        );
    }

    #[test]
    fn forma_dell_intestazione() {
        let (_, sk) = chiavi_di_prova();
        let v = valore("post", "/api/sysinfo", 1791000001, b"{}", &sk);
        let (ts, firma) = v.split_once('.').unwrap();
        assert_eq!(ts, "1791000001");
        #[allow(deprecated)]
        let byte = hbb_common::base64::decode(firma).unwrap();
        assert_eq!(byte.len(), sign::SIGNATUREBYTES);
        // post_request divide l'intestazione su ": " e la usa solo se le parti sono due.
        assert_eq!(riga(&v).split(": ").count(), 2);
    }

    #[test]
    fn audit_dei_file_uguali_nello_stesso_secondo_firme_diverse() {
        let (_, sk) = chiavi_di_prova();
        let corpo = || {
            serde_json::json!({"id": "999000111", "path": "C:\\a.txt", "ms": ora_ms()}).to_string()
        };
        let (a, b) = (corpo(), corpo());
        assert_ne!(a, b);
        let firma = |c: &str| valore("POST", "/api/audit/file", 1791000000, c.as_bytes(), &sk);
        assert_ne!(firma(&a), firma(&b));
        let prima = ora_ms();
        assert!(ora_ms() > prima);
    }

    #[test]
    fn percorso_dall_url() {
        assert_eq!(
            percorso("https://api.example.it/api/heartbeat"),
            "/api/heartbeat"
        );
        assert_eq!(
            percorso("https://api.example.it:21114/api/sysinfo?a=1&b=2"),
            "/api/sysinfo"
        );
        assert_eq!(percorso("http://h/api/audit/conn#x"), "/api/audit/conn");
        assert_eq!(
            percorso("https://h/pre/api/audit/file"),
            "/pre/api/audit/file"
        );
        assert_eq!(percorso("https://h"), "/");
        assert_eq!(percorso("https://h?q=1/2"), "/");
    }
}
