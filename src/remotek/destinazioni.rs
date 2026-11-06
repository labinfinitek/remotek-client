//! Le destinazioni del tunnel dell'agente diverse da `127.0.0.1` (ADR-0037).
//!
//! Il tunnel (src/remotek/tunnel.rs, ADR-0036) porta al PC del cliente; una
//! macchina diversa da `127.0.0.1` della sua rete si raggiunge solo se la
//! chiave `remotek-agente-destinazioni` del `custom.txt` firmato di quel PC la
//! elenca, una voce per indirizzo e porta. Vale col clic e senza clic: e' la
//! configurazione firmata del server a dire dove si puo' andare, non chi
//! chiede. La porta deve stare anche fra quelle del lasciapassare
//! ([`della_sessione`]). La chiave si legge solo da `HARD_SETTINGS`, dove
//! `read_custom_client` (src/common.rs) mette le chiavi del `custom.txt`
//! firmato che non conosce.

use super::lasciapassare::{Capacita, Valido};
use hbb_common::{config::HARD_SETTINGS, log};
use std::{
    collections::{BTreeSet, HashMap},
    net::{Ipv4Addr, SocketAddrV4},
};

/// La chiave del `custom.txt` firmato, su una riga: lo strumento di firma la
/// legge cosi'.
pub const CHIAVE_DESTINAZIONI: &str = "remotek-agente-destinazioni";

/// `remotek-agente-destinazioni` nelle impostazioni fisse.
#[derive(Debug, Clone, PartialEq)]
pub enum Destinazioni {
    /// Chiave assente: il tunnel va solo a `127.0.0.1`.
    Assente,
    /// Le voci elencate, ciascuna una volta.
    Elenco(BTreeSet<SocketAddrV4>),
    /// Vuota, voce ripetuta o non valida, spazi: vale come assente, con la
    /// riga ERROR di [`segnala_malformate`].
    Malformato,
}

/// L'indirizzo di una destinazione: un IPv4 letterale in forma canonica (il
/// testo e' quello di `to_string()`: niente zeri davanti, niente nomi,
/// niente IPv6), fuori da `127.0.0.0/8` (ci pensa ADR-0036), `0.0.0.0/8`,
/// `255.255.255.255` e dal multicast. Lo usa anche la riga di comando di
/// `remotek-cli tunnel`.
pub fn indirizzo(testo: &str) -> Option<Ipv4Addr> {
    let ip: Ipv4Addr = testo.parse().ok()?;
    let vietato = ip.is_loopback() || ip.octets()[0] == 0 || ip.is_broadcast() || ip.is_multicast();
    (ip.to_string() == testo && !vietato).then_some(ip)
}

/// La porta di una destinazione: solo cifre, niente zeri davanti, da 1 a
/// 65535, come `non_presidiato::porte_da`.
fn porta(testo: &str) -> Option<u16> {
    let cifre = !testo.is_empty() && testo.bytes().all(|b| b.is_ascii_digit());
    let p = testo.parse::<u16>().ok()?;
    (cifre && p != 0 && !testo.starts_with('0')).then_some(p)
}

/// Legge `remotek-agente-destinazioni` dalle impostazioni fisse: voci
/// `indirizzo:porta` separate da `,`, senza spazi, almeno una, ciascuna una
/// volta.
pub fn destinazioni_da(fisse: &HashMap<String, String>) -> Destinazioni {
    let Some(valore) = fisse.get(CHIAVE_DESTINAZIONI) else {
        return Destinazioni::Assente;
    };
    let mut voci = BTreeSet::new();
    for voce in valore.split(',') {
        let letta = voce
            .split_once(':')
            .and_then(|(i, p)| Some(SocketAddrV4::new(indirizzo(i)?, porta(p)?)));
        match letta {
            // `insert` falso: la voce e' ripetuta.
            Some(d) if voci.insert(d) => {}
            _ => return Destinazioni::Malformato,
        }
    }
    Destinazioni::Elenco(voci)
}

/// [`destinazioni_da`] su `HARD_SETTINGS`; se il lock non si prende,
/// malformata.
pub fn destinazioni() -> Destinazioni {
    match HARD_SETTINGS.read() {
        Ok(fisse) => destinazioni_da(&fisse),
        Err(_) => Destinazioni::Malformato,
    }
}

/// Le destinazioni esterne di una sessione: quelle elencate la cui porta sta
/// fra le porte del lasciapassare, in ordine. L'unica regola: la usano il PC
/// (`tunnel::TunnelPc`), la finestra della richiesta e la voce del CM, e la
/// riga INFO, cosi' cio' che il cliente vede e' cio' che il PC apre.
pub fn della_sessione(d: &Destinazioni, porte: &[u16]) -> Vec<SocketAddrV4> {
    match d {
        Destinazioni::Elenco(voci) => voci
            .iter()
            .filter(|v| porte.contains(&v.port()))
            .copied()
            .collect(),
        Destinazioni::Assente | Destinazioni::Malformato => Vec::new(),
    }
}

/// La riga ERROR per la chiave malformata. Si chiama una volta per
/// connessione dell'agente col lasciapassare valido di tunnel
/// (`lasciapassare::verifica_login`, accanto a
/// `non_presidiato::segnala_porte_malformate`), anche sul server non
/// presidiato spento: la chiave vale anche col clic.
pub fn segnala_malformate(valido: &Valido) {
    if matches!(valido.capacita, Capacita::Tunnel { .. })
        && destinazioni() == Destinazioni::Malformato
    {
        log::error!(
            "chiave {} malformata: il tunnel va solo a 127.0.0.1",
            CHIAVE_DESTINAZIONI
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn con(valore: &str) -> Destinazioni {
        destinazioni_da(&HashMap::from([(
            CHIAVE_DESTINAZIONI.to_owned(),
            valore.to_owned(),
        )]))
    }

    fn elenco(voci: &[&str]) -> Destinazioni {
        Destinazioni::Elenco(voci.iter().map(|v| v.parse().unwrap()).collect())
    }

    #[test]
    fn la_costante_su_una_riga() {
        let righe: Vec<&str> = include_str!("destinazioni.rs").lines().collect();
        assert!(righe
            .contains(&r#"pub const CHIAVE_DESTINAZIONI: &str = "remotek-agente-destinazioni";"#));
    }

    #[test]
    fn la_chiave_valida() {
        assert_eq!(destinazioni_da(&HashMap::new()), Destinazioni::Assente);
        assert_eq!(con("10.0.0.5:1433"), elenco(&["10.0.0.5:1433"]));
        assert_eq!(
            con("192.168.1.20:8080,10.0.0.5:1433,10.0.0.5:8080"),
            elenco(&["10.0.0.5:1433", "10.0.0.5:8080", "192.168.1.20:8080"])
        );
        assert_eq!(
            con("1.0.0.0:1,223.255.255.254:65535,128.0.0.1:22"),
            elenco(&["1.0.0.0:1", "223.255.255.254:65535", "128.0.0.1:22"])
        );
    }

    #[test]
    fn la_chiave_malformata() {
        for v in [
            "",
            ",",
            "10.0.0.5:1433,",
            ",10.0.0.5:1433",
            "10.0.0.5:1433,10.0.0.5:1433",
            "10.0.0.5:1433, 10.0.0.6:1433",
            " 10.0.0.5:1433",
            "10.0.0.5 :1433",
            "10.0.0.5",
            "10.0.0.5:",
            ":1433",
            "10.0.0.5:0",
            "10.0.0.5:01433",
            "10.0.0.5:65536",
            "10.0.0.5:+1433",
            "10.0.0.5:-1",
            "10.0.0.5:1433:1",
            "010.0.0.5:1433",
            "10.00.0.5:1433",
            "10.0.0.05:1433",
            "10.0.0:1433",
            "10.0.0.5.1:1433",
            "256.0.0.1:1433",
            "sql.cliente.local:1433",
            "localhost:1433",
            "[::1]:1433",
            "::1:1433",
            "fe80::1:1433",
            "[fe80::1]:1433",
            "0x0a.0.0.5:1433",
            "10.0.0.5:1433;10.0.0.6:1433",
            // Gli indirizzi vietati.
            "127.0.0.1:1433",
            "127.0.0.2:1433",
            "127.255.255.255:1433",
            "0.0.0.0:1433",
            "0.1.2.3:1433",
            "255.255.255.255:1433",
            "224.0.0.1:1433",
            "239.255.255.255:1433",
            "10.0.0.5:1433,127.0.0.1:8080",
        ] {
            assert_eq!(con(v), Destinazioni::Malformato, "{v:?}");
        }
    }

    #[test]
    fn gli_indirizzi() {
        assert_eq!(indirizzo("10.0.0.5"), Some(Ipv4Addr::new(10, 0, 0, 5)));
        assert_eq!(
            indirizzo("240.0.0.1"),
            Some(Ipv4Addr::new(240, 0, 0, 1)),
            "solo 255.255.255.255 fra i riservati"
        );
        for no in [
            "127.0.0.1",
            "127.1.2.3",
            "0.0.0.0",
            "0.255.0.1",
            "255.255.255.255",
            "224.0.0.0",
            "239.1.2.3",
            "010.0.0.5",
            "10.0.0.5 ",
            "nas",
            "::1",
            "",
        ] {
            assert_eq!(indirizzo(no), None, "{no:?}");
        }
    }

    #[test]
    fn le_destinazioni_della_sessione() {
        let d = elenco(&["10.0.0.5:1433", "10.0.0.5:8080", "10.0.0.6:5432"]);
        let v = |s: &str| s.parse::<SocketAddrV4>().unwrap();
        assert_eq!(
            della_sessione(&d, &[8080, 1433]),
            vec![v("10.0.0.5:1433"), v("10.0.0.5:8080")]
        );
        assert_eq!(della_sessione(&d, &[5432]), vec![v("10.0.0.6:5432")]);
        assert_eq!(della_sessione(&d, &[22]), vec![]);
        assert_eq!(della_sessione(&Destinazioni::Assente, &[1433]), vec![]);
        assert_eq!(della_sessione(&Destinazioni::Malformato, &[1433]), vec![]);
    }

    /// Solo `HARD_SETTINGS`: in `override-settings` o `default-settings` la
    /// chiave non vale. Porta 9: nessun altro test chiede quella porta.
    #[test]
    fn solo_dalle_impostazioni_fisse() {
        use hbb_common::config::{DEFAULT_SETTINGS, OVERWRITE_SETTINGS};
        let voce = "10.9.9.9:9";
        for m in [&*OVERWRITE_SETTINGS, &*DEFAULT_SETTINGS] {
            m.write()
                .unwrap()
                .insert(CHIAVE_DESTINAZIONI.to_owned(), voce.to_owned());
        }
        assert_eq!(della_sessione(&destinazioni(), &[9]), vec![]);
        for m in [&*OVERWRITE_SETTINGS, &*DEFAULT_SETTINGS] {
            m.write().unwrap().remove(CHIAVE_DESTINAZIONI);
        }
        HARD_SETTINGS
            .write()
            .unwrap()
            .insert(CHIAVE_DESTINAZIONI.to_owned(), voce.to_owned());
        let lette = destinazioni();
        HARD_SETTINGS.write().unwrap().remove(CHIAVE_DESTINAZIONI);
        assert_eq!(lette, elenco(&[voce]));
        assert_eq!(della_sessione(&lette, &[9]), vec![voce.parse().unwrap()]);
    }
}
