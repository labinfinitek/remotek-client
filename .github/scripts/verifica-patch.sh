#!/usr/bin/env bash
# Verifica della patch Remotek sul client (REGOLE 6.2 del repo interno).
#
# Controlla che, dopo una MR o un merge upstream, il client sia ancora Remotek:
# nome, server, chiave, API e accesso presidiato in hbb_common; metadati
# Windows; link e attribuzione; lingua; tema generato; nessun trigger
# automatico nei workflow upstream; ogni file diverso dal tag upstream
# elencato in REMOTEK.md; server e chiave mai dal nome del file; nessuna
# chiamata automatica ai server RustDesk.
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
# La voce "Predefinita" salva lang vuoto, che src/lang.rs risolve con la lingua
# di Windows: con tutte le traduzioni nel binario si aggirerebbe A4. Non viene
# da get_langs ma da language() della pagina desktop: li' defaultOptionLang
# resta solo nel ripiego upstream per una chiave fuori elenco, e le liste
# keys/values restano quelle di get_langs: una voce aggiunta in altro modo
# (costante, letterale) sarebbe la stessa Predefinita. Se python3 fallisce il
# controllo non e' eseguito, e questo e' un errore, non un ok.
esito_pred=$(python3 - "$INFO" <<'PY'
import re, sys
try:
    testo = open(sys.argv[1], encoding="utf-8").read()
except OSError:
    print("file mancante"); sys.exit(0)
corpi = re.findall(r"^  Widget language\(\) \{\n(.*?)^  \}$", testo, re.M | re.S)
if len(corpi) != 1:
    print(f"language() trovata {len(corpi)} volte, attesa 1: rileggere il selettore"); sys.exit(0)
righe = [r.strip() for r in corpi[0].splitlines() if not r.strip().startswith("//")]
dichiarazioni = ("List<String> keys = langsMap.keys.toList();",
                 "List<String> values = langsMap.values.toList();")
modifiche = [r for r in righe if not r.startswith(dichiarazioni) and re.search(
    r"\b(keys|values)\s*(\.(insert|insertAll|add|addAll)\s*\(|\+?=(?!=))", r)]
if any("'Default'" in r or '"Default"' in r for r in righe):
    print("il selettore lingue mostra di nuovo la voce Default (Predefinita)")
elif [r for r in righe if "defaultOptionLang" in r] != ["currentKey = defaultOptionLang;"]:
    print("defaultOptionLang usato nel selettore lingue oltre al ripiego: la voce Predefinita e' tornata?")
elif modifiche:
    print(f"elenco lingue cambiato in language() ({modifiche[0]}): la voce Predefinita e' tornata?")
PY
) || esito_pred="controllo della voce Predefinita non eseguito: python3 terminato con errore"
if [ -z "$esito_pred" ]; then
  ok 'selettore lingue: nessuna voce "Predefinita" (seguirebbe la lingua di Windows)'
else
  errore "$esito_pred  [$INFO]"
fi

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

# --- 9. Server e chiave mai dal nome del file -----------------------------------
# Un exe rinominato "...host=<server>,key=<chiave>.exe" (anche il portable, che
# passa il nome esterno in RUSTDESK_APPNAME) andrebbe su un server altrui col
# nostro marchio. Tutti gli usi passano da get_license_from_exe_name, che deve
# restituire solo un errore; a un merge upstream una nuova chiamata al parser
# o una nuova lettura di RUSTDESK_APPNAME riaprirebbe la strada senza conflitti.
WIN=src/platform/windows.rs
esito_nome=$(python3 - "$WIN" <<'PY'
import re, sys
try:
    testo = open(sys.argv[1], encoding="utf-8").read()
except OSError:
    print("file mancante"); sys.exit(0)
n = len(re.findall(r"\bfn get_license_from_exe_name\b", testo))
if n != 1:
    print(f"get_license_from_exe_name definita {n} volte, attesa 1"); sys.exit(0)
m = re.search(r"\bfn get_license_from_exe_name\(\)[^{]*\{\n(.*?)^\}", testo, re.M | re.S)
corpo = [r.strip() for r in (m.group(1) if m else "").splitlines()]
corpo = [r for r in corpo if r and not r.startswith("//")]
if len(corpo) != 1 or not re.fullmatch(r'bail!\("[^"]*"\);?', corpo[0]):
    print("get_license_from_exe_name non restituisce solo un errore: il nome del file torna a decidere server e chiave")
PY
)
if [ -n "$esito_nome" ]; then
  errore "$esito_nome  [$WIN]"
else
  ok "get_license_from_exe_name restituisce solo un errore: server e chiave mai dal nome del file"
fi
# Fuori dal parser (custom_server.rs) e dallo strumento naming.rs e' ammessa
# una sola chiamata: "--config <stringa>" di core_main.rs, che non legge il nome.
chiamate=$(grep -rnE --include='*.rs' 'get_custom_server_from_string[[:space:]]*\(' src |
  grep -vE '^src/(custom_server|naming)\.rs:' | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//')
n_cfg=$(printf '%s\n' "$chiamate" | grep -c '^src/core_main\.rs:')
altre=$(printf '%s\n' "$chiamate" | grep -v '^src/core_main\.rs:' | grep .)
if [ -z "$altre" ] && [ "$n_cfg" -le 1 ]; then
  ok "il parser del nome del file e' chiamato al piu' da --config (core_main.rs)"
else
  while IFS= read -r r; do
    [ -n "$r" ] && errore "chiamata non prevista al parser del nome del file in $(cut -d: -f1-2 <<<"$r"): rileggere"
  done <<<"$altre"
  [ "$n_cfg" -le 1 ] || errore "core_main.rs chiama il parser del nome del file $n_cfg volte, attesa 1 (--config)"
fi
# Il controllo qui sopra scarta custom_server.rs: il parser deve quindi restare
# una funzione pura. Fuori dal file si vede solo get_custom_server_from_string
# e nessuna riga legge il nome dell'exe o l'ambiente; altrimenti un helper nuovo
# li' (current_exe() + parser) chiamato da common.rs sfuggirebbe.
CS=src/custom_server.rs
esito_parser=$(python3 - "$CS" <<'PY'
import re, sys
try:
    testo = open(sys.argv[1], encoding="utf-8").read()
except OSError:
    print("file mancante"); sys.exit(0)
testo = re.sub(r"/\*.*?\*/", "", testo, flags=re.S)
righe = [r for r in testo.splitlines() if not r.strip().startswith("//")]
pub = re.findall(r'^[ \t]*pub(?:\([^)]*\))?[ \t]+(?:(?:const|async|unsafe|extern(?:[ \t]+"[^"]*")?)[ \t]+)*fn[ \t]+(\w+)',
                 "\n".join(righe), re.M)
if pub != ["get_custom_server_from_string"]:
    print("funzioni pubbliche: " + (", ".join(pub) or "nessuna") + "; attesa solo get_custom_server_from_string")
env = [r.strip() for r in righe if re.search(r"current_exe|\benv::", r)]
if env:
    print("il parser legge il nome dell'exe o l'ambiente: " + env[0])
PY
)
if [ -z "$esito_parser" ]; then
  ok "il parser del nome del file resta puro (solo get_custom_server_from_string, niente exe ne' ambiente)"
else
  while IFS= read -r r; do
    errore "$r  [$CS]"
  done <<<"$esito_parser"
fi
lettori=$(grep -rnE --include='*.rs' 'PORTABLE_APPNAME_RUNTIME_ENV_KEY|RUSTDESK_APPNAME' src |
  grep -vE '^[^:]+:[0-9]+:[[:space:]]*//' |
  grep -vE '^src/common\.rs:[0-9]+:pub const PORTABLE_APPNAME_RUNTIME_ENV_KEY: &str = "RUSTDESK_APPNAME";$')
if [ -z "$lettori" ]; then
  ok "nessuno legge RUSTDESK_APPNAME (il nome del file del portable)"
else
  while IFS= read -r r; do
    errore "RUSTDESK_APPNAME letto in $(cut -d: -f1-2 <<<"$r"): se ne ricava server o chiave va spento"
  done <<<"$lettori"
fi

# --- 10. Silenzio verso i server RustDesk ---------------------------------------
# Il client non chiama RustDesk da solo, ma non perche' abbiamo tolto codice
# (patch minima): il silenzio poggia su codice upstream che non tocchiamo, e un
# merge che lo cambi non da' conflitti. I punti:
#  - il controllo versione (version_check_request, api.rustdesk.com) passa solo
#    da do_check_software_update; la chiamano check_software_update, che esce
#    per is_custom_client() (vero perche' APP_NAME non e' RustDesk), e
#    check_update dell'updater, che esce se allow-auto-update non e' Y (N in
#    OVERWRITE_SETTINGS, che Config::get_option legge per primo) e il controllo
#    non e' manuale (manually_check_update non ha chiamanti). Dopo, l'updater
#    scaricherebbe ed eseguirebbe l'installer di RustDesk;
#  - il ripiego admin.rustdesk.com di get_api_server_ vale solo senza server:
#    get_custom_rendezvous_server restituisce PROD_RENDEZVOUS_SERVER (sezione 1);
#  - nessun altro host *.rustdesk.com nel codice Rust, fuori da commenti e test,
#    ne' nel Dart di flutter/lib, fuori dalle righe di commento (i link a
#    rustdesk.com/docs e al sito non sono server e non si contano).
# Si legge il sorgente senza commenti e con le stringhe separate dal codice, non
# per righe. Una guardia conta solo se e' un'istruzione del corpo della sua
# funzione, fuori da blocchi e senza un attributo davanti: un #[cfg] la
# toglierebbe da qualche build, come se non ci fosse. Un ERRORE chiede di
# rileggere il punto indicato e poi aggiornare il controllo; se python3
# fallisce il controllo non e' eseguito ed e' un errore.
esito_sil=$(python3 - <<'PY'
import os, re, sys

def vuoto(s):
    return re.sub(r"[^\n]", " ", s)

TOKEN = re.compile(r"""//|/\*|(?<!\w)b?r#*"|"|'""")
CARATTERE = re.compile(r"""'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]{1,6}\}|.)|[^\\'\n])'""")

def spoglia(t):
    # (codice, maschera), lunghi come t e con gli stessi a capo: in codice i
    # commenti sono spazi, in maschera anche l'interno di stringhe e caratteri.
    cod, mas, i, n = [], [], 0, len(t)
    while i < n:
        m = TOKEN.search(t, i)
        a = m.start() if m else n
        cod.append(t[i:a]); mas.append(t[i:a])
        if not m:
            break
        g = m.group()
        if g in ("//", "/*"):
            if g == "//":
                j = t.find("\n", a)
                j = n if j < 0 else j
            else:                                   # i /* */ si annidano
                p, j = 1, a + 2
                while j < n and p:
                    if t.startswith("/*", j): p, j = p + 1, j + 2
                    elif t.startswith("*/", j): p, j = p - 1, j + 2
                    else: j += 1
            cod.append(vuoto(t[a:j])); mas.append(vuoto(t[a:j]))
        elif g == "'":                              # carattere o lifetime
            c = CARATTERE.match(t, a)
            j = c.end() if c else a + 1
            cod.append(t[a:j]); mas.append("'" + vuoto(t[a + 1:j - 1]) + "'" if c else "'")
        else:                                       # stringa, anche r#"..."#
            chiusa = '"' + "#" * g.count("#")
            if "r" in g:
                k = t.find(chiusa, m.end())
                k = n if k < 0 else k
            else:
                k = m.end()
                while k < n and t[k] != '"':
                    k += 2 if t[k] == "\\" else 1
                k = min(k, n)
            j = min(k + len(chiusa), n)
            cod.append(t[a:j]); mas.append(g + vuoto(t[m.end():k]) + t[k:j])
        i = j
    return "".join(cod), "".join(mas)

GRAFFE = re.compile(r"[{}]")
def chiudi(mas, a):
    p = 0
    for g in GRAFFE.finditer(mas, a):
        p += 1 if g.group() == "{" else -1
        if p == 0:
            return g.start()
    return len(mas)

def funzioni(mas):
    # [(nome, inizio, fine)] del corpo di ogni fn che ne ha uno
    out = []
    for m in re.finditer(r"\bfn\s+(\w+)", mas):
        j, p = m.end(), 0
        while j < len(mas) and not (p == 0 and mas[j] in "{;"):
            p += {"(": 1, "[": 1, ")": -1, "]": -1}.get(mas[j], 0)
            j += 1
        if j < len(mas) and mas[j] == "{":
            out.append((m.group(1), j + 1, chiudi(mas, j)))
    return out

def sorgenti():
    for radice in ("src", "libs"):
        for d, sotto, nomi in os.walk(radice):
            sotto[:] = sorted(x for x in sotto if x not in ("target", ".git"))
            for f in sorted(nomi):
                if f.endswith(".rs"):
                    yield os.path.join(d, f)
TESTI = {}
for f in sorgenti():
    with open(f, encoding="utf-8", errors="replace") as fh:
        TESTI[f] = fh.read()
C, U = "src/common.rs", "src/updater.rs"
H, L = "libs/hbb_common/src/config.rs", "libs/hbb_common/src/lib.rs"
mancanti = [f for f in (C, U, H, L) if f not in TESTI]
if mancanti:                    # senza questi file ogni "nessun uso" sarebbe falso
    print("ERRORE\tmancano " + ", ".join(mancanti) + ": silenzio verso RustDesk non controllato")
    sys.exit(0)
ANALISI = {}
def analisi(f):
    if f not in ANALISI:
        cod, mas = spoglia(TESTI[f])
        ANALISI[f] = (cod, mas, funzioni(mas))
    return ANALISI[f]
def riga(s, pos):
    return s.count("\n", 0, pos) + 1
def dentro(fns, pos):                               # la fn piu' interna
    c = [(a, n) for n, a, b in fns if a <= pos < b]
    return max(c)[1] if c else None
def compatto(s):
    return re.sub(r"\s+", "", s)
def esito(buono, bene, male):
    print(("ok\t" + bene) if buono else ("ERRORE\t" + male))

def unica(f, nome, dove=None):
    # (riga, corpo, corpo in maschera) della sola fn `nome` in f (o nei tratti
    # `dove`), o None
    cod, mas, fns = analisi(f)
    c = [(a, b) for n, a, b in fns if n == nome and (dove is None or any(x <= a < y for x, y in dove))]
    if len(c) != 1:
        print("ERRORE\t%s definita %d volte in %s, attesa 1: rileggere" % (nome, len(c), f))
        return None
    return riga(mas, c[0][0]), cod[c[0][0]:c[0][1]], mas[c[0][0]:c[0][1]]

def in_testa(k, a):
    # vero se in k (corpo compattato della maschera: le graffe sono solo
    # codice) la posizione a apre un'istruzione del corpo: fuori da blocchi e
    # dopo `;` o `}`, non dopo un attributo come #[cfg(...)]
    p = k[:a]
    return p.count("{") == p.count("}") and (p == "" or p[-1] in ";}")

def usi(nome):
    # "file:riga (funzione)" di ogni uso di `nome` nel codice: niente
    # commenti, stringhe, la definizione stessa e le righe `use`
    pat = re.compile(r"(?<!\w)" + re.escape(nome) + r"(?!\w)")
    out = []
    for f in sorted(TESTI):
        if nome not in TESTI[f]:
            continue
        cod, mas, fns = analisi(f)
        imp = [(u.start(), u.end()) for u in re.finditer(r"\buse\b[^;]*;", mas)]
        for m in pat.finditer(mas):
            p = m.start()
            if re.search(r"\bfn\s+$", mas[max(0, p - 12):p]) or any(a <= p < b for a, b in imp):
                continue
            out.append((f, riga(mas, p), dentro(fns, p)))
    return out
def fuori(elenco, ammessi):
    return ["%s:%d (%s)" % (f, r, fn or "fuori da funzioni") for f, r, fn in elenco if (f, fn) not in ammessi]

P = r"(?:(?:crate|hbb_common|config|common|keys)::)*"   # prefissi di percorso

x, y = unica(C, "is_custom_client"), unica(C, "get_app_name")
if x and y:
    esito(re.fullmatch(P + r'get_app_name\(\)!="RustDesk"', compatto(x[1]))
          and re.fullmatch(P + r"APP_NAME\.read\(\)\.unwrap\(\)\.clone\(\)", compatto(y[1])),
          "is_custom_client() e' vero perche' APP_NAME non e' RustDesk",
          "is_custom_client() o get_app_name() non dipendono piu' solo da APP_NAME: rileggere, "
          "da li' dipende il controllo versione verso api.rustdesk.com  [%s:%d]" % (C, x[0]))

x = unica(C, "check_software_update")
if x:
    esito(re.match(r"if" + P + r"is_custom_client\(\)\{return;?\}", compatto(x[1])),
          "check_software_update() esce per prima cosa se is_custom_client()",
          "check_software_update() non esce piu' per prima cosa con is_custom_client(): "
          "la GUI interrogherebbe api.rustdesk.com  [%s:%d]" % (C, x[0]))

male = fuori(usi("do_check_software_update"), {(C, "check_software_update"), (U, "check_update")})
esito(not male, "do_check_software_update() (api.rustdesk.com) e' chiamata solo da check_software_update e check_update",
      "do_check_software_update() chiamata fuori dalle due guardie, in " + ", ".join(male) +
      ": interrogherebbe api.rustdesk.com")

male = fuori(usi("version_check_request"), {(C, "do_check_software_update")})
esito(not male, "version_check_request() (l'indirizzo api.rustdesk.com) si usa solo in do_check_software_update",
      "version_check_request() usata anche in " + ", ".join(male) + ": un'altra strada verso api.rustdesk.com")

x = unica(U, "check_update")
if x:
    k = compatto(x[2])
    g = re.search(r"if!\(manually\|\|" + P + r"Config::get_bool_option\(" + P +
                  r"OPTION_ALLOW_AUTO_UPDATE\)\)\{returnOk\(\(\)\);?\}", k)
    c = re.search(r"do_check_software_update\(", k)    # senza spazi: niente \b
    esito(g and in_testa(k, g.start()) and (c is None or g.end() <= c.start()),
          "check_update() dell'updater esce se allow-auto-update non e' Y e il controllo non e' manuale",
          "check_update() non esce piu' con allow-auto-update spento prima di do_check_software_update: "
          "l'updater scaricherebbe l'installer di RustDesk  [%s:%d]" % (U, x[0]))

male = fuori(usi("manually_check_update"), set()) + fuori(usi("check_update"), {(U, "start_auto_update_check_")})
for f in sorted(TESTI):
    if "CheckUpdate" in TESTI[f]:
        cod, mas, fns = analisi(f)
        male += ["%s:%d (%s, invio di CheckUpdate)" % (f, riga(mas, m.start()), dentro(fns, m.start()))
                 for m in re.finditer(r"\bsend\s*\(\s*(?:\w+::)*CheckUpdate\b", mas)
                 if (f, dentro(fns, m.start())) != (U, "manually_check_update")]
esito(not male, "il controllo manuale (scavalca allow-auto-update) parte solo da manually_check_update, che non ha chiamanti",
      "l'updater puo' partire in modo manuale, scavalcando allow-auto-update: " + ", ".join(male))

cod, mas, fns = analisi(H)
blocchi = [(m.end(), chiudi(mas, m.end() - 1)) for m in re.finditer(r"^impl\s+Config\s*\{", mas, re.M)]
x, y, z = unica(H, "get_option", blocchi), unica(H, "get_bool_option", blocchi), unica(H, "get_or")
if x and y and z:
    esito(compatto(x[1]).startswith("get_or(&OVERWRITE_SETTINGS,")
          and re.fullmatch(r"option2bool\(k,&(?:Self|Config)::get_option\(k\)\)", compatto(y[1]))
          and compatto(z[1]) == "a.read().unwrap().get(k).or(b.get(k)).or(c.read().unwrap().get(k)).cloned()",
          "Config::get_option legge per primo OVERWRITE_SETTINGS (allow-auto-update N e approve-mode click vincono)",
          "Config::get_option/get_bool_option/get_or non danno piu' la precedenza a OVERWRITE_SETTINGS: "
          "rileggere, ne dipendono auto-update spento e accesso presidiato  [%s:%d]" % (H, x[0]))

x, y = unica(C, "get_api_server_"), unica(C, "get_custom_rendezvous_server")
if x and y:
    k = compatto(x[1])
    m = re.search(r"let(\w+)=" + P + r"get_custom_rendezvous_server\(custom\);if!\1\.is_empty\(\)\{", k)
    kp = compatto(y[2])
    g = re.search(r"if!" + P + r"PROD_RENDEZVOUS_SERVER\.read\(\)\.unwrap\(\)\.is_empty\(\)\{return" + P +
                  r"PROD_RENDEZVOUS_SERVER\.read\(\)\.unwrap\(\)\.clone\(\);?\}", kp)
    esito(("admin.rustdesk.com" not in k or (m and k.count("admin.rustdesk.com") == 1
                                           and k.endswith('"https://admin.rustdesk.com".to_owned()')))
          and g and in_testa(kp, g.start()),
          "il ripiego admin.rustdesk.com di get_api_server_ vale solo senza server: prima c'e' PROD_RENDEZVOUS_SERVER",
          "get_api_server_ o get_custom_rendezvous_server cambiate: il ripiego admin.rustdesk.com potrebbe "
          "valere anche col nostro server, rileggere  [%s:%d]" % (C, x[0]))

HOST = re.compile(r"(?i)(?<![\w.-])((?!www\.)[a-z0-9-]+(?:\.[a-z0-9-]+)*\.rustdesk\.com)(?![\w-])")
TEST = re.compile(r"#\[cfg\(test\)\]\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*\{")
NOTI = {(L, "version_check_request", "api.rustdesk.com"), (C, "get_api_server_", "admin.rustdesk.com")}
male = []
for f in sorted(TESTI):
    if not re.search(r"(?i)[a-z0-9-]\.rustdesk\.com", TESTI[f]):
        continue
    cod, mas, fns = analisi(f)
    test = [(m.start(), chiudi(mas, m.end() - 1)) for m in TEST.finditer(mas)]
    for m in HOST.finditer(cod):
        p = m.start()
        if not any(a <= p < b for a, b in test) and (f, dentro(fns, p), m.group(1).lower()) not in NOTI:
            male.append("%s:%d (%s)" % (f, riga(cod, p), m.group(1)))
dart = 0
for d, sotto, nomi in os.walk("flutter/lib"):       # il Dart fa anche chiamate http sue
    sotto.sort()
    for f in sorted(x for x in nomi if x.endswith(".dart")):
        dart += 1
        with open(os.path.join(d, f), encoding="utf-8", errors="replace") as fh:
            for n, r in enumerate(fh, 1):
                if not r.lstrip().startswith("//"):
                    male += ["%s:%d (%s)" % (os.path.join(d, f), n, h) for h in HOST.findall(r)]
if not dart:
    male.append("flutter/lib senza file .dart, codice Dart non controllato")
esito(not male, "nessun altro host *.rustdesk.com nel codice Rust e Dart: solo api (version_check_request) e admin (ripiego)",
      "host RustDesk nuovo o spostato nel codice Rust o Dart, in " + ", ".join(male) +
      ": se il client lo contatta da solo va spento, altrimenti rileggere e aggiornare questo controllo")
PY
) || esito_sil="${esito_sil:-}"$'\nERRORE\t'"controllo del silenzio verso RustDesk non eseguito: python3 terminato con errore"
[ -n "$esito_sil" ] || esito_sil=$'ERRORE\t'"controllo del silenzio verso RustDesk senza esito"
while IFS=$'\t' read -r tipo msg; do
  [ -n "$tipo$msg" ] || continue
  if [ "$tipo" = ok ]; then ok "$msg"; else errore "$msg"; fi
done <<<"$esito_sil"

printf '\nverifica-patch: %s errori, %s avvisi\n' "$errori" "$avvisi"
[ "$errori" -eq 0 ]
