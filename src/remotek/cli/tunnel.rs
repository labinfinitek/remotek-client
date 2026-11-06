//! `tunnel <ID> <locale>:<porta> ...` (ADR-0036): una connessione di port
//! forward verso il PC, con piu' flussi dentro (src/remotek/tunnel.rs). Ogni
//! connessione TCP accettata su `127.0.0.1:<locale>` di questa macchina e' un
//! flusso verso `<porta>` di `127.0.0.1` del PC, o, con
//! `<locale>:<indirizzo>:<porta>`, verso un'altra macchina vista dal PC, che
//! il PC raggiunge solo se il suo `custom.txt` firmato la elenca (ADR-0037).
//! Si entra se il cliente accetta, col lasciapassare di tunnel da utente, o
//! senza clic dove il `custom.txt` firmato lo lascia entrare.

use super::terminale::{sessione, Evento, Gestore};
use crate::client::{Client, Interface};
use crate::remotek::{
    destinazioni,
    tunnel::{
        ciclo, Apertura, Chiusura, FineDalPc, Motivo, Resoconto, DURATA_MASSIMA, DURATA_MINIMA,
        MASSIMO_PORTE,
    },
};
use hbb_common::{
    config::{LocalConfig, PeerConfig},
    message_proto::{login_response, message, Message},
    protobuf::Message as _,
    rendezvous_proto::ConnType,
    tokio::{self, net::TcpListener, sync::mpsc},
    Stream,
};
use std::{
    io::Write,
    net::{Ipv4Addr, SocketAddrV4},
    sync::mpsc::{channel, Receiver},
    time::Duration,
};

/// Una porta del tunnel: `locale` su `127.0.0.1` di questa macchina,
/// `remota` di `indirizzo` visto dal PC (`127.0.0.1` o un'altra macchina,
/// ADR-0037).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Porta {
    pub locale: u16,
    pub indirizzo: Ipv4Addr,
    pub remota: u16,
}

/// Le porte della riga di comando: `<locale>:<porta>` verso `127.0.0.1` del
/// PC, `<locale>:<indirizzo>:<porta>` verso un'altra macchina vista dal PC.
/// L'indirizzo come nella chiave firmata (`destinazioni::indirizzo`: IPv4 in
/// forma canonica, niente nomi; `127.x` si scrive con la forma corta). Porte
/// da 1 a 65535, locali diverse e coppie indirizzo e porta diverse; da 1 a
/// [`MASSIMO_PORTE`] porte del lasciapassare ([`porte_del_lasciapassare`]).
pub(super) fn leggi_porte(parole: &[&str]) -> Result<Vec<Porta>, String> {
    let numero = |s: &str| s.parse::<u16>().ok().filter(|p| *p != 0);
    let sintassi =
        "porta non valida: <locale>:<porta> o <locale>:<indirizzo>:<porta>, porte da 1 a 65535";
    let mut porte: Vec<Porta> = Vec::new();
    for parola in parole {
        let (locale, indirizzo, remota) = match parola.split(':').collect::<Vec<_>>()[..] {
            [l, r] => (l, Some(Ipv4Addr::LOCALHOST), r),
            [l, i, r] => (l, destinazioni::indirizzo(i), r),
            _ => return Err(sintassi.to_owned()),
        };
        let (Some(locale), Some(remota)) = (numero(locale), numero(remota)) else {
            return Err(sintassi.to_owned());
        };
        let indirizzo = indirizzo.ok_or_else(|| {
            format!(
                "indirizzo non valido in {parola}: IPv4 in forma canonica, niente nomi, \
                 ne' 127.x (si scrive <locale>:<porta>), 0.x, 255.255.255.255 o multicast"
            )
        })?;
        let porta = Porta {
            locale,
            indirizzo,
            remota,
        };
        if porte.iter().any(|p| {
            p.locale == porta.locale || (p.indirizzo, p.remota) == (porta.indirizzo, porta.remota)
        }) {
            return Err("porta ripetuta".to_owned());
        }
        porte.push(porta);
    }
    if porte.is_empty() || porte_del_lasciapassare(&porte).len() > MASSIMO_PORTE {
        return Err(format!("da 1 a {MASSIMO_PORTE} porte del PC"));
    }
    Ok(porte)
}

/// Le porte del lasciapassare: le porte remote senza ripetizioni, in ordine.
pub(super) fn porte_del_lasciapassare(porte: &[Porta]) -> Vec<u16> {
    let mut remote: Vec<u16> = Vec::new();
    for p in porte {
        if !remote.contains(&p.remota) {
            remote.push(p.remota);
        }
    }
    remote
}

/// La riga di comando chiede un indirizzo diverso da `127.0.0.1`.
pub(super) fn verso_altre_macchine(porte: &[Porta]) -> bool {
    porte.iter().any(|p| p.indirizzo != Ipv4Addr::LOCALHOST)
}

/// `--durata`: da [`DURATA_MINIMA`] a [`DURATA_MASSIMA`] secondi.
pub(super) fn leggi_durata(s: &str) -> Result<u32, String> {
    s.parse::<u32>()
        .ok()
        .filter(|d| (DURATA_MINIMA..=DURATA_MASSIMA).contains(d))
        .ok_or_else(|| format!("durata da {DURATA_MINIMA} a {DURATA_MASSIMA} secondi"))
}

/// Il lasciapassare e' di tunnel per le porte chieste: restituisce la durata
/// firmata. Si legge solo il payload; la firma la verifica il PC.
pub(super) fn durata_del_lasciapassare(lasciapassare: &str, porte: &[u16]) -> Option<u32> {
    use hbb_common::base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    let p: serde_json::Value = lasciapassare
        .split_once('.')
        .and_then(|(p, _)| URL_SAFE_NO_PAD.decode(p).ok())
        .and_then(|p| serde_json::from_slice(&p).ok())?;
    let firmate: Vec<u16> = serde_json::from_value(p["porte"].clone()).ok()?;
    let stesse = firmate.len() == porte.len() && porte.iter().all(|x| firmate.contains(x));
    (p["v"] == 2 && p["capacita"] == "tunnel" && stesse)
        .then(|| p["durata"].as_u64())
        .flatten()
        .and_then(|d| u32::try_from(d).ok())
}

/// Il PC dichiara il tunnel (`dichiarazione::dichiara_il_tunnel`) e, se la
/// riga di comando chiede altre macchine (`esterne`), le destinazioni
/// (`dichiarazione::dichiara_le_destinazioni`); se no lo scrive su `avvisi`
/// (stderr) e da' `USCITA_NON_DICHIARA`.
pub(super) fn controlla_dichiarazione(
    platform_additions: &str,
    esterne: bool,
    avvisi: &mut impl Write,
) -> Result<(), i32> {
    use crate::remotek::dichiarazione::{dichiara_il_tunnel, dichiara_le_destinazioni};
    let manca = if !dichiara_il_tunnel(platform_additions) {
        "il tunnel"
    } else if esterne && !dichiara_le_destinazioni(platform_additions) {
        "il tunnel verso altre macchine"
    } else {
        return Ok(());
    };
    writeln!(
        avvisi,
        "remotek-cli: il PC non dichiara {manca}: aggiornare {} sul PC",
        super::marchio()
    )
    .and_then(|()| avvisi.flush())
    .ok();
    Err(super::USCITA_NON_DICHIARA)
}

/// Il codice d'uscita alla fine del tunnel: 0 se l'ha chiuso l'agente o se
/// e' finita la durata (qui o sul PC), 1 altrimenti.
pub(super) fn codice_alla_fine(motivo: &Motivo) -> i32 {
    match motivo {
        Motivo::Chiusura(Chiusura::Agente | Chiusura::Durata)
        | Motivo::DalPc(FineDalPc::Durata) => 0,
        _ => super::USCITA_ERRORE,
    }
}

/// Apre il tunnel verso il PC `id`. `chiedi` chiede all'API il lasciapassare
/// per le porte remote e la durata, dopo i controlli locali (porte locali
/// libere, sessione senza password).
pub(super) fn tunnel(
    id: &str,
    porte: &[Porta],
    attesa: u64,
    chiedi: impl FnOnce(&[u16]) -> Result<String, String>,
) -> Result<i32, String> {
    // Le porte locali si prendono subito: occupate, niente lasciapassare.
    let mut ascolti = Vec::new();
    for p in porte {
        let a = std::net::TcpListener::bind(("127.0.0.1", p.locale))
            .and_then(|a| a.set_nonblocking(true).map(|()| a))
            .map_err(|e| format!("porta locale {}: {e}", p.locale))?;
        ascolti.push((a, *p));
    }
    let (tx, rx) = channel();
    let session = sessione(id, Gestore::con_eventi(tx), ConnType::PORT_FORWARD)?;
    let remote = porte_del_lasciapassare(porte);
    let lasciapassare = chiedi(&remote)?;
    let durata = durata_del_lasciapassare(&lasciapassare, &remote)
        .ok_or("l'API non ha dato un lasciapassare di tunnel per queste porte")?;
    let id = {
        let mut lc = session
            .lc
            .write()
            .map_err(|_| "sessione non disponibile".to_owned())?;
        lc.remotek_lasciapassare = lasciapassare;
        // Host vuoto e porta 0: il PC usa i flussi, un PC vecchio rifiuta.
        lc.port_forward = (String::new(), 0);
        lc.get_id().to_owned()
    };
    let esito = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("runtime: {e}"))
        .and_then(|r| {
            r.block_on(collega_e_servi(
                &session,
                &rx,
                ascolti,
                Duration::from_secs(attesa),
                Duration::from_secs(durata.into()),
            ))
        });
    // Come per il terminale, il CLI non tiene la configurazione del PC.
    PeerConfig::remove(&id);
    esito
}

/// Il login (fino alla `PeerInfo`, entro `attesa`), poi i flussi.
async fn collega_e_servi(
    session: &crate::ui_session_interface::Session<Gestore>,
    eventi: &Receiver<Evento>,
    ascolti: Vec<(std::net::TcpListener, Porta)>,
    attesa: Duration,
    durata: Duration,
) -> Result<i32, String> {
    let token = LocalConfig::get_option("access_token");
    let key = crate::get_key(false).await;
    let accesso = async {
        let ((mut stream, _, _, _, _), _) = Client::start(
            &session.get_id(),
            &key,
            &token,
            ConnType::PORT_FORWARD,
            session.clone(),
        )
        .await
        .map_err(|e| e.to_string())?;
        if !stream.is_secured() {
            return Err("connessione non cifrata da un capo all'altro: rifiutata".to_owned());
        }
        login(session, eventi, &mut stream)
            .await
            .map(|pa| (stream, pa))
    };
    let (mut stream, platform_additions) = tokio::time::timeout(attesa, accesso)
        .await
        .map_err(|_| format!("nessuna risposta entro {} secondi", attesa.as_secs()))??;
    let porte: Vec<Porta> = ascolti.iter().map(|(_, p)| *p).collect();
    let esterne = verso_altre_macchine(&porte);
    if let Err(codice) =
        controlla_dichiarazione(&platform_additions, esterne, &mut std::io::stderr())
    {
        return Ok(codice);
    }
    let (tx, rx) = mpsc::unbounded_channel();
    for (a, p) in ascolti {
        let a = TcpListener::from_std(a).map_err(|e| format!("porta locale {}: {e}", p.locale))?;
        eprintln!("remotek-cli: {}", riga_in_ascolto(&p));
        let tx = tx.clone();
        let destinazione = SocketAddrV4::new(p.indirizzo, p.remota);
        tokio::spawn(async move {
            while let Ok((s, _)) = a.accept().await {
                if tx.send((destinazione, s)).is_err() {
                    return;
                }
            }
        });
    }
    let r = ciclo(&mut stream, Apertura::Controllante(rx), fine(durata), None).await;
    eprintln!("remotek-cli: {}", riga_finale(&r));
    Ok(codice_alla_fine(&r.motivo))
}

/// La riga su stderr per ogni porta in ascolto.
pub(super) fn riga_in_ascolto(p: &Porta) -> String {
    format!(
        "tunnel in ascolto su 127.0.0.1:{} verso {}:{} dal PC",
        p.locale, p.indirizzo, p.remota
    )
}

/// La riga su stderr alla chiusura: motivo, flussi, byte nei due versi.
pub(super) fn riga_finale(r: &Resoconto) -> String {
    format!(
        "tunnel chiuso ({}): {} flussi, {} byte mandati al PC, {} ricevuti",
        r.motivo, r.flussi, r.byte_mandati, r.byte_ricevuti
    )
}

/// Il tunnel finisce con SIGINT o SIGTERM (Ctrl+C su Windows) o alla durata.
async fn fine(durata: Duration) -> Chiusura {
    #[cfg(unix)]
    let termina = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending().await,
        }
    };
    #[cfg(not(unix))]
    let termina = std::future::pending::<()>();
    tokio::select! {
        _ = tokio::time::sleep(durata) => Chiusura::Durata,
        Ok(()) = tokio::signal::ctrl_c() => Chiusura::Agente,
        _ = termina => Chiusura::Agente,
    }
}

/// I messaggi del login: l'hash, la risposta e i `TestDelay`. Restituisce le
/// `platform_additions` della `PeerInfo`; i rifiuti passano dai messaggi del
/// client (`evento_da_msgbox`), come per il terminale.
async fn login(
    session: &crate::ui_session_interface::Session<Gestore>,
    eventi: &Receiver<Evento>,
    stream: &mut Stream,
) -> Result<String, String> {
    let mut avvisato = false;
    loop {
        let bytes = match stream.next().await {
            Some(Ok(b)) => b,
            Some(Err(e)) => return Err(format!("connessione chiusa: {e}")),
            None => return Err("connessione chiusa dal PC".to_owned()),
        };
        let msg = Message::parse_from_bytes(&bytes).map_err(|e| e.to_string())?;
        let (mut pa, mut rifiuto) = (None, None);
        match msg.union {
            Some(message::Union::Hash(hash)) => session.handle_hash("", hash, stream).await,
            Some(message::Union::LoginResponse(lr)) => match lr.union {
                // L'attesa del clic o il motivo arrivano anche come eventi
                // (`evento_da_msgbox`); un errore senza evento resta un rifiuto.
                Some(login_response::Union::Error(err)) => {
                    if !session.handle_login_error(&err) {
                        rifiuto = Some(err);
                    }
                }
                Some(login_response::Union::PeerInfo(pi)) => pa = Some(pi.platform_additions),
                _ => {}
            },
            Some(message::Union::TestDelay(t)) => session.handle_test_delay(t, stream).await,
            _ => {}
        }
        while let Ok(evento) = eventi.try_recv() {
            match evento {
                Evento::Attesa if !avvisato => {
                    eprintln!("remotek-cli: in attesa che il cliente accetti");
                    avvisato = true;
                }
                Evento::Errore(e) => return Err(e),
                _ => {}
            }
        }
        if let Some(e) = rifiuto {
            return Err(e);
        }
        if let Some(pa) = pa {
            return Ok(pa);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn porta(locale: u16, remota: u16) -> Porta {
        Porta {
            locale,
            indirizzo: Ipv4Addr::LOCALHOST,
            remota,
        }
    }

    fn verso(locale: u16, indirizzo: &str, remota: u16) -> Porta {
        Porta {
            locale,
            indirizzo: indirizzo.parse().unwrap(),
            remota,
        }
    }

    /// `<locale>:<indirizzo>:<porta>` (ADR-0037): l'indirizzo con le regole
    /// della chiave firmata; locali diverse, coppie diverse; le porte del
    /// lasciapassare sono le remote senza ripetizioni.
    #[test]
    fn la_forma_lunga() {
        assert_eq!(
            leggi_porte(&["15433:10.0.0.5:1433"]),
            Ok(vec![verso(15433, "10.0.0.5", 1433)])
        );
        let porte =
            leggi_porte(&["1433:1433", "15433:10.0.0.5:1433", "18080:10.0.0.6:8080"]).unwrap();
        assert_eq!(
            porte,
            vec![
                porta(1433, 1433),
                verso(15433, "10.0.0.5", 1433),
                verso(18080, "10.0.0.6", 8080)
            ]
        );
        assert_eq!(porte_del_lasciapassare(&porte), vec![1433, 8080]);
        assert!(verso_altre_macchine(&porte));
        assert!(!verso_altre_macchine(&[porta(1433, 1433)]));
        assert_eq!(
            riga_in_ascolto(&porte[1]),
            "tunnel in ascolto su 127.0.0.1:15433 verso 10.0.0.5:1433 dal PC"
        );
        assert_eq!(
            riga_in_ascolto(&porte[0]),
            "tunnel in ascolto su 127.0.0.1:1433 verso 127.0.0.1:1433 dal PC"
        );
        // Indirizzi vietati, nomi, IPv6, zeri davanti.
        for vietato in [
            "15433:127.0.0.1:1433",
            "15433:127.0.0.2:1433",
            "15433:0.0.0.0:1433",
            "15433:0.1.2.3:1433",
            "15433:255.255.255.255:1433",
            "15433:224.0.0.1:1433",
            "15433:010.0.0.5:1433",
            "15433:sql.cliente.local:1433",
            "15433:localhost:1433",
            "15433:[::1]:1433",
            "15433:::1:1433",
            "15433::1433",
            "15433:10.0.0.5:0",
            "15433:10.0.0.5:65536",
            "0:10.0.0.5:1433",
            "15433:10.0.0.5:1433:1",
        ] {
            assert!(leggi_porte(&[vietato]).is_err(), "{vietato}");
        }
        // Ripetizioni: la stessa locale, la stessa coppia.
        assert!(leggi_porte(&["15433:10.0.0.5:1433", "15433:10.0.0.6:1433"]).is_err());
        assert!(leggi_porte(&["15433:10.0.0.5:1433", "25433:10.0.0.5:1433"]).is_err());
        // La stessa porta su macchine diverse: una porta del lasciapassare.
        let stessa =
            leggi_porte(&["1433:1433", "15433:10.0.0.5:1433", "25433:10.0.0.6:1433"]).unwrap();
        assert_eq!(porte_del_lasciapassare(&stessa), vec![1433]);
        // Al massimo 8 porte del lasciapassare, non 8 righe.
        let nove: Vec<String> = (1..=9)
            .map(|i| format!("{}:10.0.0.{}:{}", 2000 + i, i, 3000 + i))
            .collect();
        let nove: Vec<&str> = nove.iter().map(String::as_str).collect();
        assert!(leggi_porte(&nove[..8]).is_ok());
        assert!(leggi_porte(&nove).is_err());
        let dieci: Vec<String> = (1..=10)
            .map(|i| format!("{}:10.0.0.{}:1433", 2000 + i, i))
            .collect();
        let dieci: Vec<&str> = dieci.iter().map(String::as_str).collect();
        assert_eq!(
            porte_del_lasciapassare(&leggi_porte(&dieci).unwrap()),
            vec![1433]
        );
    }

    #[test]
    fn le_porte_della_riga_di_comando() {
        assert_eq!(leggi_porte(&["1433:1433"]), Ok(vec![porta(1433, 1433)]));
        assert_eq!(
            leggi_porte(&["15433:1433", "18080:8080"]),
            Ok(vec![porta(15433, 1433), porta(18080, 8080)])
        );
        for sbagliate in [
            &["0:1433"][..],
            &["1433:0"],
            &["65536:1433"],
            &["1433:65536"],
            &["1433"],
            &["a:1433"],
            &["1433:1433:1"],
            &["-1:1433"],
            &[],
        ] {
            assert!(leggi_porte(sbagliate).is_err(), "{sbagliate:?}");
        }
        // Ripetute, nel locale o nel remoto.
        assert!(leggi_porte(&["1433:1433", "1433:8080"]).is_err());
        assert!(leggi_porte(&["15433:1433", "25433:1433"]).is_err());
        // Piu' di 8.
        let nove: Vec<String> = (1..=9)
            .map(|i| format!("{}:{}", 2000 + i, 3000 + i))
            .collect();
        let nove: Vec<&str> = nove.iter().map(String::as_str).collect();
        assert!(leggi_porte(&nove[..8]).is_ok());
        assert!(leggi_porte(&nove).is_err());
    }

    #[test]
    fn la_durata() {
        assert_eq!(leggi_durata("60"), Ok(60));
        assert_eq!(leggi_durata("28800"), Ok(28800));
        for d in ["59", "28801", "0", "-60", "un'ora", ""] {
            assert!(leggi_durata(d).is_err(), "{d}");
        }
    }

    fn lasciapassare(payload: &str) -> String {
        use hbb_common::base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
        format!("{}.firma", URL_SAFE_NO_PAD.encode(payload))
    }

    #[test]
    fn il_lasciapassare_di_tunnel_con_le_porte_chieste() {
        let l = lasciapassare(r#"{"v":2,"capacita":"tunnel","porte":[1433,8080],"durata":3600}"#);
        assert_eq!(durata_del_lasciapassare(&l, &[8080, 1433]), Some(3600));
        assert_eq!(durata_del_lasciapassare(&l, &[1433]), None);
        assert_eq!(durata_del_lasciapassare(&l, &[1433, 8080, 22]), None);
        let exec = lasciapassare(r#"{"v":2,"capacita":"exec","porte":[],"durata":0}"#);
        assert_eq!(durata_del_lasciapassare(&exec, &[]), None);
        assert_eq!(durata_del_lasciapassare("non.base64!", &[1433]), None);
    }

    #[test]
    fn senza_dichiarazione_126() {
        let mut avvisi = Vec::new();
        assert_eq!(
            controlla_dichiarazione(
                r#"{"remotek_trascrizione":true,"remotek_vista_cm":true}"#,
                false,
                &mut avvisi
            ),
            Err(super::super::USCITA_NON_DICHIARA)
        );
        assert_eq!(super::super::USCITA_NON_DICHIARA, 126);
        let testo = String::from_utf8(avvisi).unwrap();
        assert!(testo.contains("non dichiara il tunnel"), "{testo}");
        let mut avvisi = Vec::new();
        assert_eq!(
            controlla_dichiarazione(r#"{"remotek_tunnel":true}"#, false, &mut avvisi),
            Ok(())
        );
        assert!(avvisi.is_empty());
        // Il PC della #71: 126 solo se la riga di comando chiede altre
        // macchine.
        let mut avvisi = Vec::new();
        assert_eq!(
            controlla_dichiarazione(r#"{"remotek_tunnel":true}"#, true, &mut avvisi),
            Err(126)
        );
        let testo = String::from_utf8(avvisi).unwrap();
        assert!(
            testo.contains("non dichiara il tunnel verso altre macchine"),
            "{testo}"
        );
        let nuovo = r#"{"remotek_tunnel":true,"remotek_tunnel_destinazioni":true}"#;
        for esterne in [false, true] {
            let mut avvisi = Vec::new();
            assert_eq!(controlla_dichiarazione(nuovo, esterne, &mut avvisi), Ok(()));
            assert!(avvisi.is_empty());
        }
        // Senza il tunnel, 126 anche con la forma corta.
        let mut avvisi = Vec::new();
        assert_eq!(
            controlla_dichiarazione(
                r#"{"remotek_tunnel_destinazioni":true}"#,
                false,
                &mut avvisi
            ),
            Err(126)
        );
    }

    #[test]
    fn codici_alla_fine() {
        assert_eq!(codice_alla_fine(&Motivo::Chiusura(Chiusura::Agente)), 0);
        assert_eq!(codice_alla_fine(&Motivo::Chiusura(Chiusura::Durata)), 0);
        assert_eq!(codice_alla_fine(&Motivo::DalPc(FineDalPc::Durata)), 0);
        for m in [
            Motivo::DalPc(FineDalPc::Cliente),
            Motivo::ChiusaDallAltroCapo,
            Motivo::Rete("x".into()),
            Motivo::Protocollo("x"),
        ] {
            assert_eq!(codice_alla_fine(&m), 1, "{m:?}");
        }
    }
}
