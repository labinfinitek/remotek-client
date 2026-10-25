//! Il PC verifica il lasciapassare dell'agente AI (ADR-0028).
//!
//! remotek-cli lo manda nel login del terminale, in `os_login.password` con
//! `os_login.username` vuoto (src/client.rs, `handle_hash`). Qui si toglie dal
//! login, prima che arrivi altrove, e si verifica come dice il README
//! dell'API ("Lasciapassare dell'agente"): firma Ed25519 della chiave
//! pubblica cablata sui byte del payload, prima di leggere il JSON; poi
//! versione, modo, PC, finestra di tempo con 30 secondi di tolleranza, nonce
//! mai visto. I nonce visti stanno in memoria, nel processo del server, fino a
//! `scade + 30`: un riavvio del servizio li dimentica.
//!
//! Un lasciapassare non valido non ferma la connessione: si prosegue come
//! senza (il terminale da utente, col clic del cliente).

use hbb_common::{
    base64::{
        engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
        Engine as _,
    },
    config::Config,
    log,
    message_proto::{login_request, LoginRequest},
    sodiumoxide::crypto::sign,
};
use serde_derive::Deserialize;
use serde_json::{json, Value};
use std::{collections::HashMap, fmt, sync::Mutex};

/// La chiave pubblica dell'API, in base64 come `KEY` del `custom.txt`
/// (src/common.rs). Vuota, ogni lasciapassare e' rifiutato.
pub const CHIAVE: &str = "Dv+D4Cwob7srUHd6Is+MZToqnKbrMt5vVFPqHEaB7sg=";

const CONTESTO: &[u8] = b"remotek-lasciapassare-v1.";
/// Secondi di tolleranza per gli orologi, prima di `emesso` e dopo `scade`.
const TOLLERANZA: i64 = 30;

/// I nonce gia' visti, con l'ora fino a cui si tengono.
static VISTI: Mutex<Option<HashMap<String, i64>>> = Mutex::new(None);

/// Un lasciapassare valido: chi l'ha chiesto e il nonce per l'audit.
#[derive(Debug, Clone, PartialEq)]
pub struct Valido {
    pub agente: String,
    pub tecnico: String,
    pub nonce: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Rifiuto {
    Spenta,
    Formato,
    Firma,
    VersioneOModo,
    AltroPc,
    NonAncoraValido,
    Scaduto,
    Riusato,
}

impl fmt::Display for Rifiuto {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            Rifiuto::Spenta => "funzione spenta",
            Rifiuto::Formato => "formato",
            Rifiuto::Firma => "firma",
            Rifiuto::VersioneOModo => "versione o modo",
            Rifiuto::AltroPc => "altro PC",
            Rifiuto::NonAncoraValido => "non ancora valido",
            Rifiuto::Scaduto => "scaduto",
            Rifiuto::Riusato => "riusato",
        })
    }
}

#[derive(Deserialize)]
struct Payload {
    v: i64,
    pc: String,
    agente: String,
    tecnico: String,
    modo: String,
    emesso: i64,
    scade: i64,
    nonce: String,
}

/// Toglie il lasciapassare dal login di un terminale: `os_login` con lo
/// username vuoto e la password no. Il modo con le credenziali di Windows
/// (username non vuoto) e gli altri tipi di connessione restano com'erano.
pub fn togli(lr: &mut LoginRequest) -> Option<String> {
    if !matches!(lr.union, Some(login_request::Union::Terminal(_))) {
        return None;
    }
    let os = lr.os_login.as_mut()?;
    if !os.username.is_empty() || os.password.is_empty() {
        return None;
    }
    Some(std::mem::take(&mut os.password))
}

/// Verifica `stringa` per il PC `pc` all'ora `adesso` (secondi unix) con la
/// chiave `chiave` (base64). `visti` sono i nonce gia' accettati, con l'ora
/// fino a cui tenerli.
pub fn verifica(
    stringa: &str,
    chiave: &str,
    pc: &str,
    adesso: i64,
    visti: &mut HashMap<String, i64>,
) -> Result<Valido, Rifiuto> {
    visti.retain(|_, fino| *fino >= adesso);
    let pk = STANDARD
        .decode(chiave)
        .ok()
        .and_then(|k| sign::PublicKey::from_slice(&k))
        .ok_or(Rifiuto::Spenta)?;
    let (payload, firma) = stringa.split_once('.').ok_or(Rifiuto::Formato)?;
    let payload = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| Rifiuto::Formato)?;
    let firma = URL_SAFE_NO_PAD
        .decode(firma)
        .ok()
        .and_then(|f| sign::Signature::from_bytes(&f).ok())
        .ok_or(Rifiuto::Formato)?;
    // La firma sui byte cosi' come sono arrivati, prima di leggere il JSON.
    let messaggio = [CONTESTO, payload.as_slice()].concat();
    if !sign::verify_detached(&firma, &messaggio, &pk) {
        return Err(Rifiuto::Firma);
    }
    let p: Payload = serde_json::from_slice(&payload).map_err(|_| Rifiuto::Formato)?;
    if p.v != 1 || p.modo != "amministratore" {
        return Err(Rifiuto::VersioneOModo);
    }
    if p.pc != pc {
        return Err(Rifiuto::AltroPc);
    }
    if adesso < p.emesso - TOLLERANZA {
        return Err(Rifiuto::NonAncoraValido);
    }
    if adesso > p.scade + TOLLERANZA {
        return Err(Rifiuto::Scaduto);
    }
    if p.nonce.is_empty() {
        return Err(Rifiuto::Formato);
    }
    if visti.contains_key(&p.nonce) {
        return Err(Rifiuto::Riusato);
    }
    visti.insert(p.nonce.clone(), p.scade + TOLLERANZA);
    Ok(Valido {
        agente: p.agente,
        tecnico: p.tecnico,
        nonce: p.nonce,
    })
}

/// [`verifica`] con la chiave cablata, l'ID di questo PC, l'ora e i nonce del
/// processo. Un rifiuto e' una riga di log con l'ID del controllante, mai la
/// stringa ne' il nonce.
pub fn verifica_login(stringa: &str, controllante: &str) -> Option<Valido> {
    let adesso = hbb_common::get_time() / 1000;
    let esito = match VISTI.lock() {
        Ok(mut visti) => {
            let visti = visti.get_or_insert_with(HashMap::new);
            verifica(stringa, CHIAVE, &Config::get_id(), adesso, visti)
        }
        Err(_) => Err(Rifiuto::Spenta),
    };
    match esito {
        Ok(v) => {
            log::info!(
                "lasciapassare valido: agente {}, tecnico {}, controllante {}",
                v.agente,
                v.tecnico,
                controllante
            );
            Some(v)
        }
        Err(e) => {
            log::warn!(
                "lasciapassare rifiutato: {} (controllante {})",
                e,
                controllante
            );
            None
        }
    }
}

/// I due campi facoltativi dell'audit della connessione (README dell'API,
/// "Collegamento alla connessione"): `lasciapassare`, il nonce del
/// lasciapassare valido; per il terminale `amministratore`, se la shell gira
/// con i permessi di amministratore. `sistema`: SYSTEM per la scelta del
/// cliente col lasciapassare (src/remotek/scelta_terminale.rs, `come_sistema`).
pub fn audit(
    audit: &mut Value,
    valido: Option<&Valido>,
    terminale: bool,
    os_username: &str,
    sistema: bool,
) {
    if let Some(v) = valido {
        audit["lasciapassare"] = json!(v.nonce);
    }
    if terminale {
        audit["amministratore"] = json!(sistema || amministratore(os_username));
    }
}

/// Senza la scelta del cliente, la shell gira da amministratore solo nel modo
/// `os_login` di upstream (nome e password di un amministratore di Windows) su
/// un PC installato: gira come il servizio. Altrimenti la shell da utente non
/// e' mai elevata (src/remotek/token_terminale.rs).
fn amministratore(os_username: &str) -> bool {
    #[cfg(target_os = "windows")]
    return !os_username.trim().is_empty() && crate::platform::is_installed();
    #[cfg(not(target_os = "windows"))]
    {
        let _ = os_username;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hbb_common::message_proto::{OSLogin, Terminal};

    // Il vettore del README dell'API (api-v0.8.0, "Lasciapassare dell'agente").
    const CHIAVE_PROVA: &str = "A6EHv/POEL4dcN0Y50vAmWfk1jCbpQ1fHdyGZBJVMbg=";
    const PC: &str = "999000111";
    const EMESSO: i64 = 1791000000;
    const VETTORE: &str = "eyJ2IjoxLCJwYyI6Ijk5OTAwMDExMSIsImFnZW50ZSI6ImFnZW50ZS1tYXJpbyIsInRlY25pY28iOiJtYXJpbyIsIm1vZG8iOiJhbW1pbmlzdHJhdG9yZSIsImVtZXNzbyI6MTc5MTAwMDAwMCwic2NhZGUiOjE3OTEwMDAwNjAsIm5vbmNlIjoiQUFFQ0F3UUZCZ2NJQ1FvTERBME9EdyJ9.BRkkm77ln4n8PxwF7Jmi3_-LiCHX7xGjLFjiXb3Knpc3kg9Rlv6DNoxForK3F28DLaUF5HWkudV4ZaWexK4YDA";

    fn prova(stringa: &str, chiave: &str, pc: &str, adesso: i64) -> Result<Valido, Rifiuto> {
        verifica(stringa, chiave, pc, adesso, &mut HashMap::new())
    }

    /// Un lasciapassare firmato col seme del vettore (0..31) su `payload`.
    fn firmato(payload: &str) -> String {
        let seme: [u8; 32] = std::array::from_fn(|i| i as u8);
        let (_, sk) = sign::keypair_from_seed(&sign::Seed(seme));
        let firma = sign::sign_detached(&[CONTESTO, payload.as_bytes()].concat(), &sk);
        format!(
            "{}.{}",
            URL_SAFE_NO_PAD.encode(payload),
            URL_SAFE_NO_PAD.encode(firma.to_bytes())
        )
    }

    #[test]
    fn vettore_del_readme_valido() {
        assert_eq!(
            prova(VETTORE, CHIAVE_PROVA, PC, EMESSO),
            Ok(Valido {
                agente: "agente-mario".into(),
                tecnico: "mario".into(),
                nonce: "AAECAwQFBgcICQoLDA0ODw".into(),
            })
        );
        // Lo stesso payload firmato qui da' la stessa stringa.
        let payload = String::from_utf8(
            URL_SAFE_NO_PAD
                .decode(VETTORE.split_once('.').unwrap().0)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(firmato(&payload), VETTORE);
    }

    #[test]
    fn altro_pc_e_finestra_di_tempo() {
        assert_eq!(
            prova(VETTORE, CHIAVE_PROVA, "999000112", EMESSO),
            Err(Rifiuto::AltroPc)
        );
        assert!(prova(VETTORE, CHIAVE_PROVA, PC, EMESSO + 60 + 30).is_ok());
        assert_eq!(
            prova(VETTORE, CHIAVE_PROVA, PC, EMESSO + 60 + 31),
            Err(Rifiuto::Scaduto)
        );
        assert!(prova(VETTORE, CHIAVE_PROVA, PC, EMESSO - 30).is_ok());
        assert_eq!(
            prova(VETTORE, CHIAVE_PROVA, PC, EMESSO - 31),
            Err(Rifiuto::NonAncoraValido)
        );
    }

    #[test]
    fn riusato_finche_non_scade() {
        let mut visti = HashMap::new();
        assert!(verifica(VETTORE, CHIAVE_PROVA, PC, EMESSO, &mut visti).is_ok());
        assert_eq!(
            verifica(VETTORE, CHIAVE_PROVA, PC, EMESSO + 1, &mut visti),
            Err(Rifiuto::Riusato)
        );
        // Dopo scade + 30 il nonce si toglie dalla memoria.
        verifica(VETTORE, CHIAVE_PROVA, PC, EMESSO + 91, &mut visti).unwrap_err();
        assert!(visti.is_empty());
    }

    #[test]
    fn un_byte_alterato_rompe_la_firma() {
        let (payload, firma) = VETTORE.split_once('.').unwrap();
        let altera = |s: &str| {
            let mut b = URL_SAFE_NO_PAD.decode(s).unwrap();
            b[10] ^= 1;
            URL_SAFE_NO_PAD.encode(b)
        };
        for rotto in [
            format!("{payload}.{}", altera(firma)),
            format!("{}.{firma}", altera(payload)),
        ] {
            assert_eq!(prova(&rotto, CHIAVE_PROVA, PC, EMESSO), Err(Rifiuto::Firma));
        }
        // Firmato bene ma con un'altra chiave pubblica: firma.
        assert_eq!(prova(VETTORE, CHIAVE, PC, EMESSO), Err(Rifiuto::Firma));
    }

    #[test]
    fn versione_modo_formato_e_chiave_vuota() {
        let base = r#"{"v":1,"pc":"999000111","agente":"a","tecnico":"t","modo":"amministratore","emesso":1791000000,"scade":1791000060,"nonce":"n"}"#;
        assert!(prova(&firmato(base), CHIAVE_PROVA, PC, EMESSO).is_ok());
        for (da, a) in [
            (r#""v":1"#, r#""v":2"#),
            (r#""modo":"amministratore""#, r#""modo":"utente""#),
        ] {
            let altro = firmato(&base.replace(da, a));
            assert_eq!(
                prova(&altro, CHIAVE_PROVA, PC, EMESSO),
                Err(Rifiuto::VersioneOModo)
            );
        }
        for rotto in [
            firmato("non json"),
            "senza-punto".into(),
            "a.b".into(),
            String::new(),
        ] {
            assert_eq!(
                prova(&rotto, CHIAVE_PROVA, PC, EMESSO),
                Err(Rifiuto::Formato),
                "{rotto}"
            );
        }
        assert_eq!(prova(VETTORE, "", PC, EMESSO), Err(Rifiuto::Spenta));
    }

    #[test]
    fn campi_dell_audit() {
        let valido = prova(VETTORE, CHIAVE_PROVA, PC, EMESSO).ok();
        let mut a = json!({"type": 0});
        audit(&mut a, valido.as_ref(), true, "", false);
        assert_eq!(
            a,
            json!({"type": 0, "lasciapassare": "AAECAwQFBgcICQoLDA0ODw", "amministratore": false})
        );
        // "Accetta come amministratore": la shell e' SYSTEM.
        let mut a = json!({"type": 0});
        audit(&mut a, valido.as_ref(), true, "", true);
        assert_eq!(a["amministratore"], json!(true));
        let mut a = json!({"type": 0});
        audit(&mut a, None, false, "", false);
        assert_eq!(a, json!({"type": 0}));
    }

    fn login(username: &str, password: &str, terminale: bool) -> LoginRequest {
        let mut lr = LoginRequest {
            os_login: Some(OSLogin {
                username: username.into(),
                password: password.into(),
                ..Default::default()
            })
            .into(),
            ..Default::default()
        };
        if terminale {
            lr.set_terminal(Terminal::new());
        }
        lr
    }

    #[test]
    fn il_login_non_tiene_il_lasciapassare() {
        let mut lr = login("", VETTORE, true);
        assert_eq!(togli(&mut lr).as_deref(), Some(VETTORE));
        assert_eq!(lr.os_login.password, "");
        // Le credenziali di Windows, un login senza password e le altre
        // connessioni restano come sono.
        for (u, p, t) in [
            ("Administrator", "pw", true),
            ("", "", true),
            ("", "pw", false),
        ] {
            let mut lr = login(u, p, t);
            assert_eq!(togli(&mut lr), None);
            assert_eq!(lr.os_login.password, p);
        }
    }
}
