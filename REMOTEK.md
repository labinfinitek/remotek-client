# Cosa cambia in questo fork rispetto a upstream

Base upstream: **RustDesk 1.4.9** (tag `1.4.9`). Ogni modifica e' elencata qui
con il motivo e la decisione che la giustifica (ADR nel repo interno
Infinitek). `git log 1.4.9..remotek` mostra la stessa lista come commit.

| File | Modifica | Motivo | ADR |
|---|---|---|---|
| `.gitmodules` | `libs/hbb_common` punta al fork `labinfinitek/remotek-hbb-common` | `APP_NAME`, server e chiave del server stanno li' | 0002 |
| `flutter/lib/desktop/pages/desktop_setting_page.dart` (dialogo Informazioni) | privacy e sito su `remotek.infinitek.it`; riga "Basato su RustDesk, AGPL-3.0 - sorgenti" con link al repo | marchio e attribuzione AGPL | 0002, REGOLE 13.2 |
| `flutter/lib/desktop/pages/install_page.dart` | link "End-user license agreement" su `remotek.infinitek.it/privacy.html` | marchio | 0002 |
| `.github/workflows/flutter-nightly.yml` | cron rimosso, solo `workflow_dispatch` | nessun build automatico | 0006, 0009 |
| `.github/workflows/{ci,flutter-ci,flutter-tag,fdroid}.yml` | blocco `on:` con solo `workflow_dispatch` | niente build su pull request, push o tag nel fork | 0006 |
| `SECURITY.md`, `NOTICE`, `REMOTEK.md`, `CHANGELOG-REMOTEK.md`, `.github/pull_request_template.md` | documenti del fork | licenza, sicurezza, tracciabilita' | REGOLE 13 |

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
