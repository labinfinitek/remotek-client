# Changelog Remotek (client)

Formato: Keep a Changelog 1.1.0, in italiano. Versioni: `remotek-<upstream>-<n>`.
Una riga per cambiamento visibile a chi usa o installa il client; la sezione
"Sicurezza" e' obbligatoria per ogni correzione di sicurezza.

## [Non rilasciato]
Base upstream: RustDesk 1.4.9.

### Modificato
- Workflow upstream solo ad avvio manuale (nightly senza cron; ci, flutter-ci,
  flutter-tag, fdroid senza trigger automatici).
- I colori del tema vengono da `brand/brand.toml` (valori attuali identici a RustDesk 1.4.9).
- Submodule `libs/hbb_common` dal fork `labinfinitek/remotek-hbb-common`.

### Aggiunto
- `SECURITY.md`, `NOTICE`, `REMOTEK.md`, template di pull request.
