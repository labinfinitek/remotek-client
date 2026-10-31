//! Codice del fork Remotek che non sta in upstream (REMOTEK.md).

#[cfg(any(target_os = "linux", all(windows, target_arch = "x86_64")))]
pub mod cli;
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod exec;
pub mod firma;
pub mod lasciapassare;
pub mod non_presidiato;
pub mod scelta_terminale;
pub mod stampante;
pub mod token_terminale;
pub mod trascrizione;
pub mod uscita_terminale;
pub mod vista_cm;
