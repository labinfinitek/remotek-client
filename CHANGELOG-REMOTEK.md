# Changelog Remotek (client)

Formato: Keep a Changelog 1.1.0, in italiano. Versioni: `remotek-<upstream>-<n>`.
Una riga per cambiamento visibile a chi usa o installa il client; la sezione
"Sicurezza" e' obbligatoria per ogni correzione di sicurezza.

## [Non rilasciato]
Base upstream: RustDesk 1.4.9.

### Modificato
- Proprieta' dei file Windows (exe interno e autoestraente): prodotto "Remotek", societa' "Infinitek S.r.l.", copyright con attribuzione a RustDesk; cartella di estrazione del portable separata da quella di RustDesk.
- Workflow upstream solo ad avvio manuale (nightly senza cron; ci, flutter-ci,
  flutter-tag, fdroid senza trigger automatici).
- Submodule `libs/hbb_common` dal fork `labinfinitek/remotek-hbb-common`.

### Aggiunto
- `SECURITY.md`, `NOTICE`, `REMOTEK.md`, template di pull request.
- Workflow `remotek-build.yml`: build manuale dell'exe Windows x86_64 come artefatto, con attestazione di provenienza; nessuna release pubblica.
