//! `remotek-cli`: il codice sta in `librustdesk::remotek::cli` (solo Linux).

#[cfg(target_os = "linux")]
fn main() {
    std::process::exit(librustdesk::remotek::cli::main());
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("remotek-cli: solo Linux");
    std::process::exit(1);
}
