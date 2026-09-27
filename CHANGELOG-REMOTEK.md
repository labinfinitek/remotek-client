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
- I colori del tema vengono da `brand/brand.toml`: accento Blu Infinitek `#1F6BFF`, pulsanti `#2B7FFF`, sfondo del tema chiaro `#EEF3FB` e riquadro della qualita' di connessione `#060A14` dalla palette del sito; bordi e testo secondario restano quelli di RustDesk 1.4.9, piu' leggibili su fondo chiaro.
- Submodule `libs/hbb_common` dal fork `labinfinitek/remotek-hbb-common`.
- Il client si chiama Remotek e nasce gia' configurato: server `remote.infinitek.it` con la sua chiave pubblica, API `https://remote.infinitek.it`; l'aggiornamento automatico verso RustDesk ufficiale e' spento e non riattivabile dalle impostazioni (hbb_common `2b42505`).
- Italiano di default; nel selettore restano italiano e inglese.
- Icone Infinitek al posto di quelle di RustDesk su Windows: exe da scaricare e installato, finestra, barra delle applicazioni, area di notifica, connection manager.
- Voce Disinstalla di Windows (App installate): l'editore e' "Infinitek S.r.l." e non piu' "Remotek", anche dopo l'aggiornamento di un'installazione precedente.
- Dialogo Informazioni: il riquadro del copyright dice "Copyright © 2026 Infinitek S.r.l." seguito dall'attribuzione a RustDesk e a Purslane Tech Pte. Ltd., come le proprieta' dell'exe; tolto lo slogan di RustDesk.

### Aggiunto
- Logo Infinitek nella home: simbolo e scritta nel tema scuro, solo il simbolo nel tema chiaro.
- Dialogo Informazioni: riga "Basato su RustDesk, AGPL-3.0 - sorgenti" con link al repo; privacy e sito puntano a `remotek.infinitek.it` (anche nel dialogo di installazione).
- `SECURITY.md`, `NOTICE`, `REMOTEK.md`, template di pull request.
- Workflow `remotek-controlli.yml` e script `verifica-patch.sh`: a ogni push e PR controllano che il client sia ancora Remotek (nome, server, chiave, API, accesso presidiato, metadati ed editore, link e copyright, lingua, chiave che verifica `custom.txt`, tema, nessuna configurazione dal nome del file, nessuna chiamata automatica ai server RustDesk), che nessun workflow upstream parta da solo e che ogni file diverso da upstream sia elencato in `REMOTEK.md`.
- Workflow `remotek-build.yml`: build manuale dell'exe Windows x86_64 come artefatto, con attestazione di provenienza; nessuna release pubblica.

### Corretto
- Lingua: tolta dal selettore la voce "Predefinita", che seguiva la lingua di Windows: sceglierla, su un Windows francese, tedesco o spagnolo, faceva parlare al client quella lingua. Restano italiano, che vale se non si sceglie nulla, e inglese.
- Installazione su Windows: il pacchetto contiene `Remotek.exe` e non piu' `rustdesk.exe`, cosi' servizio, collegamenti, disinstallazione e "e' installato?" trovano il programma in `C:\Program Files\Remotek`.

### Sicurezza
- Accesso presidiato di default: ogni sessione in entrata va accettata con un clic sul PC controllato; ID e password (monouso o permanente) da soli non bastano piu'. L'impostazione e' bloccata: non si cambia da impostazioni, riga di comando o API. La verifica in due passaggi (2FA) e' spenta, perche' con l'accettazione a clic non aggiunge protezione e il suo codice aprirebbe la sessione senza clic: l'interruttore nelle impostazioni non ha effetto. L'accesso non presidiato si abilitera' solo per singolo cliente con un `custom.txt` firmato; anche con un `custom.txt` la 2FA resta spenta, in ogni modalita'. L'accettazione a clic la puo' togliere solo un `custom.txt` firmato da Infinitek (riga seguente; hbb_common `6d59c29`).
- `custom.txt` vale solo se firmato da Infinitek: il client ne verifica la firma con la chiave pubblica Remotek al posto di quella di RustDesk. Da questo eseguibile in poi un `custom.txt` firmato da RustDesk non viene piu' applicato — nessuno ne ha uno e il nostro flusso non li usa — e valgono solo i nostri; un file senza firma valida resta ignorato in silenzio e restano i default del binario. E' il primo passo dell'accesso non presidiato per singolo cliente, che resta indisponibile finche' non c'e' lo strumento di firma. Un `custom.txt` firmato vale pero' su qualunque installazione Remotek, non solo su quella del cliente per cui e' stato emesso: il client ne verifica la firma e nient'altro, non scade e non si revoca; per toglierlo di mezzo servono una chiave nuova e un eseguibile nuovo per tutti. Va trattato come una password: si consegna al cliente e non si lascia in giro.
- Il nome del file non cambia piu' server, chiave, API o relay: un exe rinominato con `host=`, `key=`, `api=` o `relay=` (o con la configurazione codificata in base64 nel nome, anche non firmata) usa comunque il server e la chiave di Remotek, anche come portable. Restano gli usi del nome che non toccano il server: supporto rapido (`-qs`) e `install.exe`.
