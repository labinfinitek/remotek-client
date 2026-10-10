#!/usr/bin/env bash
# Controprova della sez. 19 di verifica-patch.sh (remotek-cli: niente
# password del PC, mai amministratore). Su una copia di src/remotek/,
# src/remotek_cli.rs e dello script, ogni caso che il controllo deve vedere
# lo fa fallire; la copia intatta lo passa. Le altre sezioni falliscono nella
# copia (mancano i loro file) e non contano: si guarda solo la sez. 19.
#
# Uso:   bash .github/scripts/controprova-sez19.sh
# Esce con 1 se un caso passa il controllo o se la copia intatta non lo passa.
set -euo pipefail
radice="$(git rev-parse --show-toplevel)"
copia="$(mktemp -d)"
trap 'rm -rf "$copia"' EXIT

prepara() {
  rm -rf "$copia"/*  "$copia"/.git "$copia"/.github
  mkdir -p "$copia/src" "$copia/.github/scripts"
  cp -R "$radice/src/remotek" "$copia/src/"
  cp "$radice/src/remotek_cli.rs" "$copia/src/"
  cp "$radice/.github/scripts/verifica-patch.sh" "$copia/.github/scripts/"
  git -C "$copia" init -q
}

sez19() { # le righe della sez. 19 nell'uscita dello script sulla copia
  (cd "$copia" && bash .github/scripts/verifica-patch.sh 2>/dev/null || true) |
    grep -E '^ok +remotek-cli:|^ERRORE +(src/remotek|controllo di remotek-cli)' || true
}

guai=0
prepara
if sez19 | grep -q '^ok '; then
  echo "ok      copia intatta: la sez. 19 passa"
else
  echo "ERRORE  copia intatta: la sez. 19 non passa"; sez19; guai=1
fi

caso() { # caso <descrizione> <file> <righe da aggiungere in fondo>
  prepara
  printf '\n%s\n' "$3" >>"$copia/$2"
  if sez19 | grep -q 'spia_controprova'; then
    echo "ok      la sez. 19 vede: $1"
  else
    echo "ERRORE  la sez. 19 non vede: $1"; sez19; guai=1
  fi
}

caso '#[cfg(test)] su un use' src/remotek/firma.rs '#[cfg(test)]
use std::fmt;
fn spia_controprova() -> bool { std::env::var("X").is_ok() }'
caso 'una stringa che contiene //' src/remotek/firma.rs \
  'fn spia_controprova() -> bool { let _u = "https://h/x"; std::env::var("X").is_ok() }'
caso 'std::env::vars()' src/remotek/firma.rs \
  'fn spia_controprova() -> usize { std::env::vars().count() }'
caso 'src/remotek_cli.rs' src/remotek_cli.rs \
  'fn spia_controprova() -> bool { std::env::var("X").is_ok() }'

exit "$guai"
