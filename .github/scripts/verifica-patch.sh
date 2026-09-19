#!/usr/bin/env bash
# Verifica della patch Remotek sul client (REGOLE 6.2 del repo interno).
#
# Controlla che, dopo una MR o un merge upstream, il client sia ancora Remotek:
# nome, server, chiave, API e accesso presidiato in hbb_common; metadati
# Windows; link e attribuzione; lingua; tema generato; nessun trigger
# automatico nei workflow upstream; ogni file diverso dal tag upstream
# elencato in REMOTEK.md.
#
# Uso:   bash .github/scripts/verifica-patch.sh
# Esce con 1 se c'e' almeno un ERRORE. Gli AVVISI non fanno fallire; con
# VERIFICA_RIGIDA=1 (lo imposta la CI) un controllo che non si puo' eseguire
# (tag upstream assente, PyYAML assente) e' un errore, non un avviso.
#
# Non confronta server e chiave con l'installazione reale: quello lo fa
# strumenti/verifica-patch-client.sh del repo interno, che ha accesso ai valori.
# Servono solo bash, git, grep, sed e python3 (con PyYAML per i workflow).
set -u
cd "$(git rev-parse --show-toplevel)" || exit 1

errori=0
avvisi=0
ok()     { printf 'ok      %s\n' "$1"; }
errore() { printf 'ERRORE  %s\n' "$1"; errori=$((errori + 1)); }
avviso() { printf 'AVVISO  %s\n' "$1"; avvisi=$((avvisi + 1)); }

contiene() { # contiene <file> <regex estesa> <descrizione>
  if [ ! -f "$1" ]; then errore "$3: manca $1"; return; fi
  if grep -Eq -- "$2" "$1"; then ok "$3"; else errore "$3  [$1]"; fi
}
non_contiene() { # non_contiene <file> <regex estesa> <descrizione>
  if [ ! -f "$1" ]; then errore "$3: manca $1"; return; fi
  if grep -Eq -- "$2" "$1"; then errore "$3  [$1]"; else ok "$3"; fi
}

# --- 1. hbb_common: nome, server, chiave, API, auto-update, accesso presidiato -
CFG=libs/hbb_common/src/config.rs
CHIAVE_UPSTREAM='OeVuKk5nlHiXp\+APNn0Y3pC1Iwpwn44JGqrQCsWqmBw='
if [ ! -f "$CFG" ]; then
  errore "submodule libs/hbb_common non inizializzato (git submodule update --init libs/hbb_common)"
else
  contiene "$CFG" 'static ref APP_NAME: RwLock<String> = RwLock::new\("Remotek"' \
    'APP_NAME e'"'"' Remotek'
  contiene "$CFG" 'static ref PROD_RENDEZVOUS_SERVER: RwLock<String> = RwLock::new\("[^"]+"' \
    'PROD_RENDEZVOUS_SERVER non e'"'"' vuoto'
  contiene "$CFG" 'pub const RS_PUB_KEY: &str = "[A-Za-z0-9+/]{43}=";' \
    'RS_PUB_KEY e'"'"' una chiave di 32 byte in base64'
  non_contiene "$CFG" "RS_PUB_KEY: &str = \"$CHIAVE_UPSTREAM\"" \
    'RS_PUB_KEY non e'"'"' la chiave del server pubblico di RustDesk'
  contiene "$CFG" 'static ref DEFAULT_SETTINGS.*OPTION_API_SERVER\.to_owned\(\), "https://[^"]+"' \
    'API server di default impostato e in https (senza, il client ripiega su admin.rustdesk.com)'
  # OVERWRITE_SETTINGS si cerca dall'inizio della definizione e senza commenti:
  # a un merge, una nostra riga commentata accanto a quella upstream non basta.
  # Una sola definizione: la nostra dentro /* */ su righe proprie, o sotto un
  # #[cfg] che la esclude, lascerebbe compilare solo quella upstream.
  OVR='^[[:space:]]*pub static ref OVERWRITE_SETTINGS:'
  non_contiene "$CFG" "$OVR"'.*(//|/\*)' \
    'OVERWRITE_SETTINGS senza commenti nella riga della definizione'
  n_ovr=$(grep -Ec -- "$OVR" "$CFG")
  if [ "$n_ovr" = 1 ]; then
    ok 'OVERWRITE_SETTINGS definito una sola volta'
  else
    errore "OVERWRITE_SETTINGS definito $n_ovr volte, atteso 1 (una riga dentro /* */ o sotto #[cfg]?)  [$CFG]"
  fi
  contiene "$CFG" "$OVR"'.*OPTION_ALLOW_AUTO_UPDATE\.to_owned\(\), "N"' \
    'auto-update upstream spento in OVERWRITE_SETTINGS'
  contiene "$CFG" "$OVR"'.*OPTION_APPROVE_MODE\.to_owned\(\), "click"' \
    'accesso presidiato: approve-mode forzato a click in OVERWRITE_SETTINGS (senza, bastano ID e password)'
  contiene "$CFG" "$OVR"'.*\("2fa"\.to_owned\(\), ""\.to_owned\(\)\)' \
    '2FA spenta in OVERWRITE_SETTINGS (con click il codice aprirebbe la sessione senza clic)'

  server=$(sed -nE 's/.*static ref PROD_RENDEZVOUS_SERVER: RwLock<String> = RwLock::new\("([^"]*)".*/\1/p' "$CFG")
  elenco=$(sed -nE 's/^pub const RENDEZVOUS_SERVERS: &\[&str\] = &\["([^"]*)"\];.*/\1/p' "$CFG")
  api=$(sed -nE 's/.*OPTION_API_SERVER\.to_owned\(\), "https:\/\/([^"\/]*)".*/\1/p' "$CFG")
  if [ -n "$server" ] && [ "$server" = "$elenco" ] && [ "$server" = "$api" ]; then
    ok "server, elenco dei server e API puntano allo stesso host ($server)"
  else
    errore "host non coerenti: PROD_RENDEZVOUS_SERVER='$server' RENDEZVOUS_SERVERS='$elenco' API='$api'"
  fi
  case "$server$elenco$api" in
    *rustdesk*) errore "un host di RustDesk e' rimasto nella configurazione" ;;
    *) ok "nessun host di RustDesk nella configurazione" ;;
  esac
fi

# --- 2. Metadati Windows ------------------------------------------------------
RC=flutter/windows/runner/Runner.rc
contiene "$RC" 'VALUE "CompanyName", "Infinitek S\.r\.l\."' 'Runner.rc: CompanyName'
contiene "$RC" 'VALUE "ProductName", "Remotek"' 'Runner.rc: ProductName'
contiene "$RC" 'VALUE "FileDescription", "Remotek"' 'Runner.rc: FileDescription'
contiene "$RC" 'VALUE "OriginalFilename", "Remotek\.exe"' 'Runner.rc: OriginalFilename'
contiene "$RC" 'VALUE "LegalCopyright", ".*Infinitek S\.r\.l\..*RustDesk.*AGPL' \
  'Runner.rc: copyright con attribuzione a RustDesk e licenza'
contiene "$RC" 'VALUE "InternalName", "rustdesk"' \
  'Runner.rc: InternalName resta quello upstream (il nome del binario non si tocca)'
contiene libs/portable/Cargo.toml '^ProductName = "Remotek"' 'autoestraente: ProductName'
contiene libs/portable/Cargo.toml '^CompanyName = "Infinitek S\.r\.l\."' 'autoestraente: CompanyName'
contiene libs/portable/src/main.rs 'const APP_PREFIX: &str = "remotek";' \
  'autoestraente: cartella di estrazione separata da RustDesk'
contiene Cargo.toml '^name = "rustdesk"' 'Cargo.toml radice: il nome del pacchetto resta upstream'

# --- 3. Link e attribuzione ---------------------------------------------------
INFO=flutter/lib/desktop/pages/desktop_setting_page.dart
INST=flutter/lib/desktop/pages/install_page.dart
non_contiene "$INFO" 'rustdesk\.com' 'dialogo Informazioni: nessun link a rustdesk.com'
non_contiene "$INST" 'rustdesk\.com' 'dialogo di installazione: nessun link a rustdesk.com'
contiene "$INFO" 'Basato su RustDesk' 'dialogo Informazioni: attribuzione a RustDesk (AGPL 5)'
contiene "$INFO" 'github\.com/labinfinitek/remotek-client' \
  'dialogo Informazioni: link ai sorgenti (AGPL 6)'

# --- 4. Lingua ----------------------------------------------------------------
contiene src/common.rs 'OPTION_LANGUAGE\.to_owned\(\)' 'italiano di default (load_custom_client)'
contiene src/ui_interface.rs 'matches!\(a\.0, "it" \| "en"\)' 'selettore lingue: solo it ed en'

# --- 5. Chiave dei client personalizzati (custom.txt) --------------------------
# Finche' non esiste la MR `custom:` la chiave e' quella di RustDesk: nessuno
# di noi puo' firmare un custom.txt, ma un custom.txt firmato da RustDesk con
# override-settings sostituirebbe i default forzati di hbb_common, accesso
# presidiato compreso: senza cambiare il binario e' l'unico modo, va ricordato.
if grep -q '5Qbwsde3unUcJBtrx9ZkvUmwFNoExHzpryHuPUdqlWM=' src/common.rs 2>/dev/null; then
  avviso "read_custom_client usa ancora la chiave pubblica di RustDesk (MR custom: non ancora fatta): un custom.txt firmato da RustDesk scavalca i default forzati"
else
  ok "read_custom_client non usa la chiave pubblica di RustDesk"
fi

# --- 6. Tema generato -----------------------------------------------------------
if python3 brand/genera_tema.py --check >/dev/null 2>&1; then
  ok "flutter/lib/brand.g.dart e' aggiornato rispetto a brand/brand.toml"
else
  errore "flutter/lib/brand.g.dart non corrisponde a brand/brand.toml (python3 brand/genera_tema.py)"
fi

# --- 7. Nessun trigger automatico nei workflow ----------------------------------
# Un merge upstream puo' riportare cron, push e pull_request: un build dura
# circa un'ora e mezza e un tag upstream pubblicherebbe una release.
esito_wf=$(python3 - <<'PY'
import glob, sys
try:
    import yaml
except ImportError:
    print("SENZA-YAML"); sys.exit(0)
AMMESSI = {"workflow_dispatch", "workflow_call"}
NOSTRI = {".github/workflows/remotek-controlli.yml": AMMESSI | {"push", "pull_request"}}
for f in sorted(glob.glob(".github/workflows/*.y*ml")):
    with open(f, encoding="utf-8") as fh:
        d = yaml.safe_load(fh) or {}
    on = d.get("on", d.get(True))          # YAML 1.1 legge la chiave `on` come True
    trig = {on} if isinstance(on, str) else set(on or [])
    extra = trig - NOSTRI.get(f, AMMESSI)
    if extra:
        print(f"{f}: {', '.join(sorted(extra))}")
PY
)
if [ "$esito_wf" = "SENZA-YAML" ]; then
  msg="PyYAML non disponibile: trigger dei workflow non controllati"
  if [ "${VERIFICA_RIGIDA:-0}" = "1" ]; then errore "$msg"; else avviso "$msg"; fi
elif [ -n "$esito_wf" ]; then
  while IFS= read -r r; do errore "trigger automatico in $r"; done <<<"$esito_wf"
else
  ok "nessun workflow parte da solo (solo workflow_dispatch e workflow_call; remotek-controlli su push e PR)"
fi

# --- 8. Ogni file diverso da upstream e' elencato in REMOTEK.md -----------------
tag=$(sed -nE 's/^Base upstream: \*\*RustDesk [^*]+\*\* \(tag `([^`]+)`\).*/\1/p' REMOTEK.md | head -n 1)
base=""
for ref in "refs/tags/$tag" "refs/remotek-upstream/$tag"; do
  if [ -n "$tag" ] && git rev-parse -q --verify "$ref^{commit}" >/dev/null 2>&1; then base=$ref; break; fi
done
if [ -z "$tag" ]; then
  errore "REMOTEK.md non dichiara il tag upstream di base"
elif [ -z "$base" ]; then
  msg="tag upstream $tag non presente in questo clone: elenco dei file non controllato (git fetch --no-tags https://github.com/rustdesk/rustdesk refs/tags/$tag:refs/remotek-upstream/$tag)"
  if [ "${VERIFICA_RIGIDA:-0}" = "1" ]; then errore "$msg"; else avviso "$msg"; fi
else
  fuori=$(git diff --name-only "$base" HEAD | python3 -c '
import itertools, re, sys
dichiarati = set()
for riga in open("REMOTEK.md", encoding="utf-8"):
    if not riga.startswith("| `"):
        continue
    prima = riga.split(" | ")[0]
    for voce in re.findall(r"`([^`]+)`", prima):
        m = re.fullmatch(r"(.*)\{([^}]+)\}(.*)", voce)     # `a/{b,c}.yml`
        dichiarati.update([m.group(1) + x + m.group(3) for x in m.group(2).split(",")] if m else [voce])
for f in sys.stdin.read().split():
    if f not in dichiarati and not any(d.endswith("/") and f.startswith(d) for d in dichiarati):
        print(f)
')
  if [ -n "$fuori" ]; then
    while IFS= read -r f; do errore "$f e' diverso da $tag ma non e' elencato in REMOTEK.md"; done <<<"$fuori"
  else
    n=$(git diff --name-only "$base" HEAD | wc -l)
    ok "i $n file diversi da $tag sono tutti elencati in REMOTEK.md"
  fi
fi

printf '\nverifica-patch: %s errori, %s avvisi\n' "$errori" "$avvisi"
[ "$errori" -eq 0 ]
