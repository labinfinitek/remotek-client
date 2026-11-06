//! Il tunnel dell'agente AI (ADR-0036): una connessione di port forward per
//! sessione, con piu' flussi TCP dentro.
//!
//! Dopo il login il controllante (`remotek-cli tunnel`) e il PC si scambiano
//! i messaggi di questo file, ciascuno in un messaggio intero della
//! connessione (`Stream::send_raw` e `Stream::next`, cifrati con la chiave
//! della sessione: il tunnel non chiama `set_raw`, che toglierebbe la
//! cifratura e i confini dei messaggi). Lo stesso codice gira sui due capi.
//!
//! Un flusso e' una connessione TCP accettata dal CLI su `127.0.0.1`, verso
//! una porta del PC. L'apertura porta anche l'indirizzo IPv4 di destinazione
//! (ADR-0037, per un passo dopo): oggi il CLI manda sempre `127.0.0.1` e il PC
//! si collega solo a `127.0.0.1` ([`indirizzo_ammesso`]),
//! solo alle porte del lasciapassare. Ogni verso di ogni flusso ha il suo
//! credito: chi manda non supera i byte che l'altro capo gli ha concesso
//! ([`FINESTRA`] all'inizio), e il capo che riceve concede di nuovo solo dopo
//! aver scritto sul socket locale. Cosi' un flusso lento non ferma gli altri
//! e nessun capo tiene in memoria piu' della finestra per flusso. Un
//! messaggio malformato o un credito superato chiude la connessione.

use super::lasciapassare::{Capacita, Valido};
use hbb_common::{
    bytes::{BufMut, Bytes, BytesMut},
    log,
    tokio::{
        self,
        io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
        net::TcpStream,
        sync::mpsc,
        task::AbortHandle,
        time::Instant,
    },
    tokio_util::codec::{BytesCodec, Framed},
};
use std::{
    collections::HashMap,
    fmt,
    future::Future,
    io,
    net::{Ipv4Addr, SocketAddr},
    pin::Pin,
    time::Duration,
};

/// Flussi aperti insieme in una connessione.
pub const MASSIMO_FLUSSI: usize = 64;
/// Byte di dati in un messaggio.
pub const MASSIMO_DATI: usize = 32 * 1024;
/// Byte che un capo puo' mandare su un flusso prima che l'altro ne conceda
/// altri.
pub const FINESTRA: u32 = 256 * 1024;
/// Le porte di un tunnel: da 1 a 8, diverse, mai 0.
pub const MASSIMO_PORTE: usize = 8;
/// La durata del lasciapassare di tunnel, in secondi.
pub const DURATA_MINIMA: u32 = 60;
pub const DURATA_MASSIMA: u32 = 28800;

const TIPO_APRI: u8 = 1;
const TIPO_DATI: u8 = 2;
const TIPO_CHIUDI: u8 = 3;
const TIPO_ERRORE: u8 = 4;
const TIPO_CREDITO: u8 = 5;
const TIPO_FINE: u8 = 6;

/// Un messaggio del tunnel. `flusso` non e' mai 0.
#[derive(Debug, Clone, PartialEq)]
pub enum Messaggio {
    /// Dal controllante: apre il flusso verso `indirizzo`:`porta` visti dal
    /// PC. L'indirizzo c'e' per ADR-0037; oggi vale solo `127.0.0.1`.
    Apri {
        flusso: u32,
        indirizzo: Ipv4Addr,
        porta: u16,
    },
    /// Da 1 a [`MASSIMO_DATI`] byte del flusso.
    Dati { flusso: u32, dati: Bytes },
    /// Chi lo manda non scrivera' altro su quel flusso (fine del suo verso).
    Chiudi { flusso: u32 },
    /// Il flusso e' finito per un errore, nei due versi.
    Errore { flusso: u32, errore: ErroreFlusso },
    /// Chi lo manda ha scritto `byte` sul socket locale: l'altro capo ne puo'
    /// mandare altrettanti.
    Credito { flusso: u32, byte: u32 },
    /// Dal PC: chiude la connessione, per il motivo detto.
    Fine { motivo: FineDalPc },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ErroreFlusso {
    PortaNonAmmessa = 1,
    CollegamentoFallito = 2,
    /// Il socket locale ha dato errore.
    Interrotto = 3,
    /// Un indirizzo di destinazione diverso da `127.0.0.1` (ADR-0037).
    IndirizzoNonAmmesso = 4,
}

/// Perche' il PC chiude il tunnel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FineDalPc {
    /// La durata del lasciapassare e' finita.
    Durata = 1,
    /// Il cliente l'ha chiuso dal connection manager.
    Cliente = 2,
    /// Un'ora senza traffico, come il port forward di upstream.
    Inattivita = 3,
    /// Un altro motivo (console web, errore del connection manager).
    Altro = 4,
}

/// Il messaggio non rispetta il formato.
#[derive(Debug, Clone, PartialEq)]
pub struct Malformato(pub &'static str);

impl ErroreFlusso {
    fn da_byte(b: u8) -> Option<Self> {
        [
            Self::PortaNonAmmessa,
            Self::CollegamentoFallito,
            Self::Interrotto,
            Self::IndirizzoNonAmmesso,
        ]
        .into_iter()
        .find(|e| *e as u8 == b)
    }
}

impl FineDalPc {
    fn da_byte(b: u8) -> Option<Self> {
        [Self::Durata, Self::Cliente, Self::Inattivita, Self::Altro]
            .into_iter()
            .find(|m| *m as u8 == b)
    }
}

impl fmt::Display for ErroreFlusso {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            Self::PortaNonAmmessa => "porta non ammessa",
            Self::CollegamentoFallito => "collegamento fallito",
            Self::Interrotto => "interrotto",
            Self::IndirizzoNonAmmesso => "indirizzo non ammesso",
        })
    }
}

impl fmt::Display for FineDalPc {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(match self {
            Self::Durata => "durata scaduta",
            Self::Cliente => "chiuso dal cliente",
            Self::Inattivita => "un'ora senza traffico",
            Self::Altro => "chiuso dal PC",
        })
    }
}

/// Il messaggio in byte: il tipo, poi i campi in big endian.
pub fn codifica(m: &Messaggio) -> Bytes {
    let mut b = BytesMut::with_capacity(11);
    match m {
        Messaggio::Apri {
            flusso,
            indirizzo,
            porta,
        } => {
            b.put_u8(TIPO_APRI);
            b.put_u32(*flusso);
            b.put_slice(&indirizzo.octets());
            b.put_u16(*porta);
        }
        Messaggio::Dati { flusso, dati } => {
            b.reserve(dati.len());
            b.put_u8(TIPO_DATI);
            b.put_u32(*flusso);
            b.put_slice(dati);
        }
        Messaggio::Chiudi { flusso } => {
            b.put_u8(TIPO_CHIUDI);
            b.put_u32(*flusso);
        }
        Messaggio::Errore { flusso, errore } => {
            b.put_u8(TIPO_ERRORE);
            b.put_u32(*flusso);
            b.put_u8(*errore as u8);
        }
        Messaggio::Credito { flusso, byte } => {
            b.put_u8(TIPO_CREDITO);
            b.put_u32(*flusso);
            b.put_u32(*byte);
        }
        Messaggio::Fine { motivo } => {
            b.put_u8(TIPO_FINE);
            b.put_u8(*motivo as u8);
        }
    }
    b.freeze()
}

/// Il messaggio dai byte. Ogni tipo ha la sua lunghezza esatta; flusso 0,
/// porta 0, dati vuoti o oltre [`MASSIMO_DATI`], credito 0 o oltre la
/// [`FINESTRA`] e codici sconosciuti sono malformati.
pub fn decodifica(b: Bytes) -> Result<Messaggio, Malformato> {
    let (&tipo, resto) = b.split_first().ok_or(Malformato("vuoto"))?;
    if tipo == TIPO_FINE {
        return match resto {
            [m] => FineDalPc::da_byte(*m)
                .map(|motivo| Messaggio::Fine { motivo })
                .ok_or(Malformato("motivo della fine")),
            _ => Err(Malformato("lunghezza")),
        };
    }
    let flusso = resto
        .get(..4)
        .map(|f| u32::from_be_bytes([f[0], f[1], f[2], f[3]]))
        .ok_or(Malformato("lunghezza"))?;
    if flusso == 0 {
        return Err(Malformato("flusso 0"));
    }
    let campi = &resto[4..];
    match (tipo, campi) {
        (TIPO_APRI, [i0, i1, i2, i3, a, b]) => match u16::from_be_bytes([*a, *b]) {
            0 => Err(Malformato("porta 0")),
            porta => Ok(Messaggio::Apri {
                flusso,
                indirizzo: Ipv4Addr::new(*i0, *i1, *i2, *i3),
                porta,
            }),
        },
        (TIPO_DATI, dati) if !dati.is_empty() && dati.len() <= MASSIMO_DATI => {
            Ok(Messaggio::Dati {
                flusso,
                dati: b.slice(5..),
            })
        }
        (TIPO_DATI, _) => Err(Malformato("dati vuoti o troppi")),
        (TIPO_CHIUDI, []) => Ok(Messaggio::Chiudi { flusso }),
        (TIPO_ERRORE, [e]) => ErroreFlusso::da_byte(*e)
            .map(|errore| Messaggio::Errore { flusso, errore })
            .ok_or(Malformato("codice d'errore")),
        (TIPO_CREDITO, [a, b, c, d]) => match u32::from_be_bytes([*a, *b, *c, *d]) {
            byte @ 1..=FINESTRA => Ok(Messaggio::Credito { flusso, byte }),
            _ => Err(Malformato("credito")),
        },
        (TIPO_APRI | TIPO_CHIUDI | TIPO_ERRORE | TIPO_CREDITO, _) => Err(Malformato("lunghezza")),
        _ => Err(Malformato("tipo sconosciuto")),
    }
}

/// L'indirizzo a cui il PC si collega per un flusso, da quello chiesto
/// nell'apertura: solo `127.0.0.1` (mai un nome, mai `localhost`) con una
/// porta del lasciapassare. Altrimenti l'errore del flusso.
pub fn indirizzo_ammesso(
    indirizzo: Ipv4Addr,
    porta: u16,
    ammesse: &[u16],
) -> Result<SocketAddr, ErroreFlusso> {
    if indirizzo != Ipv4Addr::LOCALHOST {
        return Err(ErroreFlusso::IndirizzoNonAmmesso);
    }
    if porta == 0 || !ammesse.contains(&porta) {
        return Err(ErroreFlusso::PortaNonAmmessa);
    }
    Ok(SocketAddr::from((Ipv4Addr::LOCALHOST, porta)))
}

/// Le porte di un tunnel vanno bene: da 1 a [`MASSIMO_PORTE`], diverse, mai 0.
pub fn porte_valide(porte: &[u16]) -> bool {
    (1..=MASSIMO_PORTE).contains(&porte.len())
        && !porte.contains(&0)
        && porte
            .iter()
            .enumerate()
            .all(|(i, p)| !porte[..i].contains(p))
}

/// La durata del tunnel va bene: da [`DURATA_MINIMA`] a [`DURATA_MASSIMA`].
pub fn durata_valida(durata: i64) -> bool {
    (i64::from(DURATA_MINIMA)..=i64::from(DURATA_MASSIMA)).contains(&durata)
}

/// Quale capo della connessione.
#[derive(Debug, Clone, PartialEq)]
pub enum Lato {
    /// `remotek-cli tunnel`: apre i flussi.
    Controllante,
    /// Il PC: accetta i flussi verso le porte del lasciapassare.
    Pc { porte: Vec<u16> },
}

/// Il conto di un flusso aperto.
#[derive(Debug, Default)]
struct Conto {
    /// Byte che questo capo puo' ancora mandare.
    credito: u32,
    /// Byte ricevuti e non ancora scritti sul socket locale.
    da_scrivere: u32,
    /// Questo capo ha mandato `Chiudi`.
    chiuso_in_uscita: bool,
    /// L'altro capo ha mandato `Chiudi`.
    chiuso_in_entrata: bool,
}

/// Cosa fare per un messaggio ricevuto.
#[derive(Debug, PartialEq)]
pub enum Azione {
    /// Il PC si collega a questo indirizzo per il flusso nuovo.
    Collega { flusso: u32, indirizzo: SocketAddr },
    /// Il PC rifiuta il flusso nuovo: manda `Errore`.
    Rifiuta { flusso: u32, errore: ErroreFlusso },
    /// Scrivere sul socket locale del flusso.
    Scrivi { flusso: u32, dati: Bytes },
    /// Il flusso puo' mandare altri `byte`.
    Credito { flusso: u32, byte: u32 },
    /// L'altro capo non scrivera' altro: chiudere la scrittura locale.
    FineEntrata { flusso: u32 },
    /// Il flusso e' finito per un errore dell'altro capo.
    Interrotto { flusso: u32, errore: ErroreFlusso },
    /// Il PC chiude la connessione.
    Fine(FineDalPc),
    /// Un messaggio per un flusso gia' chiuso da questo capo.
    Ignora,
}

/// Lo stato dei flussi di una connessione, senza rete: i limiti, il credito
/// e le regole su chi apre e quali flussi esistono. Un errore e' una
/// violazione che chiude la connessione.
#[derive(Debug)]
pub struct Stato {
    lato: Lato,
    flussi: HashMap<u32, Conto>,
    /// L'ultimo flusso aperto: gli identificativi crescono e non si
    /// riusano, cosi' un messaggio per un flusso appena chiuso si riconosce.
    ultimo: u32,
}

impl Stato {
    pub fn nuovo(lato: Lato) -> Self {
        Self {
            lato,
            flussi: HashMap::new(),
            ultimo: 0,
        }
    }

    pub fn aperti(&self) -> usize {
        self.flussi.len()
    }

    pub fn esiste(&self, flusso: u32) -> bool {
        self.flussi.contains_key(&flusso)
    }

    /// Il controllante apre un flusso: il suo identificativo, o `None` se ce
    /// ne sono gia' [`MASSIMO_FLUSSI`] (o finiti gli identificativi).
    pub fn apri(&mut self) -> Option<u32> {
        if self.lato != Lato::Controllante || self.flussi.len() >= MASSIMO_FLUSSI {
            return None;
        }
        let flusso = self.ultimo.checked_add(1)?;
        self.ultimo = flusso;
        self.flussi.insert(flusso, Conto::nuovo());
        Some(flusso)
    }

    pub fn ricevi(&mut self, m: Messaggio) -> Result<Azione, Malformato> {
        let flusso = match &m {
            Messaggio::Fine { motivo } => {
                return match self.lato {
                    Lato::Controllante => Ok(Azione::Fine(*motivo)),
                    Lato::Pc { .. } => Err(Malformato("fine dal controllante")),
                }
            }
            Messaggio::Apri {
                flusso,
                indirizzo,
                porta,
            } => return self.apri_dal_controllante(*flusso, *indirizzo, *porta),
            Messaggio::Dati { flusso, .. }
            | Messaggio::Chiudi { flusso }
            | Messaggio::Errore { flusso, .. }
            | Messaggio::Credito { flusso, .. } => *flusso,
        };
        if flusso > self.ultimo {
            return Err(Malformato("flusso mai aperto"));
        }
        let Some(conto) = self.flussi.get_mut(&flusso) else {
            return Ok(Azione::Ignora);
        };
        match m {
            Messaggio::Dati { dati, .. } => {
                if conto.chiuso_in_entrata {
                    return Err(Malformato("dati dopo la chiusura"));
                }
                conto.da_scrivere = conto
                    .da_scrivere
                    .checked_add(dati.len() as u32)
                    .filter(|n| *n <= FINESTRA)
                    .ok_or(Malformato("credito superato"))?;
                Ok(Azione::Scrivi { flusso, dati })
            }
            Messaggio::Credito { byte, .. } => {
                if conto.chiuso_in_uscita {
                    return Ok(Azione::Ignora);
                }
                conto.credito = conto
                    .credito
                    .checked_add(byte)
                    .filter(|n| *n <= FINESTRA)
                    .ok_or(Malformato("credito oltre la finestra"))?;
                Ok(Azione::Credito { flusso, byte })
            }
            Messaggio::Chiudi { .. } => {
                if conto.chiuso_in_entrata {
                    return Err(Malformato("chiuso due volte"));
                }
                conto.chiuso_in_entrata = true;
                if conto.chiuso_in_uscita {
                    self.flussi.remove(&flusso);
                }
                Ok(Azione::FineEntrata { flusso })
            }
            Messaggio::Errore { errore, .. } => {
                self.flussi.remove(&flusso);
                Ok(Azione::Interrotto { flusso, errore })
            }
            Messaggio::Apri { .. } | Messaggio::Fine { .. } => Err(Malformato("impossibile")),
        }
    }

    fn apri_dal_controllante(
        &mut self,
        flusso: u32,
        indirizzo: Ipv4Addr,
        porta: u16,
    ) -> Result<Azione, Malformato> {
        let Lato::Pc { porte } = &self.lato else {
            return Err(Malformato("apertura dal PC"));
        };
        if flusso <= self.ultimo {
            return Err(Malformato("flusso gia' usato"));
        }
        if self.flussi.len() >= MASSIMO_FLUSSI {
            return Err(Malformato("troppi flussi"));
        }
        self.ultimo = flusso;
        match indirizzo_ammesso(indirizzo, porta, porte) {
            Ok(indirizzo) => {
                self.flussi.insert(flusso, Conto::nuovo());
                Ok(Azione::Collega { flusso, indirizzo })
            }
            Err(errore) => Ok(Azione::Rifiuta { flusso, errore }),
        }
    }

    /// Quanti byte il flusso puo' mandare ora (0 se chiuso o senza credito).
    pub fn credito(&self, flusso: u32) -> u32 {
        self.flussi
            .get(&flusso)
            .filter(|c| !c.chiuso_in_uscita)
            .map_or(0, |c| c.credito)
    }

    /// Questo capo manda `n` byte del flusso: `false` se il credito non basta
    /// o il flusso non puo' mandare (allora non si manda niente).
    pub fn manda(&mut self, flusso: u32, n: u32) -> bool {
        match self.flussi.get_mut(&flusso) {
            Some(c) if !c.chiuso_in_uscita && n <= c.credito => {
                c.credito -= n;
                true
            }
            _ => false,
        }
    }

    /// Scritti `n` byte sul socket locale: `true` se va mandato il `Credito`
    /// (il flusso c'e' ancora e l'altro capo puo' ancora scrivere).
    pub fn scritti(&mut self, flusso: u32, n: u32) -> bool {
        match self.flussi.get_mut(&flusso) {
            Some(c) => {
                c.da_scrivere = c.da_scrivere.saturating_sub(n);
                !c.chiuso_in_entrata
            }
            None => false,
        }
    }

    /// Il socket locale non dara' altro: `true` se va mandato `Chiudi`.
    pub fn fine_lettura(&mut self, flusso: u32) -> bool {
        let Some(c) = self.flussi.get_mut(&flusso) else {
            return false;
        };
        if c.chiuso_in_uscita {
            return false;
        }
        c.chiuso_in_uscita = true;
        if c.chiuso_in_entrata {
            self.flussi.remove(&flusso);
        }
        true
    }

    /// Il flusso finisce qui per un errore locale: `true` se va mandato
    /// `Errore` all'altro capo.
    pub fn interrompi(&mut self, flusso: u32) -> bool {
        self.flussi.remove(&flusso).is_some()
    }
}

impl Conto {
    fn nuovo() -> Self {
        Self {
            credito: FINESTRA,
            ..Default::default()
        }
    }
}

/// Un capo della connessione che manda e riceve messaggi interi.
pub trait Connessione: Send {
    fn manda(&mut self, messaggio: Bytes) -> impl Future<Output = io::Result<()>> + Send;
    /// Il prossimo messaggio; `None` quando l'altro capo ha chiuso.
    fn ricevi(&mut self) -> impl Future<Output = Option<io::Result<BytesMut>>> + Send;
}

/// La connessione di RustDesk: i messaggi passano da `send_raw` e `next`,
/// cifrati con la chiave della sessione.
impl Connessione for hbb_common::Stream {
    async fn manda(&mut self, messaggio: Bytes) -> io::Result<()> {
        self.send_raw(messaggio.to_vec())
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))
    }

    async fn ricevi(&mut self) -> Option<io::Result<BytesMut>> {
        self.next().await
    }
}

/// Come si collega il PC a un indirizzo ammesso.
pub type Collega<S> =
    Box<dyn Fn(SocketAddr) -> Pin<Box<dyn Future<Output = io::Result<S>> + Send>> + Send + Sync>;

/// Da dove vengono i flussi.
pub enum Apertura<S> {
    /// Il CLI: le connessioni accettate sulle porte locali, con la porta del
    /// PC a cui vanno.
    Controllante(mpsc::UnboundedReceiver<(u16, S)>),
    /// Il PC: le porte del lasciapassare e come collegarsi.
    Pc {
        porte: Vec<u16>,
        collega: Collega<S>,
    },
}

/// Perche' un capo chiude il tunnel, da fuori del ciclo.
#[derive(Debug, Clone, PartialEq)]
pub enum Chiusura {
    /// La durata e' finita.
    Durata,
    /// Il cliente dal connection manager.
    Cliente,
    /// L'agente (SIGINT, SIGTERM, Ctrl+C).
    Agente,
    Altro(String),
}

/// Come e' finito il tunnel.
#[derive(Debug, Clone, PartialEq)]
pub enum Motivo {
    Chiusura(Chiusura),
    /// Il PC ha mandato `Fine`.
    DalPc(FineDalPc),
    /// Nessun traffico per il tempo di inattivita'.
    Inattivita,
    /// L'altro capo ha chiuso la connessione.
    ChiusaDallAltroCapo,
    Rete(String),
    Protocollo(&'static str),
}

impl fmt::Display for Motivo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Motivo::Chiusura(Chiusura::Durata) => f.write_str("durata scaduta"),
            Motivo::Chiusura(Chiusura::Cliente) => f.write_str("chiuso dal cliente"),
            Motivo::Chiusura(Chiusura::Agente) => f.write_str("chiuso dall'agente"),
            Motivo::Chiusura(Chiusura::Altro(m)) => f.write_str(m),
            Motivo::DalPc(m) => write!(f, "il PC ha chiuso: {m}"),
            Motivo::Inattivita => f.write_str("nessun traffico per troppo tempo"),
            Motivo::ChiusaDallAltroCapo => f.write_str("connessione chiusa dall'altro capo"),
            Motivo::Rete(e) => write!(f, "rete: {e}"),
            Motivo::Protocollo(e) => write!(f, "messaggio non valido: {e}"),
        }
    }
}

/// Il resoconto della connessione, per la riga finale.
#[derive(Debug, Clone, PartialEq)]
pub struct Resoconto {
    pub motivo: Motivo,
    /// Flussi aperti in tutto.
    pub flussi: u64,
    /// Byte di dati mandati all'altro capo.
    pub byte_mandati: u64,
    /// Byte di dati ricevuti dall'altro capo.
    pub byte_ricevuti: u64,
}

/// Cosa dicono i compiti dei socket locali al ciclo.
enum Evento<S> {
    Letti(u32, Bytes),
    FineLettura(u32),
    Scritti(u32, u32),
    ErroreSocket(u32),
    Collegato(u32, io::Result<S>),
}

/// Il socket locale di un flusso: chi scrive, chi riceve il credito, i
/// compiti da fermare.
struct Locale {
    scrittura: Option<mpsc::UnboundedSender<Bytes>>,
    credito: Option<mpsc::UnboundedSender<u32>>,
    lettura: Option<AbortHandle>,
    compiti: Vec<AbortHandle>,
    /// Sul PC, i dati arrivati prima che il collegamento riesca.
    in_attesa: Vec<Bytes>,
}

impl Locale {
    fn in_collegamento(compito: AbortHandle) -> Self {
        Self {
            scrittura: None,
            credito: None,
            lettura: None,
            compiti: vec![compito],
            in_attesa: Vec::new(),
        }
    }

    /// Il flusso e' finito per un errore: si ferma tutto subito.
    fn ferma(self) {
        self.compiti.iter().for_each(AbortHandle::abort);
        if let Some(l) = self.lettura {
            l.abort();
        }
    }
}

/// I compiti di lettura e scrittura del socket `s` del flusso.
fn avvia_socket<S>(flusso: u32, s: S, eventi: &mpsc::UnboundedSender<Evento<S>>) -> Locale
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (mut lettura, mut scrittura) = tokio::io::split(s);
    let (tx_dati, mut rx_dati) = mpsc::unbounded_channel::<Bytes>();
    let (tx_credito, mut rx_credito) = mpsc::unbounded_channel::<u32>();
    let ev = eventi.clone();
    // Scrive in ordine i dati del flusso; finiti i dati (canale chiuso), chiude
    // la scrittura del socket.
    let scrivi = tokio::spawn(async move {
        while let Some(dati) = rx_dati.recv().await {
            if scrittura.write_all(&dati).await.is_err() {
                ev.send(Evento::ErroreSocket(flusso)).ok();
                return;
            }
            ev.send(Evento::Scritti(flusso, dati.len() as u32)).ok();
        }
        scrittura.shutdown().await.ok();
    });
    let ev = eventi.clone();
    // Legge solo quanto il credito permette.
    let leggi = tokio::spawn(async move {
        let mut credito = FINESTRA as usize;
        loop {
            while credito == 0 {
                match rx_credito.recv().await {
                    Some(n) => credito += n as usize,
                    None => return,
                }
            }
            while let Ok(n) = rx_credito.try_recv() {
                credito += n as usize;
            }
            let mut buf = BytesMut::with_capacity(credito.min(MASSIMO_DATI));
            match lettura.read_buf(&mut buf).await {
                Ok(0) => {
                    ev.send(Evento::FineLettura(flusso)).ok();
                    return;
                }
                Ok(n) => {
                    credito -= n;
                    ev.send(Evento::Letti(flusso, buf.freeze())).ok();
                }
                Err(_) => {
                    ev.send(Evento::ErroreSocket(flusso)).ok();
                    return;
                }
            }
        }
    });
    Locale {
        scrittura: Some(tx_dati),
        credito: Some(tx_credito),
        lettura: Some(leggi.abort_handle()),
        compiti: Vec::new(),
        in_attesa: Vec::new(),
    }
    .con_scrittura(scrivi.abort_handle())
}

impl Locale {
    /// Il compito di scrittura non si ferma alla fine del flusso: finisce di
    /// scrivere i dati gia' arrivati. Si ferma solo per un errore.
    fn con_scrittura(mut self, scrivi: AbortHandle) -> Self {
        self.compiti.push(scrivi);
        self
    }
}

/// Il ciclo del tunnel su un capo: legge i messaggi di `conn`, apre i flussi
/// (`apertura`), muove i dati col credito, finche' `fine` non risolve,
/// l'altro capo chiude, scade `inattivita` o arriva una violazione. Sul PC,
/// alla chiusura chiesta da fuori o per inattivita' manda `Fine`.
pub async fn ciclo<C, S>(
    conn: &mut C,
    apertura: Apertura<S>,
    fine: impl Future<Output = Chiusura>,
    inattivita: Option<Duration>,
) -> Resoconto
where
    C: Connessione,
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (mut nuovi, collega, lato) = match apertura {
        Apertura::Controllante(rx) => (Some(rx), None, Lato::Controllante),
        Apertura::Pc { porte, collega } => (None, Some(collega), Lato::Pc { porte }),
    };
    let pc = matches!(lato, Lato::Pc { .. });
    let mut stato = Stato::nuovo(lato);
    let mut locali: HashMap<u32, Locale> = HashMap::new();
    let (tx_eventi, mut rx_eventi) = mpsc::unbounded_channel::<Evento<S>>();
    let mut r = Resoconto {
        motivo: Motivo::ChiusaDallAltroCapo,
        flussi: 0,
        byte_mandati: 0,
        byte_ricevuti: 0,
    };
    let mut ultimo_traffico = Instant::now();
    tokio::pin!(fine);
    let motivo = loop {
        let scadenza = inattivita.map(|d| ultimo_traffico + d);
        let silenzio = async move {
            match scadenza {
                Some(s) => tokio::time::sleep_until(s).await,
                None => std::future::pending().await,
            }
        };
        // I messaggi da mandare all'altro capo, decisi dal ramo che e' scattato.
        let mut da_mandare: Vec<Messaggio> = Vec::new();
        tokio::select! {
            ricevuto = conn.ricevi() => {
                let bytes = match ricevuto {
                    Some(Ok(b)) => b,
                    Some(Err(e)) => break Motivo::Rete(e.to_string()),
                    None => break Motivo::ChiusaDallAltroCapo,
                };
                ultimo_traffico = Instant::now();
                let azione = match decodifica(bytes.freeze()).and_then(|m| stato.ricevi(m)) {
                    Ok(a) => a,
                    Err(Malformato(e)) => break Motivo::Protocollo(e),
                };
                match azione {
                    Azione::Collega { flusso, indirizzo } => {
                        r.flussi += 1;
                        // Il collegamento non ferma il ciclo: lo fa un compito.
                        let futuro = collega.as_ref().map(|c| c(indirizzo));
                        let ev = tx_eventi.clone();
                        let compito = tokio::spawn(async move {
                            let esito = match futuro {
                                Some(f) => f.await,
                                None => Err(io::Error::new(io::ErrorKind::Other, "nessun collegamento")),
                            };
                            ev.send(Evento::Collegato(flusso, esito)).ok();
                        });
                        locali.insert(flusso, Locale::in_collegamento(compito.abort_handle()));
                    }
                    Azione::Rifiuta { flusso, errore } => {
                        log::warn!("tunnel: flusso {} rifiutato: {}", flusso, errore);
                        da_mandare.push(Messaggio::Errore { flusso, errore });
                    }
                    Azione::Scrivi { flusso, dati } => {
                        r.byte_ricevuti += dati.len() as u64;
                        if let Some(l) = locali.get_mut(&flusso) {
                            match &l.scrittura {
                                Some(tx) => {
                                    tx.send(dati).ok();
                                }
                                None => l.in_attesa.push(dati),
                            }
                        }
                    }
                    Azione::Credito { flusso, byte } => {
                        if let Some(tx) = locali.get(&flusso).and_then(|l| l.credito.as_ref()) {
                            tx.send(byte).ok();
                        }
                    }
                    Azione::FineEntrata { flusso } => {
                        // Senza il mittente il compito di scrittura finisce i
                        // dati gia' arrivati, poi chiude la scrittura.
                        if let Some(l) = locali.get_mut(&flusso) {
                            l.scrittura = None;
                        }
                    }
                    Azione::Interrotto { flusso, errore } => {
                        log::debug!("tunnel: flusso {} chiuso dall'altro capo: {}", flusso, errore);
                        if let Some(l) = locali.remove(&flusso) {
                            l.ferma();
                        }
                    }
                    Azione::Fine(m) => break Motivo::DalPc(m),
                    Azione::Ignora => {}
                }
            }
            Some(evento) = rx_eventi.recv() => match evento {
                Evento::Letti(flusso, dati) => {
                    if stato.manda(flusso, dati.len() as u32) {
                        r.byte_mandati += dati.len() as u64;
                        da_mandare.push(Messaggio::Dati { flusso, dati });
                    }
                }
                Evento::Scritti(flusso, n) => {
                    if stato.scritti(flusso, n) {
                        da_mandare.push(Messaggio::Credito { flusso, byte: n });
                    }
                }
                Evento::FineLettura(flusso) => {
                    if stato.fine_lettura(flusso) {
                        da_mandare.push(Messaggio::Chiudi { flusso });
                    }
                }
                Evento::ErroreSocket(flusso) => {
                    if stato.interrompi(flusso) {
                        da_mandare.push(Messaggio::Errore { flusso, errore: ErroreFlusso::Interrotto });
                    }
                    if let Some(l) = locali.remove(&flusso) {
                        l.ferma();
                    }
                }
                Evento::Collegato(flusso, Ok(s)) => {
                    // Il flusso puo' essere gia' finito (Errore dell'altro capo).
                    if let Some(attesa) = locali.remove(&flusso) {
                        let mut l = avvia_socket(flusso, s, &tx_eventi);
                        if let Some(tx) = &l.scrittura {
                            for dati in attesa.in_attesa {
                                tx.send(dati).ok();
                            }
                        }
                        // L'altro capo aveva gia' chiuso il suo verso.
                        if chiuso_in_entrata(&stato, flusso) {
                            l.scrittura = None;
                        }
                        locali.insert(flusso, l);
                    }
                }
                Evento::Collegato(flusso, Err(e)) => {
                    log::warn!("tunnel: flusso {}: collegamento fallito: {}", flusso, e);
                    if stato.interrompi(flusso) {
                        da_mandare.push(Messaggio::Errore { flusso, errore: ErroreFlusso::CollegamentoFallito });
                    }
                    locali.remove(&flusso);
                }
            },
            Some((porta, s)) = async { match nuovi.as_mut() { Some(rx) => rx.recv().await, None => None } } => {
                match stato.apri() {
                    Some(flusso) => {
                        r.flussi += 1;
                        locali.insert(flusso, avvia_socket(flusso, s, &tx_eventi));
                        // Oggi sempre 127.0.0.1 del PC (ADR-0037).
                        da_mandare.push(Messaggio::Apri {
                            flusso,
                            indirizzo: Ipv4Addr::LOCALHOST,
                            porta,
                        });
                    }
                    // Oltre il limite il socket si chiude subito.
                    None => log::warn!("tunnel: gia' {} flussi aperti, connessione rifiutata", MASSIMO_FLUSSI),
                }
            }
            chiusura = &mut fine => break Motivo::Chiusura(chiusura),
            _ = silenzio => break Motivo::Inattivita,
        }
        // Un flusso finito nei due versi non ha piu' bisogno di leggere.
        locali.retain(|flusso, l| {
            let resta = stato.esiste(*flusso) || l.scrittura.is_some();
            if !resta {
                if let Some(lettura) = l.lettura.take() {
                    lettura.abort();
                }
            }
            resta
        });
        for m in da_mandare {
            if let Messaggio::Dati { .. } | Messaggio::Apri { .. } = m {
                ultimo_traffico = Instant::now();
            }
            if let Err(e) = conn.manda(codifica(&m)).await {
                r.motivo = Motivo::Rete(e.to_string());
                fine_dei_locali(locali);
                return r;
            }
        }
    };
    if pc {
        let motivo_pc = match &motivo {
            Motivo::Chiusura(Chiusura::Durata) => Some(FineDalPc::Durata),
            Motivo::Chiusura(Chiusura::Cliente) => Some(FineDalPc::Cliente),
            Motivo::Inattivita => Some(FineDalPc::Inattivita),
            Motivo::Chiusura(_) => Some(FineDalPc::Altro),
            _ => None,
        };
        if let Some(m) = motivo_pc {
            // L'altro capo puo' essere gia' sparito: la fine resta quella.
            conn.manda(codifica(&Messaggio::Fine { motivo: m }))
                .await
                .ok();
        }
    }
    fine_dei_locali(locali);
    r.motivo = motivo;
    r
}

fn chiuso_in_entrata(stato: &Stato, flusso: u32) -> bool {
    stato
        .flussi
        .get(&flusso)
        .map_or(true, |c| c.chiuso_in_entrata)
}

/// Alla fine della connessione si ferma tutto.
fn fine_dei_locali(locali: HashMap<u32, Locale>) {
    locali.into_values().for_each(Locale::ferma);
}

/// Il login di port forward del tunnel di remotek-cli (src/client.rs,
/// `handle_hash`): il lasciapassare va in `os_login.password`, con lo
/// username vuoto. Lo mette solo il CLI (`remotek_lasciapassare`), con host
/// vuoto e porta 0; il port forward del client grafico non lo ha mai.
pub fn login_del_tunnel(lc: &crate::client::LoginConfigHandler) -> bool {
    lc.conn_type == hbb_common::rendezvous_proto::ConnType::PORT_FORWARD
        && !lc.remotek_lasciapassare.is_empty()
        && lc.port_forward.0.is_empty()
        && lc.port_forward.1 == 0
}

/// Il port forward di una connessione sul PC (`port_forward_socket` di
/// src/server/connection.rs): il socket di upstream o il tunnel dell'agente,
/// che si collega flusso per flusso.
pub enum Inoltro {
    Upstream(Framed<TcpStream, BytesCodec>),
    Tunnel(TunnelPc),
}

/// Il tunnel autorizzato sul PC, dal lasciapassare verificato.
#[derive(Debug, Clone, PartialEq)]
pub struct TunnelPc {
    pub agente: String,
    pub tecnico: String,
    pub porte: Vec<u16>,
    pub durata: Duration,
    /// L'autorizzazione: la durata si conta da qui.
    pub inizio: Instant,
}

/// Il tunnel della connessione all'autorizzazione: solo col lasciapassare
/// valido di tunnel.
pub fn del_lasciapassare(valido: Option<&Valido>) -> Option<TunnelPc> {
    let v = valido?;
    let Capacita::Tunnel { porte, durata } = &v.capacita else {
        return None;
    };
    Some(TunnelPc {
        agente: v.agente.clone(),
        tecnico: v.tecnico.clone(),
        porte: porte.clone(),
        durata: Duration::from_secs(u64::from(*durata)),
        inizio: Instant::now(),
    })
}

/// Il login di port forward sul PC.
#[derive(Debug, PartialEq)]
pub enum LoginPortForward {
    /// Senza lasciapassare: il port forward di upstream, com'e'.
    Upstream,
    /// Il tunnel dell'agente col suo lasciapassare valido.
    Tunnel(Valido),
    /// Un lasciapassare non valido o non di tunnel, o il login del tunnel
    /// (host vuoto e porta 0) senza lasciapassare: rifiutato, mai upstream.
    Rifiuto,
}

/// Il testo del rifiuto del login, per il controllante.
pub const RIFIUTO_LOGIN: &str = "tunnel rifiutato: lasciapassare di tunnel non valido";

/// Il login di port forward e' quello del tunnel di remotek-cli: host vuoto e
/// porta 0, che upstream non usa (RDP e' host `RDP` e porta 0, il port
/// forward dei tecnici ha una porta).
pub fn forma_del_tunnel(host: &str, porta: i32) -> bool {
    host.is_empty() && porta == 0
}

/// Decide il login di port forward: `lasciapassare` e' quello tolto dal
/// login (`lasciapassare::togli`), `host` e `porta` quelli del `PortForward`,
/// `verifica` la verifica del lasciapassare (`lasciapassare::verifica_login`).
pub fn login_port_forward(
    lasciapassare: Option<String>,
    host: &str,
    porta: i32,
    verifica: impl FnOnce(&str) -> Option<Valido>,
) -> LoginPortForward {
    let Some(l) = lasciapassare else {
        return if forma_del_tunnel(host, porta) {
            LoginPortForward::Rifiuto
        } else {
            LoginPortForward::Upstream
        };
    };
    match verifica(&l) {
        Some(v) if super::lasciapassare::tunnel(Some(&v)) => LoginPortForward::Tunnel(v),
        _ => LoginPortForward::Rifiuto,
    }
}

/// Cosa mostra il CM come "Port Forward" della connessione.
pub fn descrizione(v: &Valido) -> String {
    match &v.capacita {
        Capacita::Tunnel { porte, .. } => porte
            .iter()
            .map(|p| format!("127.0.0.1:{p}"))
            .collect::<Vec<_>>()
            .join(", "),
        _ => String::new(),
    }
}

/// La connessione sul PC. Un `TestDelay` mandato prima dell'autorizzazione
/// puo' avere la risposta del controllante ancora in viaggio: se c'era
/// (`test_delay_in_volo`), il primo messaggio che e' un `TestDelay` si salta.
/// Nessun messaggio del tunnel si legge come `TestDelay`: il loro primo byte
/// (il tipo, da 1 a 6) e' un campo 0, che protobuf non ammette.
struct ConnessionePc<'a> {
    stream: &'a mut hbb_common::Stream,
    test_delay_in_volo: bool,
}

impl Connessione for ConnessionePc<'_> {
    async fn manda(&mut self, messaggio: Bytes) -> io::Result<()> {
        Connessione::manda(self.stream, messaggio).await
    }

    async fn ricevi(&mut self) -> Option<io::Result<BytesMut>> {
        loop {
            let r = self.stream.next().await;
            if let (true, Some(Ok(b))) = (self.test_delay_in_volo, &r) {
                self.test_delay_in_volo = false;
                if e_test_delay(b) {
                    continue;
                }
            }
            return r;
        }
    }
}

fn e_test_delay(b: &[u8]) -> bool {
    use hbb_common::{message_proto::Message, protobuf::Message as _};
    Message::parse_from_bytes(b).is_ok_and(|m| m.has_test_delay())
}

/// Il tunnel sul PC (`try_port_forward_loop` di src/server/connection.rs, al
/// posto della copia di upstream): i flussi fino alla durata del
/// lasciapassare contata dall'autorizzazione, alla chiusura dal CM, alla
/// console web o a un'ora senza traffico, come il port forward di upstream.
/// Alla fine una riga INFO, senza contenuto; l'Err porta il motivo a
/// `on_close`.
pub async fn servi_pc(
    stream: &mut hbb_common::Stream,
    tunnel: TunnelPc,
    rx_from_cm: &mut mpsc::UnboundedReceiver<crate::ipc::Data>,
    conn_id: i32,
    controllante: &str,
    test_delay_in_volo: bool,
) -> hbb_common::ResultType<()> {
    log::info!("tunnel dell'agente: porte {:?}", tunnel.porte);
    let inizio = chrono::Local::now();
    let mut hbbs_rx = crate::hbbs_http::sync::signal_receiver();
    let scadenza = tunnel.inizio + tunnel.durata;
    let fine = async {
        let durata = tokio::time::sleep_until(scadenza);
        tokio::pin!(durata);
        loop {
            tokio::select! {
                _ = &mut durata => return Chiusura::Durata,
                Some(d) = rx_from_cm.recv() => match d {
                    crate::ipc::Data::Close => return Chiusura::Cliente,
                    crate::ipc::Data::CmErr(e) => return Chiusura::Altro(format!("connection manager: {e}")),
                    _ => {}
                },
                Ok(conns) = hbbs_rx.recv() => {
                    if conns.contains(&conn_id) {
                        return Chiusura::Altro("chiuso dalla console web".to_owned());
                    }
                }
            }
        }
    };
    let collega: Collega<TcpStream> = Box::new(|indirizzo| {
        Box::pin(async move {
            hbb_common::timeout(3000, TcpStream::connect(indirizzo))
                .await
                .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "timeout"))?
        })
    });
    let mut conn = ConnessionePc {
        stream,
        test_delay_in_volo,
    };
    let porte = tunnel.porte.clone();
    let r = ciclo(
        &mut conn,
        Apertura::Pc { porte, collega },
        fine,
        Some(Duration::from_secs(3600)),
    )
    .await;
    log::info!(
        "tunnel chiuso: agente {}, tecnico {}, controllante {}, porte {:?}, inizio {}, fine {}, flussi {}, byte al controllante {}, byte dal controllante {}, motivo: {}",
        tunnel.agente,
        tunnel.tecnico,
        controllante,
        tunnel.porte,
        inizio.format("%Y-%m-%d %H:%M:%S"),
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        r.flussi,
        r.byte_mandati,
        r.byte_ricevuti,
        r.motivo
    );
    hbb_common::bail!("tunnel: {}", r.motivo)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remotek::lasciapassare::Modo;
    use hbb_common::tokio::{io::DuplexStream, sync::oneshot, time::timeout};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    const ATTESA: Duration = Duration::from_secs(10);

    fn tutti() -> Vec<Messaggio> {
        vec![
            Messaggio::Apri {
                flusso: 1,
                indirizzo: Ipv4Addr::LOCALHOST,
                porta: 1433,
            },
            Messaggio::Apri {
                flusso: u32::MAX,
                indirizzo: Ipv4Addr::LOCALHOST,
                porta: 65535,
            },
            Messaggio::Dati {
                flusso: 7,
                dati: Bytes::from_static(b"x"),
            },
            Messaggio::Dati {
                flusso: 7,
                dati: Bytes::from(vec![0xAB; MASSIMO_DATI]),
            },
            Messaggio::Chiudi { flusso: 2 },
            Messaggio::Errore {
                flusso: 3,
                errore: ErroreFlusso::PortaNonAmmessa,
            },
            Messaggio::Errore {
                flusso: 3,
                errore: ErroreFlusso::CollegamentoFallito,
            },
            Messaggio::Errore {
                flusso: 3,
                errore: ErroreFlusso::Interrotto,
            },
            Messaggio::Errore {
                flusso: 3,
                errore: ErroreFlusso::IndirizzoNonAmmesso,
            },
            Messaggio::Credito { flusso: 4, byte: 1 },
            Messaggio::Credito {
                flusso: 4,
                byte: FINESTRA,
            },
            Messaggio::Fine {
                motivo: FineDalPc::Durata,
            },
            Messaggio::Fine {
                motivo: FineDalPc::Cliente,
            },
            Messaggio::Fine {
                motivo: FineDalPc::Inattivita,
            },
            Messaggio::Fine {
                motivo: FineDalPc::Altro,
            },
        ]
    }

    #[test]
    fn andata_e_ritorno_di_ogni_messaggio() {
        for m in tutti() {
            assert_eq!(decodifica(codifica(&m)), Ok(m.clone()), "{m:?}");
        }
        assert_eq!(
            &codifica(&Messaggio::Apri {
                flusso: 1,
                indirizzo: Ipv4Addr::LOCALHOST,
                porta: 1433
            })[..],
            &[1, 0, 0, 0, 1, 127, 0, 0, 1, 0x05, 0x99]
        );
        // L'indirizzo di destinazione fa andata e ritorno com'e' (ADR-0037).
        let altro = Messaggio::Apri {
            flusso: 9,
            indirizzo: Ipv4Addr::new(192, 168, 10, 20),
            porta: 1433,
        };
        assert_eq!(
            &codifica(&altro)[..],
            &[1, 0, 0, 0, 9, 192, 168, 10, 20, 0x05, 0x99]
        );
        assert_eq!(decodifica(codifica(&altro)), Ok(altro));
    }

    #[test]
    fn messaggi_malformati() {
        let malformati: &[&[u8]] = &[
            &[],
            &[0],
            &[7, 0, 0, 0, 1],
            &[42, 0, 0, 0, 1],
            &[1, 0, 0, 0, 1, 127, 0, 0, 1, 0, 0],     // porta 0
            &[1, 0, 0, 0, 0, 127, 0, 0, 1, 0, 22],    // flusso 0
            &[1, 0, 0, 0, 1, 127, 0, 0, 1, 0],        // corto
            &[1, 0, 0, 0, 1, 127, 0, 0, 1, 0, 22, 0], // lungo
            &[1, 0, 0, 0, 1, 0, 22],                  // senza indirizzo
            &[2, 0, 0, 0, 1],                         // dati vuoti
            &[3, 0, 0, 0, 1, 0],
            &[3, 0, 0, 1],
            &[4, 0, 0, 0, 1, 9], // codice sconosciuto
            &[4, 0, 0, 0, 1],
            &[5, 0, 0, 0, 1, 0, 0, 0, 0], // credito 0
            &[5, 0, 0, 0, 1, 0, 4, 0, 1], // oltre la finestra
            &[5, 0, 0, 0, 1, 0, 0, 1],
            &[6],
            &[6, 0],
            &[6, 5],
            &[6, 1, 1],
        ];
        for m in malformati {
            assert!(decodifica(Bytes::copy_from_slice(m)).is_err(), "{m:?}");
        }
        let mut troppi = vec![2, 0, 0, 0, 1];
        troppi.extend(vec![0; MASSIMO_DATI + 1]);
        assert!(decodifica(Bytes::from(troppi)).is_err());
        // Una risposta a TestDelay non e' un messaggio del tunnel, e viceversa.
        let mut td = hbb_common::message_proto::Message::new();
        td.set_test_delay(Default::default());
        use hbb_common::protobuf::Message as _;
        let td = td.write_to_bytes().unwrap();
        assert!(e_test_delay(&td));
        assert!(decodifica(Bytes::from(td)).is_err());
        for m in tutti() {
            assert!(!e_test_delay(&codifica(&m)), "{m:?}");
        }
    }

    #[test]
    fn il_pc_si_collega_solo_a_127_0_0_1_e_alle_porte_ammesse() {
        let ammesse = [1433, 8080];
        let locale = Ipv4Addr::LOCALHOST;
        assert_eq!(
            indirizzo_ammesso(locale, 1433, &ammesse),
            Ok("127.0.0.1:1433".parse().unwrap())
        );
        assert_eq!(
            indirizzo_ammesso(locale, 8080, &ammesse),
            Ok("127.0.0.1:8080".parse().unwrap())
        );
        for porta in [0, 22, 1434, 3389, 65535] {
            assert_eq!(
                indirizzo_ammesso(locale, porta, &ammesse),
                Err(ErroreFlusso::PortaNonAmmessa),
                "{porta}"
            );
        }
        assert_eq!(
            indirizzo_ammesso(locale, 1433, &[]),
            Err(ErroreFlusso::PortaNonAmmessa)
        );
        // Un altro indirizzo, anche con una porta ammessa: mai (ADR-0037).
        for altro in [
            Ipv4Addr::new(127, 0, 0, 2),
            Ipv4Addr::UNSPECIFIED,
            Ipv4Addr::new(192, 168, 1, 10),
            Ipv4Addr::BROADCAST,
        ] {
            assert_eq!(
                indirizzo_ammesso(altro, 1433, &ammesse),
                Err(ErroreFlusso::IndirizzoNonAmmesso),
                "{altro}"
            );
        }
        // Lo stesso nello stato: la porta non ammessa e' un errore del flusso.
        let mut pc = Stato::nuovo(Lato::Pc {
            porte: ammesse.to_vec(),
        });
        assert_eq!(
            pc.ricevi(Messaggio::Apri {
                flusso: 1,
                indirizzo: Ipv4Addr::LOCALHOST,
                porta: 22
            }),
            Ok(Azione::Rifiuta {
                flusso: 1,
                errore: ErroreFlusso::PortaNonAmmessa
            })
        );
        assert_eq!(
            pc.ricevi(Messaggio::Apri {
                flusso: 2,
                indirizzo: Ipv4Addr::LOCALHOST,
                porta: 1433
            }),
            Ok(Azione::Collega {
                flusso: 2,
                indirizzo: "127.0.0.1:1433".parse().unwrap()
            })
        );
        assert_eq!(pc.aperti(), 1);
    }

    #[test]
    fn porte_e_durata() {
        assert!(porte_valide(&[1433]));
        assert!(porte_valide(&[1, 2, 3, 4, 5, 6, 7, 65535]));
        assert!(!porte_valide(&[]));
        assert!(!porte_valide(&[0]));
        assert!(!porte_valide(&[1433, 1433]));
        assert!(!porte_valide(&[1, 2, 3, 4, 5, 6, 7, 8, 9]));
        assert!(durata_valida(60) && durata_valida(28800));
        assert!(!durata_valida(59) && !durata_valida(28801) && !durata_valida(0));
    }

    #[test]
    fn credito_e_regole_dello_stato() {
        let mut cli = Stato::nuovo(Lato::Controllante);
        let f = cli.apri().unwrap();
        assert_eq!(cli.credito(f), FINESTRA);
        assert!(cli.manda(f, FINESTRA));
        assert!(!cli.manda(f, 1), "oltre il credito non si manda");
        assert_eq!(
            cli.ricevi(Messaggio::Credito {
                flusso: f,
                byte: 10
            }),
            Ok(Azione::Credito {
                flusso: f,
                byte: 10
            })
        );
        assert!(cli.manda(f, 10));
        // Credito oltre la finestra: violazione.
        assert!(cli
            .ricevi(Messaggio::Credito {
                flusso: f,
                byte: 10
            })
            .is_ok());
        assert!(cli
            .ricevi(Messaggio::Credito {
                flusso: f,
                byte: FINESTRA
            })
            .is_err());
        // Il controllante non riceve aperture, e niente per flussi mai aperti.
        let mut cli = Stato::nuovo(Lato::Controllante);
        assert!(cli
            .ricevi(Messaggio::Apri {
                flusso: 1,
                indirizzo: Ipv4Addr::LOCALHOST,
                porta: 22
            })
            .is_err());
        assert!(cli.ricevi(Messaggio::Chiudi { flusso: 1 }).is_err());
        assert_eq!(
            cli.ricevi(Messaggio::Fine {
                motivo: FineDalPc::Durata
            }),
            Ok(Azione::Fine(FineDalPc::Durata))
        );

        let mut pc = Stato::nuovo(Lato::Pc { porte: vec![1433] });
        assert!(pc
            .ricevi(Messaggio::Fine {
                motivo: FineDalPc::Durata
            })
            .is_err());
        assert!(pc
            .ricevi(Messaggio::Apri {
                flusso: 5,
                indirizzo: Ipv4Addr::LOCALHOST,
                porta: 1433
            })
            .is_ok());
        // Identificativi che non crescono.
        assert!(pc
            .ricevi(Messaggio::Apri {
                flusso: 5,
                indirizzo: Ipv4Addr::LOCALHOST,
                porta: 1433
            })
            .is_err());
        let mut pc = Stato::nuovo(Lato::Pc { porte: vec![1433] });
        pc.ricevi(Messaggio::Apri {
            flusso: 1,
            indirizzo: Ipv4Addr::LOCALHOST,
            porta: 1433,
        })
        .unwrap();
        // Dati oltre la finestra: violazione.
        let dati = Bytes::from(vec![0; MASSIMO_DATI]);
        for _ in 0..FINESTRA as usize / MASSIMO_DATI {
            pc.ricevi(Messaggio::Dati {
                flusso: 1,
                dati: dati.clone(),
            })
            .unwrap();
        }
        assert!(pc
            .ricevi(Messaggio::Dati {
                flusso: 1,
                dati: dati.clone()
            })
            .is_err());
        // Scritti, si concede di nuovo.
        assert!(pc.scritti(1, MASSIMO_DATI as u32));
        assert!(pc
            .ricevi(Messaggio::Dati {
                flusso: 1,
                dati: dati.clone()
            })
            .is_ok());
        // Chiusura nei due versi: poi i messaggi in ritardo si ignorano.
        assert_eq!(
            pc.ricevi(Messaggio::Chiudi { flusso: 1 }),
            Ok(Azione::FineEntrata { flusso: 1 })
        );
        assert!(pc
            .ricevi(Messaggio::Dati {
                flusso: 1,
                dati: dati.clone()
            })
            .is_err());
        assert!(!pc.scritti(1, 1), "niente credito a chi ha chiuso");
        assert!(pc.fine_lettura(1));
        assert!(!pc.esiste(1));
        assert_eq!(
            pc.ricevi(Messaggio::Credito { flusso: 1, byte: 1 }),
            Ok(Azione::Ignora)
        );
        assert_eq!(
            pc.ricevi(Messaggio::Dati { flusso: 1, dati }),
            Ok(Azione::Ignora)
        );
    }

    #[test]
    fn limite_dei_flussi() {
        let mut cli = Stato::nuovo(Lato::Controllante);
        let mut pc = Stato::nuovo(Lato::Pc { porte: vec![1433] });
        for _ in 0..MASSIMO_FLUSSI {
            let f = cli.apri().unwrap();
            assert!(matches!(
                pc.ricevi(Messaggio::Apri {
                    flusso: f,
                    indirizzo: Ipv4Addr::LOCALHOST,
                    porta: 1433
                }),
                Ok(Azione::Collega { .. })
            ));
        }
        assert_eq!(cli.apri(), None);
        assert!(pc
            .ricevi(Messaggio::Apri {
                flusso: 65,
                indirizzo: Ipv4Addr::LOCALHOST,
                porta: 1433
            })
            .is_err());
        // Chiuso uno, se ne apre un altro.
        assert!(cli.interrompi(3));
        pc.ricevi(Messaggio::Errore {
            flusso: 3,
            errore: ErroreFlusso::Interrotto,
        })
        .unwrap();
        let f = cli.apri().unwrap();
        assert_eq!(f, 65);
        assert!(pc
            .ricevi(Messaggio::Apri {
                flusso: f,
                indirizzo: Ipv4Addr::LOCALHOST,
                porta: 1433
            })
            .is_ok());
    }

    /// Una connessione in memoria: i messaggi interi su due canali.
    struct Canale {
        tx: mpsc::UnboundedSender<Bytes>,
        rx: mpsc::UnboundedReceiver<Bytes>,
    }

    impl Connessione for Canale {
        async fn manda(&mut self, m: Bytes) -> io::Result<()> {
            self.tx
                .send(m)
                .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "chiuso"))
        }

        async fn ricevi(&mut self) -> Option<io::Result<BytesMut>> {
            self.rx.recv().await.map(|b| Ok(BytesMut::from(&b[..])))
        }
    }

    fn coppia() -> (Canale, Canale) {
        let (a, b) = mpsc::unbounded_channel();
        let (c, d) = mpsc::unbounded_channel();
        (Canale { tx: a, rx: d }, Canale { tx: c, rx: b })
    }

    /// I due capi in memoria. Il CLI riceve i socket dell'app da `nuovi`; il
    /// PC si collega dando al test l'altro capo del socket in `server`.
    struct Prova {
        nuovi: mpsc::UnboundedSender<(u16, DuplexStream)>,
        server: mpsc::UnboundedReceiver<(SocketAddr, DuplexStream)>,
        chiudi_cli: oneshot::Sender<()>,
        cli: tokio::task::JoinHandle<Resoconto>,
        pc: tokio::task::JoinHandle<Resoconto>,
    }

    /// Socket locali piccoli: chi non legge si ferma presto.
    const SOCKET: usize = 1024;

    fn prova(
        porte: Vec<u16>,
        fine_pc: impl Future<Output = Chiusura> + Send + 'static,
        inattivita: Option<Duration>,
    ) -> Prova {
        let (mut c_cli, mut c_pc) = coppia();
        let (nuovi, rx_nuovi) = mpsc::unbounded_channel();
        let (tx_server, server) = mpsc::unbounded_channel();
        let (chiudi_cli, rx_chiudi) = oneshot::channel::<()>();
        let collega: Collega<DuplexStream> = Box::new(move |indirizzo| {
            let (a, b) = tokio::io::duplex(SOCKET);
            let esito = tx_server
                .send((indirizzo, b))
                .map(|()| a)
                .map_err(|_| io::Error::new(io::ErrorKind::Other, "test finito"));
            Box::pin(async move { esito })
        });
        let cli = tokio::spawn(async move {
            let fine = async move {
                rx_chiudi.await.ok();
                Chiusura::Agente
            };
            ciclo(&mut c_cli, Apertura::Controllante(rx_nuovi), fine, None).await
        });
        let pc = tokio::spawn(async move {
            ciclo(
                &mut c_pc,
                Apertura::Pc { porte, collega },
                fine_pc,
                inattivita,
            )
            .await
        });
        Prova {
            nuovi,
            server,
            chiudi_cli,
            cli,
            pc,
        }
    }

    impl Prova {
        /// L'app apre una connessione verso `porta` del PC.
        fn apri(&self, porta: u16) -> DuplexStream {
            let (app, locale) = tokio::io::duplex(SOCKET);
            self.nuovi.send((porta, locale)).unwrap();
            app
        }

        async fn server(&mut self) -> (SocketAddr, DuplexStream) {
            timeout(ATTESA, self.server.recv()).await.unwrap().unwrap()
        }

        async fn fine(self) -> (Resoconto, Resoconto) {
            self.chiudi_cli.send(()).unwrap();
            let cli = timeout(ATTESA, self.cli).await.unwrap().unwrap();
            let pc = timeout(ATTESA, self.pc).await.unwrap().unwrap();
            (cli, pc)
        }
    }

    async fn leggi_tutto(s: &mut DuplexStream) -> Vec<u8> {
        let mut v = Vec::new();
        timeout(ATTESA, s.read_to_end(&mut v))
            .await
            .unwrap()
            .unwrap();
        v
    }

    async fn leggi_n(s: &mut DuplexStream, n: usize) -> Vec<u8> {
        let mut v = vec![0; n];
        timeout(ATTESA, s.read_exact(&mut v))
            .await
            .unwrap()
            .unwrap();
        v
    }

    #[tokio::test]
    async fn due_flussi_insieme_su_due_porte() {
        let mut p = prova(vec![1433, 8080], std::future::pending(), None);
        let mut a = p.apri(1433);
        let (ind_a, mut srv_a) = p.server().await;
        let mut b = p.apri(8080);
        let (ind_b, mut srv_b) = p.server().await;
        assert_eq!(ind_a, "127.0.0.1:1433".parse().unwrap());
        assert_eq!(ind_b, "127.0.0.1:8080".parse().unwrap());
        a.write_all(b"select 1").await.unwrap();
        b.write_all(b"GET /").await.unwrap();
        assert_eq!(leggi_n(&mut srv_b, 5).await, b"GET /");
        assert_eq!(leggi_n(&mut srv_a, 8).await, b"select 1");
        srv_a.write_all(b"1").await.unwrap();
        srv_b.write_all(b"200").await.unwrap();
        assert_eq!(leggi_n(&mut a, 1).await, b"1");
        assert_eq!(leggi_n(&mut b, 3).await, b"200");
        // Piu' della finestra in un verso: il credito torna man mano.
        let grande: Vec<u8> = (0..3 * FINESTRA as usize).map(|i| i as u8).collect();
        let copia = grande.clone();
        let scrivi = tokio::spawn(async move {
            srv_a.write_all(&copia).await.unwrap();
            srv_a
        });
        assert_eq!(leggi_n(&mut a, grande.len()).await, grande);
        drop(scrivi.await.unwrap());
        let (cli, pc) = p.fine().await;
        assert_eq!(cli.motivo, Motivo::Chiusura(Chiusura::Agente));
        assert!(
            matches!(pc.motivo, Motivo::ChiusaDallAltroCapo | Motivo::Rete(_)),
            "{:?}",
            pc.motivo
        );
        assert_eq!((cli.flussi, pc.flussi), (2, 2));
        assert_eq!(cli.byte_mandati, 13);
        assert_eq!(pc.byte_ricevuti, 13);
        assert_eq!(cli.byte_ricevuti, 4 + grande.len() as u64);
        assert_eq!(pc.byte_mandati, cli.byte_ricevuti);
    }

    #[tokio::test]
    async fn un_flusso_che_non_legge_non_ferma_l_altro() {
        let mut p = prova(vec![1433, 8080], std::future::pending(), None);
        let mut lento = p.apri(1433);
        // Il server del flusso lento non legge mai.
        let (_, _srv_lento) = p.server().await;
        let scritti = Arc::new(AtomicUsize::new(0));
        let conta = scritti.clone();
        let scrivi = tokio::spawn(async move {
            let blocco = [7u8; 4096];
            loop {
                if lento.write_all(&blocco).await.is_err() {
                    return;
                }
                conta.fetch_add(blocco.len(), Ordering::SeqCst);
            }
        });
        let mut veloce = p.apri(8080);
        let (_, mut srv) = p.server().await;
        for i in 0..20u8 {
            veloce.write_all(&[i; 100]).await.unwrap();
            assert_eq!(leggi_n(&mut srv, 100).await, vec![i; 100]);
            srv.write_all(&[i]).await.unwrap();
            assert_eq!(leggi_n(&mut veloce, 1).await, vec![i]);
        }
        // Il flusso lento non va oltre la finestra (piu' i due socket e un
        // blocco in lettura).
        let massimo = FINESTRA as usize + 2 * SOCKET + MASSIMO_DATI + 4096;
        assert!(scritti.load(Ordering::SeqCst) <= massimo);
        scrivi.abort();
        p.fine().await;
    }

    #[tokio::test]
    async fn porta_non_ammessa_errore_del_flusso_gli_altri_continuano() {
        let mut p = prova(vec![1433], std::future::pending(), None);
        let mut buono = p.apri(1433);
        let (_, mut srv) = p.server().await;
        let mut vietato = p.apri(22);
        vietato.write_all(b"SSH-2.0").await.ok();
        // Il CLI chiude il socket dell'app: nessun dato, fine.
        assert_eq!(leggi_tutto(&mut vietato).await, b"");
        buono.write_all(b"ok").await.unwrap();
        assert_eq!(leggi_n(&mut srv, 2).await, b"ok");
        let (cli, pc) = p.fine().await;
        assert_eq!((cli.flussi, pc.flussi), (2, 1));
    }

    /// Un'apertura verso un indirizzo diverso da 127.0.0.1, con una porta
    /// ammessa: errore del flusso, nessun collegamento, gli altri continuano.
    #[tokio::test]
    async fn indirizzo_non_ammesso_errore_del_flusso_nessun_collegamento() {
        let (mut cli, mut pc) = coppia();
        let collegamenti = Arc::new(AtomicUsize::new(0));
        let conta = collegamenti.clone();
        let collega: Collega<DuplexStream> = Box::new(move |_| {
            conta.fetch_add(1, Ordering::SeqCst);
            let (a, _b) = tokio::io::duplex(SOCKET);
            Box::pin(async move { Ok(a) })
        });
        let lato_pc = tokio::spawn(async move {
            ciclo(
                &mut pc,
                Apertura::Pc {
                    porte: vec![1433],
                    collega,
                },
                std::future::pending(),
                None,
            )
            .await
        });
        let apri = |flusso, indirizzo| {
            codifica(&Messaggio::Apri {
                flusso,
                indirizzo,
                porta: 1433,
            })
        };
        cli.manda(apri(1, Ipv4Addr::new(10, 0, 0, 5)))
            .await
            .unwrap();
        let risposta = timeout(ATTESA, cli.ricevi())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(
            decodifica(risposta.freeze()),
            Ok(Messaggio::Errore {
                flusso: 1,
                errore: ErroreFlusso::IndirizzoNonAmmesso
            })
        );
        assert_eq!(collegamenti.load(Ordering::SeqCst), 0);
        // Il flusso dopo, verso 127.0.0.1, si collega.
        cli.manda(apri(2, Ipv4Addr::LOCALHOST)).await.unwrap();
        cli.manda(codifica(&Messaggio::Chiudi { flusso: 2 }))
            .await
            .unwrap();
        drop(cli);
        let r = timeout(ATTESA, lato_pc).await.unwrap().unwrap();
        assert_eq!(r.flussi, 1);
        assert_eq!(collegamenti.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn oltre_il_limite_il_cli_chiude_il_socket() {
        let mut p = prova(vec![1433], std::future::pending(), None);
        let mut app = Vec::new();
        for _ in 0..MASSIMO_FLUSSI {
            app.push(p.apri(1433));
            p.server().await;
        }
        let mut di_troppo = p.apri(1433);
        assert_eq!(leggi_tutto(&mut di_troppo).await, b"");
        let (cli, pc) = p.fine().await;
        assert_eq!((cli.flussi, pc.flussi), (64, 64));
    }

    #[tokio::test]
    async fn chiusure_dai_due_capi() {
        let mut p = prova(vec![1433], std::future::pending(), None);
        // L'app chiude la scrittura: il server vede la fine e risponde ancora.
        let mut app = p.apri(1433);
        let (_, mut srv) = p.server().await;
        app.write_all(b"richiesta").await.unwrap();
        app.shutdown().await.unwrap();
        assert_eq!(leggi_tutto(&mut srv).await, b"richiesta");
        srv.write_all(b"risposta").await.unwrap();
        drop(srv);
        assert_eq!(leggi_tutto(&mut app).await, b"risposta");
        // Il server chiude per primo.
        let mut app = p.apri(1433);
        let (_, mut srv) = p.server().await;
        srv.write_all(b"ciao").await.unwrap();
        srv.shutdown().await.unwrap();
        assert_eq!(leggi_tutto(&mut app).await, b"ciao");
        app.write_all(b"ancora").await.unwrap();
        drop(app);
        assert_eq!(leggi_tutto(&mut srv).await, b"ancora");
        let (cli, _) = p.fine().await;
        assert_eq!(cli.flussi, 2);
    }

    #[tokio::test]
    async fn il_pc_chiude_e_lo_dice() {
        // La durata (qui subito, dopo il primo flusso).
        let (scade, rx) = oneshot::channel::<()>();
        let fine = async move {
            rx.await.ok();
            Chiusura::Durata
        };
        let mut p = prova(vec![1433], fine, None);
        let mut app = p.apri(1433);
        p.server().await;
        scade.send(()).unwrap();
        assert_eq!(leggi_tutto(&mut app).await, b"");
        let cli = timeout(ATTESA, p.cli).await.unwrap().unwrap();
        let pc = timeout(ATTESA, p.pc).await.unwrap().unwrap();
        assert_eq!(cli.motivo, Motivo::DalPc(FineDalPc::Durata));
        assert_eq!(pc.motivo, Motivo::Chiusura(Chiusura::Durata));
        // Dal connection manager e per inattivita'.
        let p = prova(vec![1433], async { Chiusura::Cliente }, None);
        let cli = timeout(ATTESA, p.cli).await.unwrap().unwrap();
        assert_eq!(cli.motivo, Motivo::DalPc(FineDalPc::Cliente));
        let p = prova(vec![1433], std::future::pending(), Some(Duration::ZERO));
        let cli = timeout(ATTESA, p.cli).await.unwrap().unwrap();
        assert_eq!(cli.motivo, Motivo::DalPc(FineDalPc::Inattivita));
    }

    #[tokio::test]
    async fn un_messaggio_malformato_chiude_la_connessione() {
        for cattivo in [
            vec![9u8],
            codifica(&Messaggio::Dati {
                flusso: 1,
                dati: Bytes::from_static(b"x"),
            })
            .to_vec(),
            codifica(&Messaggio::Fine {
                motivo: FineDalPc::Durata,
            })
            .to_vec(),
        ] {
            let (mut altro, mut pc) = coppia();
            let (collega, _): (Collega<DuplexStream>, ()) = (
                Box::new(|_| Box::pin(async { Err(io::Error::new(io::ErrorKind::Other, "no")) })),
                (),
            );
            let compito = tokio::spawn(async move {
                ciclo(
                    &mut pc,
                    Apertura::Pc {
                        porte: vec![1433],
                        collega,
                    },
                    std::future::pending(),
                    None,
                )
                .await
            });
            altro.manda(Bytes::from(cattivo)).await.unwrap();
            let r = timeout(ATTESA, compito).await.unwrap().unwrap();
            assert!(matches!(r.motivo, Motivo::Protocollo(_)), "{:?}", r.motivo);
        }
    }

    /// I due capi su una vera connessione RustDesk (`Stream` su TCP, cifrata
    /// con la stessa chiave dai due lati) e veri socket su `127.0.0.1`.
    #[tokio::test]
    async fn su_stream_cifrato_e_socket_veri() {
        use hbb_common::sodiumoxide::crypto::secretbox;
        use tokio::net::TcpListener;
        let chiave = secretbox::gen_key();
        let ascolto = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (a, b) = tokio::join!(
            TcpStream::connect(ascolto.local_addr().unwrap()),
            ascolto.accept()
        );
        let a = a.unwrap();
        let (b, ind_b) = b.unwrap();
        let ind_a = a.local_addr().unwrap();
        let mut cli = hbb_common::Stream::from(a, ind_a);
        let mut pc = hbb_common::Stream::from(b, ind_b);
        cli.set_key(chiave.clone());
        pc.set_key(chiave);
        // Il "server SQL" del PC.
        let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let porta = server.local_addr().unwrap().port();
        let servi = tokio::spawn(async move {
            let (mut s, _) = server.accept().await.unwrap();
            let mut v = vec![0; 5];
            s.read_exact(&mut v).await.unwrap();
            s.write_all(&[v, b"!".to_vec()].concat()).await.unwrap();
        });
        let (nuovi, rx) = mpsc::unbounded_channel();
        let (chiudi, chiuso) = oneshot::channel::<()>();
        let lato_cli = tokio::spawn(async move {
            let fine = async move {
                chiuso.await.ok();
                Chiusura::Agente
            };
            ciclo(&mut cli, Apertura::Controllante(rx), fine, None).await
        });
        let collega: Collega<TcpStream> = Box::new(|i| Box::pin(TcpStream::connect(i)));
        let lato_pc = tokio::spawn(async move {
            let fine = async { std::future::pending::<Chiusura>().await };
            ciclo(
                &mut pc,
                Apertura::Pc {
                    porte: vec![porta],
                    collega,
                },
                fine,
                None,
            )
            .await
        });
        // L'app del CLI.
        let locale = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (app, accettato) = tokio::join!(
            TcpStream::connect(locale.local_addr().unwrap()),
            locale.accept()
        );
        let mut app = app.unwrap();
        nuovi.send((porta, accettato.unwrap().0)).unwrap();
        app.write_all(b"ciao?").await.unwrap();
        let mut v = vec![0; 6];
        timeout(ATTESA, app.read_exact(&mut v))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(v, b"ciao?!");
        timeout(ATTESA, servi).await.unwrap().unwrap();
        chiudi.send(()).unwrap();
        let r = timeout(ATTESA, lato_cli).await.unwrap().unwrap();
        assert_eq!((r.flussi, r.byte_mandati, r.byte_ricevuti), (1, 5, 6));
        let r = timeout(ATTESA, lato_pc).await.unwrap().unwrap();
        assert_eq!(r.motivo, Motivo::ChiusaDallAltroCapo);
    }

    fn valido(capacita: Capacita) -> Valido {
        Valido {
            agente: "agente-mario".into(),
            tecnico: "mario".into(),
            nonce: "n".into(),
            modo: Modo::Utente,
            capacita,
        }
    }

    #[test]
    fn il_login_di_port_forward() {
        let tunnel = valido(Capacita::Tunnel {
            porte: vec![1433, 8080],
            durata: 3600,
        });
        let mai = |_: &str| -> Option<Valido> { panic!("verificato") };
        // Senza lasciapassare: upstream, byte per byte.
        assert_eq!(
            login_port_forward(None, "localhost", 3389, mai),
            LoginPortForward::Upstream
        );
        assert_eq!(
            login_port_forward(None, "RDP", 0, mai),
            LoginPortForward::Upstream
        );
        // La forma del tunnel senza lasciapassare: rifiuto.
        assert_eq!(
            login_port_forward(None, "", 0, mai),
            LoginPortForward::Rifiuto
        );
        // Col lasciapassare valido di tunnel.
        assert_eq!(
            login_port_forward(Some("l".into()), "", 0, |l| {
                assert_eq!(l, "l");
                Some(tunnel.clone())
            }),
            LoginPortForward::Tunnel(tunnel.clone())
        );
        // Non valido o non di tunnel: rifiuto, mai upstream.
        for v in [
            None,
            Some(valido(Capacita::Exec)),
            Some(valido(Capacita::Terminale)),
        ] {
            for (host, porta) in [("", 0), ("localhost", 1433)] {
                assert_eq!(
                    login_port_forward(Some("l".into()), host, porta, |_| v.clone()),
                    LoginPortForward::Rifiuto
                );
            }
        }
        assert_eq!(descrizione(&tunnel), "127.0.0.1:1433, 127.0.0.1:8080");
        let t = del_lasciapassare(Some(&tunnel)).unwrap();
        assert_eq!(
            (t.porte, t.durata),
            (vec![1433, 8080], Duration::from_secs(3600))
        );
        assert!(del_lasciapassare(Some(&valido(Capacita::Exec))).is_none());
        assert!(del_lasciapassare(None).is_none());
    }
}
