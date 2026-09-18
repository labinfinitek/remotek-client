# Changelog Remotek (client)

Formato: Keep a Changelog 1.1.0, in italiano. Versioni: `remotek-<upstream>-<n>`.
Una riga per cambiamento visibile a chi usa o installa il client; la sezione
"Sicurezza" e' obbligatoria per ogni correzione di sicurezza.

## [Non rilasciato]
Base upstream: RustDesk 1.4.9.

### Modificato
- Proprieta' dei file Windows (exe interno e autoestraente): prodotto "Remotek", societa' "Infinitek S.r.l.", copyright con attribuzione a RustDesk; cartella di estrazione del portable separata da quella di RustDesk.
- Workflow upstream solo ad avvio manuale (nightly senza cron; ci, flutter-ci,
  flutter-tag, fdroid, wf-cliprdr-ci senza trigger automatici).
- I colori del tema vengono da `brand/brand.toml` (valori attuali identici a RustDesk 1.4.9).
- Submodule `libs/hbb_common` dal fork `labinfinitek/remotek-hbb-common`.
- Il client si chiama Remotek e nasce gia' configurato: server `remote.infinitek.it` con la sua chiave pubblica, API `https://remote.infinitek.it`; l'aggiornamento automatico verso RustDesk ufficiale e' spento e non riattivabile dalle impostazioni (hbb_common `2b42505`).
- Italiano di default; nel selettore restano italiano e inglese.

### Aggiunto
- Dialogo Informazioni: riga "Basato su RustDesk, AGPL-3.0 - sorgenti" con link al repo; privacy e sito puntano a `remotek.infinitek.it` (anche nel dialogo di installazione).
- `SECURITY.md`, `NOTICE`, `REMOTEK.md`, template di pull request.
- Workflow `remotek-controlli.yml` e script `verifica-patch.sh`: a ogni push e PR controllano che il client sia ancora Remotek (nome, server, chiave, API, metadati, link, lingua, tema), che nessun workflow upstream parta da solo e che ogni file diverso da upstream sia elencato in `REMOTEK.md`.
- Workflow `remotek-build.yml`: build manuale dell'exe Windows x86_64 come artefatto, con attestazione di provenienza; nessuna release pubblica.
