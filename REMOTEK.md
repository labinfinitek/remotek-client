# Cosa cambia in questo fork rispetto a upstream

Base upstream: **RustDesk 1.4.9** (tag `1.4.9`). Ogni modifica e' elencata qui
con il motivo e la decisione che la giustifica (ADR nel repo interno
Infinitek). `git log 1.4.9..remotek` mostra la stessa lista come commit.

| File | Modifica | Motivo | ADR |
|---|---|---|---|
| `.github/workflows/remotek-build.yml` | file nuovo: build manuale Windows x86_64 (job upstream copiato alla lettera, bridge e TopMostWindow riusati), exe come artefatto, nessuna release, firma predisposta e spenta, attestazione di provenienza | il workflow upstream pubblica l'exe solo in una pre-release pubblica di tutte le piattaforme | 0009 |
| `.gitmodules` | `libs/hbb_common` punta al fork `labinfinitek/remotek-hbb-common` | `APP_NAME`, server e chiave del server stanno li' | 0002 |
| `.github/workflows/flutter-nightly.yml` | cron rimosso, solo `workflow_dispatch` | nessun build automatico | 0006, 0009 |
| `brand/brand.toml`, `brand/genera_tema.py`, `flutter/lib/brand.g.dart`, `flutter/lib/common.dart` (`MyTheme` e un import) | le 12 costanti colore di `MyTheme` vengono da `brand.g.dart`, generato da `brand.toml` e committato | colori del brand in un solo file; i valori attuali sono quelli upstream | 0004 |
| `.github/workflows/{ci,flutter-ci,flutter-tag,fdroid}.yml` | blocco `on:` con solo `workflow_dispatch` | niente build su pull request, push o tag nel fork | 0006 |
| `SECURITY.md`, `NOTICE`, `REMOTEK.md`, `CHANGELOG-REMOTEK.md`, `.github/pull_request_template.md` | documenti del fork | licenza, sicurezza, tracciabilita' | REGOLE 13 |
| `flutter/windows/runner/Runner.rc` | `CompanyName`, `FileDescription`, `LegalCopyright`, `OriginalFilename`, `ProductName` (righe 92-98) | proprieta' del file e nome in Gestione attivita' dell'exe installato; sono risorse statiche e non possono leggere `APP_NAME` | 0002 |
| `libs/portable/Cargo.toml` | blocco `[package.metadata.winres]` (mai la riga `version`, che e' del bump) | metadati dell'exe autoestraente distribuito | 0002 |
| `libs/portable/src/main.rs` | `APP_PREFIX` = `remotek` | cartella di estrazione separata da quella di RustDesk portable: sullo stesso PC si cancellerebbero a vicenda | 0002 |

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
