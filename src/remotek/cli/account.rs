//! `login`, `logout`, `whoami`: l'account dell'agente sull'API Remotek.
//! Token e `user_info` stanno nelle opzioni locali come nel client Flutter
//! (flutter/lib/common/widgets/login.dart), e `display_name` e' il nome che
//! il cliente vede quando il CLI si collega (src/client.rs, `user_info`).

use hbb_common::config::{Config, LocalConfig};
use hbb_common::tls::TlsType;
use reqwest::Method;
use serde_json::{json, Value};
use std::time::Duration;

const PREFISSO_NOME: &str = "Agente AI per ";
const ACCESSO_SCADUTO: &str = "accesso scaduto o revocato: rifai `remotek-cli login`";

/// Stato e corpo di una risposta dell'API.
struct Risposta {
    status_code: u16,
    body: String,
}

fn api() -> Result<String, String> {
    let api = crate::common::get_api_server(
        Config::get_option("api-server"),
        Config::get_option("custom-rendezvous-server"),
    );
    if api.is_empty() {
        return Err("nessun server API configurato".to_owned());
    }
    Ok(api)
}

/// Un solo tentativo con il certificato sempre verificato: niente ripieghi di
/// `crate::http_request_sync` (certificati non validi, proxy TCP via hbbs).
/// Il messaggio d'errore non contiene mai corpo ne' intestazioni.
async fn richiesta(
    url: String,
    metodo: Method,
    corpo: Option<String>,
    token: Option<&str>,
) -> Result<Risposta, String> {
    let mut req = crate::hbbs_http::create_http_client_async(TlsType::Rustls, false)
        .request(metodo, url)
        .header("Content-Type", "application/json")
        .timeout(Duration::from_secs(30));
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    if let Some(c) = corpo {
        req = req.body(c);
    }
    let r = req
        .send()
        .await
        .map_err(|e| format!("API non raggiungibile: {e}"))?;
    let status_code = r.status().as_u16();
    let body = r
        .text()
        .await
        .map_err(|e| format!("risposta dell'API non leggibile: {e}"))?;
    Ok(Risposta { status_code, body })
}

fn errore_api(cosa: &str, r: &Risposta) -> String {
    let dettaglio = serde_json::from_str::<Value>(&r.body)
        .ok()
        .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_default();
    format!("{cosa}: HTTP {} {dettaglio}", r.status_code)
        .trim_end()
        .to_owned()
}

/// Il corpo di `POST /api/login` del client Flutter (hbbs.dart, `LoginRequest`).
fn corpo_login(utente: &str, password: &str, id: &str, uuid: &str, dispositivo: Value) -> Value {
    json!({
        "username": utente,
        "password": password,
        "id": id,
        "uuid": uuid,
        "autoLogin": true,
        "type": "account",
        "deviceInfo": dispositivo,
    })
}

/// Token e utente dalla risposta del login; le verifiche in piu' (email, 2FA)
/// il CLI non le gestisce.
fn esito_login(r: &Risposta) -> Result<(String, Value), String> {
    if r.status_code != 200 {
        return Err(errore_api("login rifiutato", r));
    }
    let v: Value = serde_json::from_str(&r.body)
        .map_err(|_| errore_api("login: risposta non leggibile", r))?;
    if let Some(e) = v.get("error").and_then(Value::as_str) {
        return Err(format!("login rifiutato: {e}"));
    }
    match (
        v.get("type").and_then(Value::as_str),
        v.get("access_token").and_then(Value::as_str),
    ) {
        (Some("access_token"), Some(token)) if !token.is_empty() => Ok((
            token.to_owned(),
            v.get("user").cloned().unwrap_or_else(|| json!({})),
        )),
        (Some(tipo), _) => Err(format!(
            "login: l'account chiede una verifica ({tipo}) che il CLI non gestisce"
        )),
        _ => Err("login: nessun token nella risposta".to_owned()),
    }
}

/// Il nome del tecnico da `GET /api/agente`; una persona non e' un agente.
fn esito_agente(r: &Risposta) -> Result<String, String> {
    if r.status_code != 200 {
        return Err(errore_api("verifica dell'agente non riuscita", r));
    }
    let v: Value = serde_json::from_str(&r.body)
        .map_err(|_| errore_api("verifica dell'agente: risposta non leggibile", r))?;
    match (
        v.get("agente").and_then(Value::as_bool),
        v.get("tecnico").and_then(Value::as_str),
    ) {
        (Some(true), Some(t)) if !t.trim().is_empty() => Ok(t.trim().to_owned()),
        (Some(false), _) => {
            Err("l'account non e' un agente AI: il CLI e' solo per gli agenti".to_owned())
        }
        _ => Err("verifica dell'agente: risposta senza tecnico".to_owned()),
    }
}

/// Perche' il token salvato non basta per aprire un terminale.
#[derive(Debug, PartialEq)]
enum Rifiuto {
    /// 401: token scaduto o revocato.
    Scaduto,
    Errore(String),
}

/// `POST /api/currentUser`: il token vale ancora.
fn esito_utente(r: &Risposta) -> Result<(), Rifiuto> {
    match r.status_code {
        401 => Err(Rifiuto::Scaduto),
        200 => match serde_json::from_str::<Value>(&r.body) {
            Ok(v) if v.get("error").is_none() => Ok(()),
            _ => Err(Rifiuto::Errore(errore_api(
                "verifica dell'accesso: risposta non valida",
                r,
            ))),
        },
        _ => Err(Rifiuto::Errore(errore_api(
            "verifica dell'accesso non riuscita",
            r,
        ))),
    }
}

/// `GET /api/agente`: ancora un agente, dello stesso tecnico del login.
fn esito_tecnico(r: &Risposta, salvato: &str) -> Result<(), Rifiuto> {
    if r.status_code == 401 {
        return Err(Rifiuto::Scaduto);
    }
    let tecnico = esito_agente(r).map_err(Rifiuto::Errore)?;
    if tecnico != salvato {
        return Err(Rifiuto::Errore(format!(
            "l'agente ora risponde a {tecnico}, non a {salvato}: rifai `remotek-cli login`"
        )));
    }
    Ok(())
}

fn user_info_agente(mut utente: Value, tecnico: &str) -> Value {
    if !utente.is_object() {
        utente = json!({});
    }
    utente["display_name"] = json!(format!("{PREFISSO_NOME}{tecnico}"));
    utente
}

/// Il corpo di `/api/logout` e `/api/currentUser` del client Flutter.
fn corpo_dispositivo() -> String {
    json!({ "id": Config::get_id(), "uuid": crate::encode64(hbb_common::get_uuid()) }).to_string()
}

async fn logout_api(api: &str, token: &str) -> Result<(), String> {
    let r = richiesta(
        format!("{api}/api/logout"),
        Method::POST,
        Some(corpo_dispositivo()),
        Some(token),
    )
    .await?;
    if r.status_code != 200 {
        return Err(errore_api("logout rifiutato", &r));
    }
    Ok(())
}

/// La password dell'account dell'agente, una riga da stdin: si legge prima di
/// entrare nel runtime, che non deve bloccarsi su stdin.
pub(super) fn leggi_password() -> Result<String, String> {
    let mut password = String::new();
    std::io::stdin()
        .read_line(&mut password)
        .map_err(|e| format!("stdin: {e}"))?;
    let password = password.trim_end_matches(['\r', '\n']);
    if password.is_empty() {
        return Err("password vuota: si legge da stdin, una riga".to_owned());
    }
    Ok(password.to_owned())
}

pub(super) async fn login(utente: &str, password: &str) -> Result<String, String> {
    let api = api()?;
    let dispositivo = serde_json::to_value(crate::ui_interface::get_login_device_info())
        .map_err(|e| format!("deviceInfo: {e}"))?;
    let corpo = corpo_login(
        utente,
        password,
        &Config::get_id(),
        &crate::encode64(hbb_common::get_uuid()),
        dispositivo,
    );
    let (token, utente_api) = esito_login(
        &richiesta(
            format!("{api}/api/login"),
            Method::POST,
            Some(corpo.to_string()),
            None,
        )
        .await?,
    )?;
    let agente = richiesta(format!("{api}/api/agente"), Method::GET, None, Some(&token)).await;
    let tecnico = match agente.and_then(|r| esito_agente(&r)) {
        Ok(t) => t,
        Err(e) => {
            // Il token appena avuto non serve: si chiude la sessione sull'API.
            if let Err(e2) = logout_api(&api, &token).await {
                eprintln!("remotek-cli: {e2}");
            }
            return Err(e);
        }
    };
    let salvato = salva("access_token", &token).and_then(|()| {
        salva(
            "user_info",
            &user_info_agente(utente_api, &tecnico).to_string(),
        )
    });
    if let Err(e) = salvato {
        // Senza credenziali salvate il token non serve: si chiude la sessione.
        if let Err(e2) = logout_api(&api, &token).await {
            eprintln!("remotek-cli: {e2}");
        }
        return Err(e);
    }
    Ok(format!("collegato come {utente}, {PREFISSO_NOME}{tecnico}"))
}

/// Scrive un'opzione locale e la rilegge dal file: `set_option` non restituisce
/// errori (hbb_common li scrive solo nel log, che il CLI non ha).
fn salva(chiave: &str, valore: &str) -> Result<(), String> {
    LocalConfig::set_option(chiave.to_owned(), valore.to_owned());
    if LocalConfig::get_option_from_file(chiave) != valore {
        return Err(format!("configurazione locale non scritta ({chiave})"));
    }
    Ok(())
}

/// Prima di ogni `terminal`: il token salvato vale ancora ed e' di un agente
/// dello stesso tecnico. Un errore di rete o dell'API ferma il terminale; un
/// 401 toglie anche le credenziali locali.
pub(super) async fn verifica_accesso() -> Result<(), String> {
    verifica_accesso_su(&api()?).await
}

async fn verifica_accesso_su(api: &str) -> Result<(), String> {
    let token = LocalConfig::get_option("access_token");
    let tecnico = salvati(&LocalConfig::get_option("user_info"))
        .map(|(_, tecnico)| tecnico)
        .ok_or_else(|| "user_info non leggibile: remotek-cli login <utente>".to_owned())?;
    match controlla(api, &token, &tecnico).await {
        Ok(()) => Ok(()),
        Err(Rifiuto::Errore(e)) => Err(e),
        Err(Rifiuto::Scaduto) => {
            match salva("access_token", "").and_then(|()| salva("user_info", "")) {
                Ok(()) => Err(ACCESSO_SCADUTO.to_owned()),
                Err(e) => Err(format!("{ACCESSO_SCADUTO}; {e}")),
            }
        }
    }
}

async fn controlla(api: &str, token: &str, tecnico: &str) -> Result<(), Rifiuto> {
    let utente = richiesta(
        format!("{api}/api/currentUser"),
        Method::POST,
        Some(corpo_dispositivo()),
        Some(token),
    )
    .await
    .map_err(Rifiuto::Errore)?;
    esito_utente(&utente)?;
    let agente = richiesta(format!("{api}/api/agente"), Method::GET, None, Some(token))
        .await
        .map_err(Rifiuto::Errore)?;
    esito_tecnico(&agente, tecnico)
}

pub(super) async fn logout() -> Result<String, String> {
    let token = LocalConfig::get_option("access_token");
    if token.is_empty() {
        return Ok("non collegato".to_owned());
    }
    let esito = match api() {
        Ok(api) => logout_api(&api, &token).await,
        Err(e) => Err(e),
    };
    // Le credenziali locali si tolgono comunque.
    let tolte = salva("access_token", "").and_then(|()| salva("user_info", ""));
    match (esito, tolte) {
        (Err(a), Err(b)) => Err(format!("{a}; {b}")),
        (esito, tolte) => esito.and(tolte).map(|()| "scollegato".to_owned()),
    }
}

pub(super) fn whoami() -> Result<String, String> {
    if LocalConfig::get_option("access_token").is_empty() {
        return Err("non collegato: remotek-cli login <utente>".to_owned());
    }
    descrivi(&Config::get_id(), &LocalConfig::get_option("user_info"))
}

fn descrivi(id: &str, user_info: &str) -> Result<String, String> {
    salvati(user_info)
        .map(|(utente, tecnico)| format!("ID: {id}\nutente: {utente}\ntecnico: {tecnico}"))
        .ok_or_else(|| "user_info non leggibile: remotek-cli login <utente>".to_owned())
}

/// Utente e tecnico salvati al login in `user_info`.
fn salvati(user_info: &str) -> Option<(String, String)> {
    let info: Value = serde_json::from_str(user_info).ok()?;
    let campo = |k: &str| info.get(k).and_then(Value::as_str).unwrap_or_default();
    match (
        campo("name"),
        campo("display_name").strip_prefix(PREFISSO_NOME),
    ) {
        (utente, Some(tecnico)) if !utente.is_empty() && !tecnico.is_empty() => {
            Some((utente.to_owned(), tecnico.to_owned()))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn risposta(status_code: u16, body: &str) -> Risposta {
        Risposta {
            status_code,
            body: body.to_owned(),
        }
    }

    #[test]
    fn corpo_del_login_come_flutter() {
        let c = corpo_login(
            "agente1",
            "pw",
            "123456789",
            "dXVpZA",
            json!({"os": "linux", "type": "client", "name": "h"}),
        );
        assert_eq!(
            c,
            json!({"username": "agente1", "password": "pw", "id": "123456789", "uuid": "dXVpZA",
                   "autoLogin": true, "type": "account",
                   "deviceInfo": {"os": "linux", "type": "client", "name": "h"}})
        );
    }

    #[test]
    fn login_token_verifiche_ed_errori() {
        let ok = risposta(
            200,
            r#"{"type":"access_token","access_token":"t","user":{"name":"agente1"}}"#,
        );
        assert_eq!(
            esito_login(&ok),
            Ok(("t".to_owned(), json!({"name": "agente1"})))
        );
        assert!(esito_login(&risposta(200, r#"{"type":"tfa_check","tfa_type":"totp"}"#)).is_err());
        assert!(esito_login(&risposta(200, r#"{"error":"Wrong credentials"}"#)).is_err());
        let e = esito_login(&risposta(401, r#"{"error":"Wrong credentials"}"#)).unwrap_err();
        assert_eq!(e, "login rifiutato: HTTP 401 Wrong credentials");
    }

    #[test]
    fn agente_persona_ed_errore() {
        assert_eq!(
            esito_agente(&risposta(200, r#"{"agente":true,"tecnico":"Mario Rossi"}"#)),
            Ok("Mario Rossi".to_owned())
        );
        let persona = esito_agente(&risposta(200, r#"{"agente":false}"#)).unwrap_err();
        assert!(persona.contains("non e' un agente"), "{persona}");
        assert!(esito_agente(&risposta(200, r#"{"agente":true,"tecnico":""}"#)).is_err());
        assert!(esito_agente(&risposta(401, r#"{"error":"Invalid token"}"#)).is_err());
        assert!(esito_agente(&risposta(500, "<html>")).is_err());
    }

    #[test]
    fn nome_che_vede_il_cliente() {
        let u = user_info_agente(
            json!({"name": "agente1", "display_name": "x"}),
            "Mario Rossi",
        );
        assert_eq!(
            u,
            json!({"name": "agente1", "display_name": "Agente AI per Mario Rossi"})
        );
        assert_eq!(
            user_info_agente(Value::Null, "M")["display_name"],
            "Agente AI per M"
        );
    }

    #[test]
    fn accesso_utente_e_tecnico() {
        assert_eq!(
            esito_utente(&risposta(200, r#"{"name":"agente1"}"#)),
            Ok(())
        );
        assert_eq!(esito_utente(&risposta(401, "")), Err(Rifiuto::Scaduto));
        for rotta in [risposta(500, "<html>"), risposta(200, r#"{"error":"x"}"#)] {
            assert!(matches!(esito_utente(&rotta), Err(Rifiuto::Errore(_))));
        }
        let agente = |c: &str| esito_tecnico(&risposta(200, c), "Mario");
        assert_eq!(agente(r#"{"agente":true,"tecnico":"Mario"}"#), Ok(()));
        for rifiutato in [
            r#"{"agente":false,"tecnico":""}"#,
            r#"{"agente":true,"tecnico":"Luca"}"#,
        ] {
            assert!(
                matches!(agente(rifiutato), Err(Rifiuto::Errore(_))),
                "{rifiutato}"
            );
        }
        assert_eq!(
            esito_tecnico(&risposta(401, ""), "Mario"),
            Err(Rifiuto::Scaduto)
        );
    }

    /// Un'API finta su 127.0.0.1: per ogni rotta lo stato e il corpo dati, e
    /// l'elenco delle rotte chieste.
    async fn api_finta(
        risposte: &'static [(&'static str, u16, &'static str)],
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use hbb_common::tokio::io::{AsyncReadExt, AsyncWriteExt};
        let ascolto = hbb_common::tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap();
        let url = format!("http://{}", ascolto.local_addr().unwrap());
        let chieste = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let segnate = chieste.clone();
        hbb_common::tokio::spawn(async move {
            while let Ok((mut s, _)) = ascolto.accept().await {
                // Tutta la richiesta, intestazioni e corpo, prima di rispondere.
                let mut letta = Vec::new();
                let mut buf = [0u8; 4096];
                loop {
                    let testo = String::from_utf8_lossy(&letta).to_lowercase();
                    if let Some(fine) = testo.find("\r\n\r\n") {
                        let lunghezza = testo
                            .lines()
                            .find_map(|r| r.strip_prefix("content-length:"))
                            .and_then(|n| n.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        if letta.len() >= fine + 4 + lunghezza {
                            break;
                        }
                    }
                    match s.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => letta.extend_from_slice(&buf[..n]),
                    }
                }
                let testo = String::from_utf8_lossy(&letta).into_owned();
                let rotta = testo.split_whitespace().nth(1).unwrap_or("").to_owned();
                let (stato, corpo) = risposte
                    .iter()
                    .find(|(r, _, _)| *r == rotta)
                    .map_or((404, ""), |(_, s, c)| (*s, *c));
                segnate.lock().unwrap().push(rotta);
                let r = format!(
                    "HTTP/1.1 {stato} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{corpo}",
                    corpo.len()
                );
                s.write_all(r.as_bytes()).await.unwrap();
            }
        });
        (url, chieste)
    }

    #[test]
    fn accesso_scaduto_toglie_le_credenziali() {
        let _ambiente = super::super::AMBIENTE.lock().unwrap();
        let opzione = |k: &str| LocalConfig::get_option(k);
        let prima = (opzione("access_token"), opzione("user_info"));
        LocalConfig::set_option("access_token".into(), "t".into());
        let info = r#"{"name":"agente1","display_name":"Agente AI per Mario"}"#;
        LocalConfig::set_option("user_info".into(), info.into());
        let rt = hbb_common::tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (esito, chieste) = rt.block_on(async {
            let (url, chieste) =
                api_finta(&[("/api/currentUser", 401, r#"{"error":"Invalid token"}"#)]).await;
            (verifica_accesso_su(&url).await, chieste)
        });
        let dopo = (opzione("access_token"), opzione("user_info"));
        LocalConfig::set_option("access_token".into(), prima.0);
        LocalConfig::set_option("user_info".into(), prima.1);
        assert_eq!(esito, Err(ACCESSO_SCADUTO.to_owned()));
        // Niente /api/agente dopo il 401, e le credenziali locali sono tolte.
        assert_eq!(*chieste.lock().unwrap(), ["/api/currentUser"]);
        assert_eq!(dopo, (String::new(), String::new()));
    }

    #[test]
    fn whoami_legge_user_info_o_da_errore() {
        let info = r#"{"name":"agente1","display_name":"Agente AI per Mario Rossi"}"#;
        assert_eq!(
            descrivi("123", info),
            Ok("ID: 123\nutente: agente1\ntecnico: Mario Rossi".to_owned())
        );
        for rotto in ["", "{", r#"{"name":"agente1"}"#, r#"{"display_name":"x"}"#] {
            assert!(descrivi("123", rotto).is_err(), "{rotto}");
        }
    }
}
