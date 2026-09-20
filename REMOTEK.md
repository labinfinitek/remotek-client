# Cosa cambia in questo fork rispetto a upstream

Base upstream: **RustDesk 1.4.9** (tag `1.4.9`). Ogni modifica e' elencata qui
con il motivo e la decisione che la giustifica (ADR nel repo interno
Infinitek). `git log 1.4.9..remotek` mostra la stessa lista come commit.

| File | Modifica | Motivo | ADR |
|---|---|---|---|
| `.github/workflows/remotek-build.yml` | file nuovo: build manuale Windows x86_64 (job upstream copiato alla lettera, bridge e TopMostWindow riusati), exe come artefatto, nessuna release, firma predisposta e spenta, attestazione di provenienza; l'exe dell'applicazione nel pacchetto si chiama `<APP_NAME>.exe` (non `rustdesk.exe`), con due guardie che fermano il build altrimenti | il workflow upstream pubblica l'exe solo in una pre-release pubblica di tutte le piattaforme; l'installazione copia la cartella senza rinominare, mentre servizio, collegamenti e disinstallazione cercano `<APP_NAME>.exe` | 0009, 0002 |
| `.github/workflows/remotek-controlli.yml`, `.github/scripts/verifica-patch.sh`, `.gitleaks.toml`, `.github/zizmor.yml` | file nuovi: controlli del fork a ogni push e PR verso `remotek` (segreti, verifica della patch, tema, audit dei workflow); minuti, nessun build | la patch si verifica, non si ricorda; e' il check richiesto dal ruleset | 0012, REGOLE 6.2 e 7 |
| `.gitmodules`, `libs/hbb_common` (puntatore) | `libs/hbb_common` punta al fork `labinfinitek/remotek-hbb-common` | `APP_NAME`, server, chiave del server, API e i default forzati in `OVERWRITE_SETTINGS` stanno li': auto-update spento, accesso presidiato (`approve-mode` = `click`, ogni sessione in entrata si accetta con un clic), 2FA spenta (con `click` sarebbe un secondo modo di entrare senza clic), i quattro default di privacy spenti (`enable-lan-discovery`, `enable-record-session`, `enable-privacy-mode`, `enable-camera` = `N`), `allow-auto-record-incoming` = `N`, che e' la seconda strada della registrazione e non passa dai permessi (`src/server/video_service.rs:578-581`, `:1042-1071`), e `access-mode` vuota, senza la quale il preset "Accesso completo" ne riaccenderebbe tre (`src/server/connection.rs:2370-2385`); la cattura schermata non e' coperta da nessun default e resta possibile | 0002, 0008, 0016, 0017 |
| `flutter/lib/desktop/pages/desktop_setting_page.dart` (dialogo Informazioni) | privacy e sito su `remotek.infinitek.it`; riga "Basato su RustDesk, AGPL-3.0 - sorgenti" con link al repo; nel riquadro del copyright "Copyright © 2026 Infinitek S.r.l." e "Basato su RustDesk, Copyright © <anno> Purslane Tech Pte. Ltd." (anno calcolato come upstream) al posto della sola riga di Purslane e dello slogan di RustDesk | marchio e attribuzione AGPL; lo stesso testo di `LegalCopyright` nelle proprieta' dell'exe | 0002, REGOLE 13.2 |
| `flutter/lib/desktop/pages/install_page.dart` | link "End-user license agreement" su `remotek.infinitek.it/privacy.html` | marchio | 0002 |
| `.github/workflows/flutter-nightly.yml` | cron rimosso, solo `workflow_dispatch` | nessun build automatico | 0006, 0009 |
| `brand/brand.toml`, `brand/genera_tema.py`, `flutter/lib/brand.g.dart`, `flutter/lib/common.dart` (`MyTheme` e un import) | le 12 costanti colore di `MyTheme` vengono da `brand.g.dart`, generato da `brand.toml` e committato | colori del brand in un solo file; accento, pulsanti, sfondo del tema chiaro e `canvasColor` dalla palette Infinitek, le altre costanti upstream (motivo accanto a ciascuna in `brand.toml`) | 0004 |
| `brand/genera_icone.py`, `brand/sorgenti/`, `res/icon.ico`, `res/tray-icon.ico`, `flutter/windows/runner/resources/app_icon.ico`, `flutter/assets/icon.png`, `flutter/assets/2.0x/icon.png`, `flutter/assets/logo_light.png`, `flutter/assets/logo_dark.png` | icone Windows (exe autoestraente, exe installato, finestra, barra delle applicazioni, area di notifica, connection manager) e logo della home, generati dal marchio Infinitek con `brand/genera_icone.py`; `flutter/assets/icon.svg` resta upstream (si usa solo se manca `icon.png`) | il logo di RustDesk non si usa come logo del prodotto; icone macOS, iOS, Android e Linux restano upstream: non si costruiscono e richiedono sorgenti a 1024 px | 0002, REGOLE 13.2 |
| `.github/workflows/{ci,flutter-ci,flutter-tag,fdroid,wf-cliprdr-ci}.yml` | blocco `on:` con solo `workflow_dispatch` | niente build su pull request, push o tag nel fork | 0006 |
| `src/common.rs` (`load_custom_client`) | `lang = it` in `DEFAULT_LOCAL_SETTINGS` se non gia' impostato | italiano di default anche senza `custom.txt`; l'utente puo' scegliere l'inglese | 0011 |
| `src/common.rs` (`read_custom_client`) | la chiave pubblica cablata che verifica la firma di `custom.txt` e' la nostra (ed25519), non quella di RustDesk | le impostazioni per cliente valgono solo se le firmiamo noi: `override-settings` scrive nello stesso `OVERWRITE_SETTINGS` dei default forzati di hbb_common e vince, accesso presidiato compreso. Con la chiave di RustDesk non potevamo firmare nessun file e un file firmato da loro sarebbe stato applicato. La chiave privata non sta in nessun repo | 0003 |
| `src/ui_interface.rs` (`get_langs`) | filtro dell'elenco lingue a `it` e `en` | solo italiano e inglese selezionabili (A4); traduzioni e `src/lang.rs` intatti | 0011 |
| `flutter/lib/desktop/pages/desktop_setting_page.dart` (`language`) | tolta la voce "Predefinita" dal selettore della lingua | "Predefinita" salvava `lang` vuoto, che `src/lang.rs` risolve con la lingua di Windows: su un Windows francese, tedesco o spagnolo il client parlava quella lingua (A4); senza la voce restano italiano, che vale se non si sceglie nulla, e inglese | 0011 |
| `src/platform/windows.rs` (`get_license_from_exe_name` e un import) | restituisce sempre un errore: server, chiave, API e relay non si leggono piu' dal nome del file (`host=`, `key=`, `api=`, `relay=` o la configurazione codificata in base64, anche non firmata) ne' da `RUSTDESK_APPNAME` del portable; tolto l'import rimasto inutilizzato | un exe col nostro marchio rinominato andrebbe su un server altrui; un solo punto spegne tutti i chiamanti (avvio, server, chiave, API, installazione, dialogo Informazioni); `-qs`, `install.exe` e `custom.txt` non passano di qui | 0015, 0008, 0002 |
| `SECURITY.md`, `NOTICE`, `REMOTEK.md`, `CHANGELOG-REMOTEK.md`, `.github/pull_request_template.md` | documenti del fork | licenza, sicurezza, tracciabilita' | REGOLE 13 |
| `flutter/windows/runner/Runner.rc` | `CompanyName`, `FileDescription`, `LegalCopyright`, `OriginalFilename`, `ProductName` (righe 92-98) | proprieta' del file e nome in Gestione attivita' dell'exe installato; sono risorse statiche e non possono leggere `APP_NAME` | 0002 |
| `libs/portable/Cargo.toml` | blocco `[package.metadata.winres]` (mai la riga `version`, che e' del bump) | metadati dell'exe autoestraente distribuito | 0002 |
| `libs/portable/src/main.rs` | `APP_PREFIX` = `remotek` | cartella di estrazione separata da quella di RustDesk portable: sullo stesso PC si cancellerebbero a vicenda | 0002 |
| `src/platform/windows.rs` (`install_me` e `update_me`, valore `Publisher`) | l'editore della voce Disinstalla e' `Infinitek S.r.l.`, non il nome dell'app, in tutte e due le funzioni che scrivono la voce | in App installate di Windows l'editore e' chi pubblica il programma, come `CompanyName` delle risorse; la riga di comando dell'installazione e' testo, non legge le risorse. Installazione, "Click to upgrade" e installazione sopra una versione precedente passano da `install_me`, che cancella la chiave e la riscrive; `update_me` si raggiunge solo con `--update` (aggiornamento automatico e aggiornamento scaricato, spenti in Remotek, oppure a mano) e la riga lo tiene allineato a `install_me`, come chiede il commento upstream in `install_me` | 0002 |

Cosa **non** cambia: protocollo, codec, cattura schermo, traduzioni, campi di
versione, i link alla documentazione upstream (`doc_*` in `src/lang/en.rs`,
`src/client.rs`, `LINK_DOCS_*` in hbb_common). Il numero di versione dentro
l'app e' quello upstream; il rilascio Remotek si identifica con il tag
`remotek-<upstream>-<n>`.

Licenza: AGPL-3.0, vedi `LICENCE` e `NOTICE`. Segnalazioni di sicurezza:
`SECURITY.md`.

---

# What this fork changes (English)
Upstream base: **RustDesk 1.4.9**. Every change is listed above with its
reason. Protocol, codecs, screen capture, translations and version fields are
untouched. Licence: see `LICENCE` and `NOTICE`; security: `SECURITY.md`.
