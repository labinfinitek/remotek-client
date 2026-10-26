//! `remotek-cli`: il codice sta in `librustdesk::remotek::cli` (Linux e
//! Windows x86_64).

#[cfg(any(target_os = "linux", all(windows, target_arch = "x86_64")))]
fn main() {
    std::process::exit(librustdesk::remotek::cli::main());
}

#[cfg(not(any(target_os = "linux", all(windows, target_arch = "x86_64"))))]
fn main() {
    eprintln!("remotek-cli: solo Linux e Windows x86_64");
    std::process::exit(1);
}
