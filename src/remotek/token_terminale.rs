//! Il terminale da utente non gira mai elevato (ADR-0028, caso 1).
//!
//! `handle_installed_user` (src/server/connection.rs) prende il token di
//! `explorer.exe` della sessione della console. Con l'Administrator
//! predefinito, o con un amministratore e UAC spento, quel token e' a
//! integrita' alta e ha BUILTIN\Administrators attivo: la shell sarebbe da
//! amministratore senza che nessuno l'abbia chiesto. Qui il token si abbassa
//! prima di salvarlo; se non si puo', il terminale si rifiuta, mai col token
//! pieno di ripiego. Il token abbassato e' dello stesso utente: la DACL delle
//! pipe dell'helper, che ammette il SID dell'utente del token, non cambia.
//!
//! La decisione sta in funzioni pure, provate anche su Linux; la parte
//! Windows sono solo le chiamate di sistema.

use hbb_common::{bail, ResultType};

/// Il messaggio al controllante quando il token non si abbassa.
pub const RIFIUTO: &str =
    "Terminale rifiutato: non si possono togliere alla shell i permessi di amministratore.";

/// Il messaggio al controllante quando il client non installato gira come
/// amministratore.
pub const RIFIUTO_PORTATILE: &str = "Terminale rifiutato: il client non installato gira come amministratore. Riavvialo senza \"Esegui come amministratore\".";

// winnt.h. Nel crate `windows` stanno in Win32_System_SystemServices, una
// feature che il client non abilita.
#[cfg(any(target_os = "windows", test))]
const SE_GROUP_ENABLED: u32 = 0x4;
#[cfg(any(target_os = "windows", test))]
const SE_GROUP_USE_FOR_DENY_ONLY: u32 = 0x10;
#[cfg(target_os = "windows")]
const SE_GROUP_INTEGRITY: u32 = 0x20;

// winnt.h, i flag di CreateRestrictedToken.
const DISABLE_MAX_PRIVILEGE: u32 = 0x1;
#[cfg(test)]
const SANDBOX_INERT: u32 = 0x2;
const LUA_TOKEN: u32 = 0x4;
#[cfg(test)]
const WRITE_RESTRICTED: u32 = 0x8;

/// I flag di `CreateRestrictedToken` per il ramo [`Scelta::Ristretto`]:
/// `LUA_TOKEN` e `DISABLE_MAX_PRIVILEGE`, che toglie tutti i privilegi
/// tranne SeChangeNotify, a prescindere da cosa toglie `LUA_TOKEN`.
pub const FLAG_RISTRETTO: u32 = LUA_TOKEN | DISABLE_MAX_PRIVILEGE;

/// Cosa fare del token dell'utente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scelta {
    /// Resta com'e': gia' filtrato, o di un utente senza Administrators attivo.
    ComE,
    /// Il token collegato (`TokenLinkedToken`): quello filtrato dello stesso
    /// utente, che UAC tiene accanto a quello elevato.
    Collegato,
    /// `CreateRestrictedToken` con [`FLAG_RISTRETTO`] e Administrators solo
    /// "deny-only", poi integrita' media.
    Ristretto,
}

/// La scelta dato il `TOKEN_ELEVATION_TYPE` (1 Default, 2 Full, 3 Limited) e,
/// solo per Default, se il gruppo Administrators e' attivo nel token. Un
/// errore in una lettura che serve, o un tipo sconosciuto, rifiuta.
pub fn scegli(
    elevazione: ResultType<i32>,
    amministratori: impl FnOnce() -> ResultType<bool>,
) -> ResultType<Scelta> {
    match elevazione? {
        1 if amministratori()? => Ok(Scelta::Ristretto),
        1 | 3 => Ok(Scelta::ComE),
        2 => Ok(Scelta::Collegato),
        altro => bail!("tipo di elevazione del token sconosciuto: {}", altro),
    }
}

/// L'ultimo controllo, sul token che si usera': Administrators non dev'essere
/// attivo, altrimenti (o se non si puo' leggere) si rifiuta.
pub fn controlla_abbassato(amministratori: ResultType<bool>) -> ResultType<()> {
    if amministratori? {
        bail!("il token abbassato ha ancora BUILTIN\\Administrators attivo");
    }
    Ok(())
}

/// Client portatile non installato: la shell gira col token del processo
/// (`TerminalUserToken::SelfUser`). Se il processo e' elevato la shell
/// sarebbe da amministratore senza che nessuno l'abbia chiesto: si rifiuta,
/// come quando il token non si abbassa. Un errore nella lettura vale come
/// elevato.
pub fn portatile(elevato: ResultType<bool>) -> ResultType<()> {
    if elevato? {
        bail!("il client non installato gira come amministratore");
    }
    Ok(())
}

/// Un gruppo conta come attivo se e' abilitato e non solo "deny-only".
#[cfg(any(target_os = "windows", test))]
fn attivo(attributi: u32) -> bool {
    attributi & SE_GROUP_ENABLED != 0 && attributi & SE_GROUP_USE_FOR_DENY_ONLY == 0
}

#[cfg(target_os = "windows")]
pub use windows_impl::abbassa;

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::*;
    use hbb_common::{anyhow::anyhow, log};
    use std::{ffi::c_void, mem::size_of};
    use windows::Win32::{
        Foundation::{CloseHandle, HANDLE},
        Security::{
            CreateRestrictedToken, CreateWellKnownSid, DuplicateTokenEx, EqualSid, GetLengthSid,
            GetTokenInformation, SecurityImpersonation, SetTokenInformation, TokenElevationType,
            TokenGroups, TokenIntegrityLevel, TokenLinkedToken, TokenPrimary,
            WinBuiltinAdministratorsSid, WinMediumLabelSid, CREATE_RESTRICTED_TOKEN_FLAGS, PSID,
            SECURITY_MAX_SID_SIZE, SID_AND_ATTRIBUTES, TOKEN_ALL_ACCESS, TOKEN_ELEVATION_TYPE,
            TOKEN_GROUPS, TOKEN_LINKED_TOKEN, TOKEN_MANDATORY_LABEL, WELL_KNOWN_SID_TYPE,
        },
    };

    // I valori di winnt.h scritti sopra sono quelli del crate.
    const _: () = assert!(
        DISABLE_MAX_PRIVILEGE == windows::Win32::Security::DISABLE_MAX_PRIVILEGE.0
            && LUA_TOKEN == windows::Win32::Security::LUA_TOKEN.0
    );

    /// Abbassa `token` (primario; passa al chiamato) e restituisce quello da
    /// usare, che il chiamante chiude. Se ne crea un altro, o se fallisce,
    /// chiude `token`.
    pub fn abbassa(token: usize) -> ResultType<usize> {
        let originale = HANDLE(token as _);
        let esito = abbassa_token(originale);
        if !matches!(&esito, Ok(t) if t.0 == originale.0) {
            chiudi(originale);
        }
        esito.map(|t| t.0 as usize)
    }

    fn abbassa_token(token: HANDLE) -> ResultType<HANDLE> {
        let nuovo = match scegli(elevazione(token), || amministratori_attivi(token))? {
            Scelta::ComE => token,
            Scelta::Collegato => collegato(token)?,
            Scelta::Ristretto => ristretto(token)?,
        };
        if let Err(e) = controlla_abbassato(amministratori_attivi(nuovo)) {
            if nuovo.0 != token.0 {
                chiudi(nuovo);
            }
            return Err(e);
        }
        Ok(nuovo)
    }

    fn elevazione(token: HANDLE) -> ResultType<i32> {
        let mut tipo = TOKEN_ELEVATION_TYPE::default();
        let mut lunghezza = 0u32;
        unsafe {
            GetTokenInformation(
                token,
                TokenElevationType,
                Some(&mut tipo as *mut _ as *mut c_void),
                size_of::<TOKEN_ELEVATION_TYPE>() as u32,
                &mut lunghezza,
            )
        }
        .map_err(|e| anyhow!("TokenElevationType: {}", e))?;
        Ok(tipo.0)
    }

    fn amministratori_attivi(token: HANDLE) -> ResultType<bool> {
        let amministratori = sid_noto(WinBuiltinAdministratorsSid)?;
        let mut lunghezza = 0u32;
        // La prima chiamata da' la lunghezza e fallisce con ERROR_INSUFFICIENT_BUFFER.
        if let Err(e) = unsafe { GetTokenInformation(token, TokenGroups, None, 0, &mut lunghezza) }
        {
            if lunghezza == 0 {
                bail!("TokenGroups: {}", e);
            }
        }
        // u64: TOKEN_GROUPS contiene puntatori e va allineato.
        let mut buffer = vec![0u64; (lunghezza as usize).div_ceil(8)];
        unsafe {
            GetTokenInformation(
                token,
                TokenGroups,
                Some(buffer.as_mut_ptr() as *mut c_void),
                lunghezza,
                &mut lunghezza,
            )
        }
        .map_err(|e| anyhow!("TokenGroups: {}", e))?;
        let gruppi = unsafe { &*(buffer.as_ptr() as *const TOKEN_GROUPS) };
        let elenco = unsafe {
            std::slice::from_raw_parts(gruppi.Groups.as_ptr(), gruppi.GroupCount as usize)
        };
        Ok(elenco.iter().any(|g| {
            attivo(g.Attributes) && unsafe { EqualSid(g.Sid, amministratori.psid()) }.is_ok()
        }))
    }

    fn collegato(token: HANDLE) -> ResultType<HANDLE> {
        let mut collegato = TOKEN_LINKED_TOKEN::default();
        let mut lunghezza = 0u32;
        unsafe {
            GetTokenInformation(
                token,
                TokenLinkedToken,
                Some(&mut collegato as *mut _ as *mut c_void),
                size_of::<TOKEN_LINKED_TOKEN>() as u32,
                &mut lunghezza,
            )
        }
        .map_err(|e| anyhow!("TokenLinkedToken: {}", e))?;
        // Fuori da SYSTEM il token collegato e' di impersonazione: si rende
        // primario, come vuole CreateProcessAsUserW.
        let mut primario = HANDLE::default();
        let esito = unsafe {
            DuplicateTokenEx(
                collegato.LinkedToken,
                TOKEN_ALL_ACCESS,
                None,
                SecurityImpersonation,
                TokenPrimary,
                &mut primario,
            )
        };
        chiudi(collegato.LinkedToken);
        esito.map_err(|e| anyhow!("DuplicateTokenEx del token collegato: {}", e))?;
        Ok(primario)
    }

    fn ristretto(token: HANDLE) -> ResultType<HANDLE> {
        let amministratori = sid_noto(WinBuiltinAdministratorsSid)?;
        let da_disattivare = [SID_AND_ATTRIBUTES {
            Sid: amministratori.psid(),
            Attributes: 0,
        }];
        let mut nuovo = HANDLE::default();
        unsafe {
            CreateRestrictedToken(
                token,
                CREATE_RESTRICTED_TOKEN_FLAGS(FLAG_RISTRETTO),
                Some(&da_disattivare),
                None,
                None,
                &mut nuovo,
            )
        }
        .map_err(|e| anyhow!("CreateRestrictedToken: {}", e))?;
        if let Err(e) = integrita_media(nuovo) {
            chiudi(nuovo);
            return Err(e);
        }
        Ok(nuovo)
    }

    fn integrita_media(token: HANDLE) -> ResultType<()> {
        let media = sid_noto(WinMediumLabelSid)?;
        let etichetta = TOKEN_MANDATORY_LABEL {
            Label: SID_AND_ATTRIBUTES {
                Sid: media.psid(),
                Attributes: SE_GROUP_INTEGRITY,
            },
        };
        let lunghezza =
            size_of::<TOKEN_MANDATORY_LABEL>() as u32 + unsafe { GetLengthSid(media.psid()) };
        unsafe {
            SetTokenInformation(
                token,
                TokenIntegrityLevel,
                &etichetta as *const _ as *const c_void,
                lunghezza,
            )
        }
        .map_err(|e| anyhow!("TokenIntegrityLevel media: {}", e))
    }

    /// Un SID noto, in un buffer allineato di almeno SECURITY_MAX_SID_SIZE byte.
    struct Sid([u64; 9]);

    impl Sid {
        fn psid(&self) -> PSID {
            PSID(self.0.as_ptr() as *mut c_void)
        }
    }

    fn sid_noto(tipo: WELL_KNOWN_SID_TYPE) -> ResultType<Sid> {
        let mut sid = Sid([0; 9]);
        let mut lunghezza = SECURITY_MAX_SID_SIZE;
        unsafe {
            CreateWellKnownSid(
                tipo,
                None,
                Some(PSID(sid.0.as_mut_ptr() as *mut c_void)),
                &mut lunghezza,
            )
        }
        .map_err(|e| anyhow!("CreateWellKnownSid {:?}: {}", tipo, e))?;
        Ok(sid)
    }

    fn chiudi(token: HANDLE) {
        if let Err(e) = unsafe { CloseHandle(token) } {
            log::warn!("chiusura del token del terminale non riuscita: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hbb_common::anyhow::anyhow;

    fn mai() -> ResultType<bool> {
        panic!("la presenza di Administrators non serve per questo tipo")
    }

    #[test]
    fn full_usa_il_token_collegato() {
        assert_eq!(scegli(Ok(2), mai).unwrap(), Scelta::Collegato);
    }

    #[test]
    fn limited_resta_com_e() {
        assert_eq!(scegli(Ok(3), mai).unwrap(), Scelta::ComE);
    }

    #[test]
    fn default_con_administrators_si_restringe() {
        assert_eq!(scegli(Ok(1), || Ok(true)).unwrap(), Scelta::Ristretto);
    }

    #[test]
    fn default_senza_administrators_resta_com_e() {
        assert_eq!(scegli(Ok(1), || Ok(false)).unwrap(), Scelta::ComE);
    }

    #[test]
    fn errori_e_tipi_sconosciuti_rifiutano() {
        assert!(scegli(Err(anyhow!("GetTokenInformation")), mai).is_err());
        assert!(scegli(Ok(1), || Err(anyhow!("TokenGroups"))).is_err());
        assert!(scegli(Ok(0), mai).is_err());
        assert!(scegli(Ok(4), mai).is_err());
    }

    #[test]
    fn il_token_abbassato_non_ha_administrators_attivo() {
        assert!(controlla_abbassato(Ok(false)).is_ok());
        assert!(controlla_abbassato(Ok(true)).is_err());
        assert!(controlla_abbassato(Err(anyhow!("TokenGroups"))).is_err());
    }

    #[test]
    fn il_token_ristretto_perde_i_privilegi() {
        assert_eq!(FLAG_RISTRETTO & LUA_TOKEN, LUA_TOKEN);
        assert_eq!(
            FLAG_RISTRETTO & DISABLE_MAX_PRIVILEGE,
            DISABLE_MAX_PRIVILEGE
        );
        // Niente token "restricted" o inerte: l'helper e le pipe restano quelli di oggi.
        assert_eq!(FLAG_RISTRETTO & (SANDBOX_INERT | WRITE_RESTRICTED), 0);
    }

    #[test]
    fn il_portatile_elevato_si_rifiuta() {
        assert!(portatile(Ok(false)).is_ok());
        assert!(portatile(Ok(true)).is_err());
        assert!(portatile(Err(anyhow!("OpenProcessToken"))).is_err());
    }

    #[test]
    fn deny_only_non_conta_come_attivo() {
        assert!(attivo(SE_GROUP_ENABLED | 0x1 | 0x2));
        assert!(!attivo(SE_GROUP_ENABLED | SE_GROUP_USE_FOR_DENY_ONLY));
        assert!(!attivo(SE_GROUP_USE_FOR_DENY_ONLY));
        assert!(!attivo(0));
    }
}
