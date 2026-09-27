#!/usr/bin/env bash
# Verifica della patch Remotek sul client (REGOLE 6.2 del repo interno).
#
# Controlla che, dopo una MR o un merge upstream, il client sia ancora Remotek:
# nome, server, chiave, API e accesso presidiato in hbb_common; metadati
# Windows; link e attribuzione; lingua; chiave che verifica custom.txt; tema
# generato; nessun trigger automatico nei workflow upstream; ogni file diverso
# dal tag upstream elencato in REMOTEK.md; server e chiave mai dal nome del
# file; nessuna chiamata automatica ai server RustDesk; il token dell'account
# del tecnico fuori dai messaggi di rendezvous; un permesso bloccato che un
# messaggio di rendezvous non puo' riaccendere; il changelog che cita
# l'hbb_common del submodule; il riquadro degli avvisi della home col colore
# del marchio e i comandi che le chiavi bloccate lasciano senza effetto
# (scheda 2FA, "Visualizza telecamera") che non si mostrano.
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
  # I default di privacy del 2026-09-20 (ADR-0017): quattro funzioni, sei
  # chiavi. Prima le quattro enable-*. Il valore deve essere "N" e non la
  # stringa vuota: per le chiavi enable-* option2bool accende tutto cio' che
  # non e' esattamente "N", quindi ("", stringa vuota) le lascerebbe accese.
  for coppia in \
    'OPTION_ENABLE_LAN_DISCOVERY|rete locale: il PC non risponde piu'"'"' a chi lo cerca (rispondeva con MAC, ID, nome del PC e utente a chiunque sulla stessa rete; la porta UDP resta in ascolto come in upstream)' \
    'OPTION_ENABLE_RECORD_SESSION|registrazione della sessione spenta (copia dello schermo del cliente senza avviso)' \
    'OPTION_ENABLE_PRIVACY_MODE|modalita'"'"' privacy spenta (oscurare lo schermo contraddice l'"'"'accesso presidiato)' \
    'OPTION_ENABLE_CAMERA|videocamera spenta (su un PC aziendale e'"'"' videosorveglianza)'
  do
    chiave=${coppia%%|*}
    testo=${coppia#*|}
    contiene "$CFG" "$OVR"'.*'"$chiave"'\.to_owned\(\), "N"\.to_owned\(\)' \
      "$testo"
  done
  # Quinto controllo: la registrazione ha due porte e la seconda non passa dai
  # permessi. Con allow-auto-record-incoming il PC controllato avvia da solo un
  # registratore a ogni sessione in entrata (src/server/video_service.rs,
  # get_recorder), senza leggere enable-record-session ne' Permission::Recording
  # e senza accendere l'icona della telecamera nella finestra di accettazione.
  # Qui il valore atteso e' "N" come per allow-auto-update: per il prefisso
  # allow- option2bool accende solo l'esatto "Y".
  contiene "$CFG" "$OVR"'.*OPTION_ALLOW_AUTO_RECORD_INCOMING\.to_owned\(\), "N"\.to_owned\(\)' \
    'registrazione automatica delle sessioni in entrata spenta (partiva senza permesso e senza avviso, bastava una riga di strategia)'
  # Sesto controllo, che tiene su gli altri quattro: il client concede
  # registrazione, modalita' privacy e videocamera senza nemmeno leggere la
  # chiave enable-* appena access-mode vale "full" (src/server/connection.rs,
  # is_permission_enabled_locally). Qui il valore atteso e' la stringa vuota:
  # e' il valore neutro, quello di un'installazione appena fatta, mentre "N"
  # per questa chiave non significherebbe niente.
  contiene "$CFG" "$OVR"'.*OPTION_ACCESS_MODE\.to_owned\(\), ""\.to_owned\(\)' \
    'access-mode bloccata a vuota (con "full" il preset riaccende tre dei quattro default)'

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
# Editore della voce Disinstalla: lo scrivono install_me (installazione, "Click
# to upgrade", installazione sopra una versione precedente) e update_me (solo
# con --update). Un'altra riga Publisher, come quella upstream con {app_name},
# mostrerebbe il nome dell'app come editore.
WIN=src/platform/windows.rs
if [ ! -f "$WIN" ]; then
  errore "voce Disinstalla: manca $WIN"
else
  n_pub=$(grep -cF '/v Publisher /t REG_SZ /d \"Infinitek S.r.l.\"' "$WIN")
  n_tot=$(grep -cF '/v Publisher ' "$WIN")
  if [ "$n_pub" = 2 ] && [ "$n_tot" = 2 ]; then
    ok 'voce Disinstalla: editore Infinitek S.r.l. in install_me e update_me'
  else
    errore "voce Disinstalla: $n_pub righe Publisher con Infinitek S.r.l. su $n_tot, attese 2 su 2 (install_me e update_me)  [$WIN]"
  fi
fi

# --- 3. Link e attribuzione ---------------------------------------------------
INFO=flutter/lib/desktop/pages/desktop_setting_page.dart
INST=flutter/lib/desktop/pages/install_page.dart
non_contiene "$INFO" 'rustdesk\.com' 'dialogo Informazioni: nessun link a rustdesk.com'
non_contiene "$INST" 'rustdesk\.com' 'dialogo di installazione: nessun link a rustdesk.com'
# I testi del dialogo si cercano a inizio riga (prima del letterale solo
# spazi): a un merge, una nostra riga commentata accanto a quella upstream non
# basta. Due righe cominciano con "Basato su RustDesk": ognuna ha il suo
# controllo, altrimenti una coprirebbe l'assenza dell'altra.
contiene "$INFO" "^[[:space:]]*'Basato su RustDesk, AGPL-3\.0 - sorgenti'" \
  'dialogo Informazioni: attribuzione a RustDesk (AGPL 5)'
contiene "$INFO" 'github\.com/labinfinitek/remotek-client' \
  'dialogo Informazioni: link ai sorgenti (AGPL 6)'
contiene "$INFO" "^[[:space:]]*'Copyright © [0-9-]+ Infinitek S\.r\.l\.'" \
  'dialogo Informazioni: copyright Infinitek S.r.l. nel riquadro'
contiene "$INFO" "^[[:space:]]*'Basato su RustDesk, Copyright © .* Purslane Tech Pte\. Ltd\." \
  'dialogo Informazioni: attribuzione a RustDesk e Purslane nel riquadro'
non_contiene "$INFO" "^[[:space:]]*'Copyright © .*Purslane" \
  'dialogo Informazioni: niente riga upstream con la sola Purslane'
non_contiene "$INFO" 'Slogan_tip' 'dialogo Informazioni: niente slogan di RustDesk'

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
# read_custom_client applica il custom.txt accanto all'eseguibile, cioe' le
# impostazioni per cliente, anche di sicurezza: `override-settings` scrive nello
# stesso OVERWRITE_SETTINGS della sezione 1 e vince sui default forzati, accesso
# presidiato compreso. La chiave cablata che ne verifica la firma deve essere la
# nostra (ADR-0003): con quella di RustDesk noi non possiamo firmare nulla e un
# file firmato da loro passerebbe. Si legge il corpo della funzione, non il file:
# una seconda costante altrove, o un merge che riporta quella upstream, non deve
# passare per un grep contento. Il valore della costante da solo non basta:
# servono anche le due righe che la usano, perche' senza `get_rs_pk(KEY)` la
# chiave giusta non arriva alla verifica e senza `sign::verify` vale qualunque
# custom.txt, firmato o no. Se python3 fallisce il controllo non e' eseguito ed
# e' un errore.
CHIAVE_CUSTOM='P0M92FARVRfUC+84V77gcMlFV8Xmk4Q1exLp58xJir0='
CHIAVE_CUSTOM_RUSTDESK='5Qbwsde3unUcJBtrx9ZkvUmwFNoExHzpryHuPUdqlWM='
esito_custom=$(python3 - src/common.rs "$CHIAVE_CUSTOM" "$CHIAVE_CUSTOM_RUSTDESK" <<'PY'
import re, sys
percorso, nostra, rustdesk = sys.argv[1:4]
try:
    testo = open(percorso, encoding="utf-8").read()
except OSError:
    print("file mancante"); sys.exit(0)
corpi = re.findall(r"^pub fn read_custom_client\(config: &str\) \{\n(.*?)^\}$", testo, re.M | re.S)
if len(corpi) != 1:
    print(f"read_custom_client trovata {len(corpi)} volte, attesa 1: rileggere"); sys.exit(0)
righe = [re.sub(r"\s+", " ", r).strip() for r in corpi[0].splitlines()]
righe = [r for r in righe if r and not r.startswith("//")]
chiavi = [m.group(1) for m in (re.fullmatch(r'const KEY: &str = "([^"]*)";', r) for r in righe) if m]
attese = ["let Some(pk) = get_rs_pk(KEY) else {", "let Ok(data) = sign::verify(&data, &pk) else {"]
mancanti = [a for a in attese if a not in righe]
if chiavi != [nostra]:
    print("la chiave che verifica custom.txt non e' quella Remotek: "
          + (", ".join(chiavi) or "nessuna costante KEY nella funzione"))
elif mancanti:
    print("read_custom_client non verifica piu' la firma con quella chiave, righe mancanti: "
          + " | ".join(mancanti))
elif any(rustdesk in r for r in righe):
    print("la chiave pubblica di RustDesk e' tornata dentro read_custom_client")
PY
) || esito_custom="controllo della chiave di custom.txt non eseguito: python3 terminato con errore"
if [ -z "$esito_custom" ]; then
  ok "custom.txt verificato con la chiave pubblica Remotek (la privata non sta in nessun repo)"
else
  errore "$esito_custom  [src/common.rs]"
fi
# Fuori di li', la STRINGA base64 della chiave del server pubblico di RustDesk
# non deve comparire in nessun file versionato del fork, submodule compreso.
# Ammessa solo in src/lang/ (li' e' dentro un esempio di ID), nel modulo di test
# di src/custom_server.rs (valore atteso di un nome di file firmato, strada gia'
# chiusa dalla sezione 9) e in questo script.
#
# Cosa questo controllo NON prova: e' un confronto di stringhe, non un censimento
# delle chiavi. Nel fork resta viva una SECONDA chiave pubblica di RustDesk, in
# forma di array di byte e quindi invisibile a un grep sul base64: `const PK` di
# get_custom_server_from_config_string (src/custom_server.rs), che con
# sign::verify convalida i payload di `--config`. Sostituirla e' una decisione a
# parte (ADR-0015, che intanto ha chiuso la strada del nome del file); i due
# controlli in fondo alla sezione la tengono ferma.
#
# git grep legge i file tracciati e, nel solo superprogetto, anche quelli non
# ancora in indice (--untracked non si combina con --recurse-submodules): un
# file nuovo dentro libs/hbb_common lo vede solo dopo un `git add` li' dentro.
CS=src/custom_server.rs
# Estremi del modulo di test: dalla riga #[cfg(test)] alla prima graffa di
# chiusura in colonna zero. Esonerare "tutto cio' che segue #[cfg(test)]"
# lascerebbe passare una funzione aggiunta in fondo; contare le graffe le
# conterebbe anche dentro stringhe e commenti, e una sola graffa spaiata in un
# nome di file di prova farebbe sparire l'esenzione e accendere un rosso finto.
estremi=$(python3 - "$CS" <<'PY'
import sys
try:
    righe = open(sys.argv[1], encoding="utf-8").read().splitlines()
except OSError:
    righe = []
inizio = next((i for i, r in enumerate(righe, 1) if r.startswith("#[cfg(test)]")), 0)
fine = 0
if inizio:
    fine = next((i for i, r in enumerate(righe, 1) if i > inizio and r.rstrip() == "}"), 0)
print(inizio, fine)
PY
) || estremi=''
trovate=$( { git grep --recurse-submodules -nF -e "$CHIAVE_CUSTOM_RUSTDESK" -- . ;
             git grep --untracked -nF -e "$CHIAVE_CUSTOM_RUSTDESK" -- . ; } 2>/dev/null | sort -u)
if [ -z "$estremi" ]; then
  errore "controllo della chiave di RustDesk fuori da read_custom_client non eseguito: python3 terminato con errore  [$CS]"
elif [ "${estremi%% *}" != 0 ] && [ "${estremi##* }" = 0 ]; then
  errore "modulo di test di $CS senza graffa di chiusura in colonna zero: esenzione non calcolabile  [$CS]"
elif ! printf '%s\n' "$trovate" | grep -q .; then
  # la stringa sta in decine di src/lang/*.rs: zero righe vuol dire scansione non avvenuta
  errore "scansione della chiave di RustDesk non eseguita: git grep non ha trovato nemmeno le occorrenze di src/lang/"
else
  fuori=$(printf '%s\n' "$trovate" |
    awk -F: -v cs="$CS" -v i="${estremi%% *}" -v f="${estremi##* }" -v me=".github/scripts/verifica-patch.sh" '
      $1 ~ /^src\/lang\// { next }
      $1 == me { next }
      ($1 == cs && f > 0 && $2 >= i && $2 <= f) { next }
      { print $1 ":" $2 }')
  if [ -z "$fuori" ]; then
    ok "la stringa della chiave pubblica di RustDesk non compare fuori da src/lang/ e dal modulo di test di $CS"
  else
    while IFS= read -r r; do
      [ -n "$r" ] && errore "chiave pubblica di RustDesk in $r: se verifica qualcosa va sostituita con la nostra"
    done <<<"$fuori"
  fi
fi
# E una seconda chiave in forma di array di byte non deve entrare di nascosto.
# Due controlli, tutti e due limitati:
#  - dentro $CS: `const PK` e' l'unico array di 32 byte a valori diversi, ha il
#    valore noto di RustDesk, e le sole righe che verificano una firma restano
#    le due di get_custom_server_from_config_string; cosi' una seconda chiave
#    scritta li' dentro, anche inline (`sign::PublicKey([...])`), non passa;
#  - fuori: nessun altro .rs assegna 32 valori diversi tra loro a un array di
#    byte, scritto `[u8; 32]` o `&[u8]`. Si cerca l'assegnazione, non il tipo:
#    `[u8; 32]` in una firma (hbb_common constant_time_eq_32) non e' una chiave
#    cablata, e 32 valori uguali sono un buffer azzerato, non una chiave.
# Cosa NON vedono: una chiave che non sia l'assegnazione di un array di byte
# scritto valore per valore, cioe' passata inline a una chiamata fuori da $CS
# (`sign::PublicKey([...])`), letta con `include_bytes!`, scritta come stringa
# esadecimale o in un base64 diverso da quello cercato qui sopra. Il perno resta
# chi chiama sign::verify: fuori da $CS lo controlla solo il primo controllo
# della sezione, e solo dentro read_custom_client.
PK_NOTA='88, 168, 68, 104, 60, 5, 163, 198, 165, 38, 12, 85, 114, 203, 96, 163, 70, 48, 0, 131, 57, 12, 46, 129, 83, 17, 84, 193, 119, 197, 130, 103'
PATT_ARRAY=( -e '\[u8; *32\] *= *&?\[' -e '&\[u8\] *= *&?\[' )
array=$( { git grep --recurse-submodules -lE "${PATT_ARRAY[@]}" -- '*.rs' ;
           git grep --untracked -lE "${PATT_ARRAY[@]}" -- '*.rs' ; } 2>/dev/null | sort -u)
if ! printf '%s\n' "$array" | grep -qxF "$CS"; then
  errore "scansione degli array di 32 byte non eseguita: git grep non ha trovato nemmeno const PK di $CS  [$CS]"
else
  esito_pk=$(python3 - "$CS" "$PK_NOTA" $array <<'PY'
import re, sys
cs, attesi = sys.argv[1], sys.argv[2]
altri = [f for f in sys.argv[3:] if f != cs]
ASSEGN = re.compile(r"(?:\[u8; *32\]|&\[u8\]) *= *&?\[(.*?)\]", re.S)

def chiavi(testo):
    # solo gli array scritti valore per valore, di 32 valori e non tutti uguali:
    # [0u8; 32] e 32 zeri sono un buffer, non una chiave.
    fuori = []
    for m in ASSEGN.finditer(testo):
        v = [x.strip() for x in m.group(1).split(",") if x.strip()]
        if len(v) == 32 and len(set(v)) > 1:
            fuori.append(", ".join(v))
    return fuori

guai = []
try:
    testo = open(cs, encoding="utf-8").read()
except OSError:
    print(f"{cs} mancante: rileggere chi verifica i payload di --config"); sys.exit(0)
if chiavi(testo) != [attesi]:
    guai.append(f"in {cs} const PK non e' piu' l'unico array di 32 byte non banale, o non e' piu' la chiave nota di RustDesk: rileggere chi la usa prima di accettarla")
corpi = re.findall(
    r"^fn get_custom_server_from_config_string\(s: &str\) -> ResultType<CustomServer> \{\n(.*?)^\}$",
    testo, re.M | re.S)
if len(corpi) != 1:
    guai.append(f"get_custom_server_from_config_string trovata {len(corpi)} volte in {cs}, attesa 1: rileggere")
elif "sign::PublicKey(*PK)" not in corpi[0] or "sign::verify(" not in corpi[0]:
    guai.append(f"in {cs} i payload di --config non sono piu' verificati con const PK: rileggere")
elif testo.count("sign::verify(") != 1 or testo.count("sign::PublicKey(") != 1:
    guai.append(f"in {cs} c'e' piu' di una verifica di firma: la seconda puo' portarsi dietro un'altra chiave")
for f in altri:
    try:
        altro = open(f, encoding="utf-8").read()
    except OSError:
        guai.append(f"{f} illeggibile: controllare a mano l'array di 32 byte")
        continue
    if chiavi(altro):
        guai.append(f"array di 32 byte non previsto in {f}: controllare se e' una chiave")
print("\n".join(guai))
PY
) || esito_pk="controllo delle chiavi in forma di array non eseguito: python3 terminato con errore"
  if [ -z "$esito_pk" ]; then
    ok "in $CS l'unica chiave in forma di array resta const PK, quella nota di RustDesk (payload di --config, ADR-0015); nessun altro .rs assegna 32 valori diversi a un array di byte"
  else
    while IFS= read -r r; do
      [ -n "$r" ] && errore "$r"
    done <<<"$esito_pk"
  fi
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

# --- 11. Il token dell'account fuori dai messaggi di rendezvous -----------------
# REM-2026-001. Il campo token di PunchHoleRequest e RequestRelay portava il
# token dell'account del tecnico, cioe' la credenziale che apre anche
# /api/admin/* dell'API; RequestRelay hbbs lo inoltra al PC controllato. Upstream
# quei campi li riempie (token: token.to_owned()): a un merge la riga torna
# indietro senza conflitti se il contesto intorno cambia, e nessun test la vede.
# Si cercano tutte le costruzioni dei due messaggi in src/, non solo le due di
# client.rs, cosi' un mittente nuovo non sfugge. Un campo token assente va bene
# (esce vuoto lo stesso): e' il caso delle due RequestRelay dirette a hbbr,
# client.rs create_relay e server.rs create_relay_connection_, che il token non
# lo hanno mai scritto. Un campo token scritto deve valere Default::default(),
# e i campi scritti devono restare i nostri due: se la riga esplicita sparisce
# il comportamento resta giusto, ma sparisce anche il segnale che al merge dopo
# impedisce di rimetterci dentro il token senza accorgersene.
CLI=src/client.rs
esito_token=$(python3 - <<'PY'
import os, re

MESSAGGI = ("PunchHoleRequest", "RequestRelay")
ATTESO = "Default::default()"
male = []
visti = {m: 0 for m in MESSAGGI}
espliciti = []

sorgenti = []
for d, sotto, nomi in os.walk("src"):
    sotto.sort()
    sorgenti += [os.path.join(d, f) for f in sorted(nomi) if f.endswith(".rs")]
if not sorgenti:
    print("src senza file .rs: costruzione dei messaggi di rendezvous non controllata")
    raise SystemExit

for percorso in sorgenti:
    with open(percorso, encoding="utf-8", errors="replace") as fh:
        testo = fh.read()
    for nome in MESSAGGI:
        for m in re.finditer(r"\b%s\s*\{" % nome, testo):
            i, liv = m.end(), 1
            while i < len(testo) and liv:
                liv += {"{": 1, "}": -1}.get(testo[i], 0)
                i += 1
            corpo = testo[m.end():i - 1]
            visti[nome] += 1
            dove = "%s:%d (%s)" % (percorso, testo.count("\n", 0, m.start()) + 1, nome)
            # Le righe di solo commento non contano: il campo dev'esserci davvero.
            campo = [r for r in corpo.splitlines()
                     if not r.lstrip().startswith("//") and re.match(r"\s*token\s*:", r)]
            if not campo:
                continue
            if len(campo) > 1:
                male.append("%s: campo token scritto %d volte" % (dove, len(campo)))
                continue
            valore = campo[0].split(":", 1)[1].strip().rstrip(",").strip()
            if valore != ATTESO:
                male.append("%s: il campo token vale '%s' e non %s"
                            % (dove, valore, ATTESO))
            else:
                espliciti.append((percorso, nome))

mancanti = [n for n in MESSAGGI if not visti[n]]
if mancanti:
    male.append("nessuna costruzione di " + " ne' di ".join(mancanti) +
                " trovata in src/: il controllo non sta guardando niente")
elif not male and sorted(espliciti) != sorted([("src/client.rs", n) for n in MESSAGGI]):
    trovati = ", ".join("%s (%s)" % c for c in sorted(espliciti)) or "nessuno"
    male.append("i campi token scritti a " + ATTESO + " sono " + trovati +
                ", attesi uno per messaggio in src/client.rs (le due righe di "
                "REM-2026-001): rileggere e aggiornare questo controllo")
for r in male:
    print(r)
PY
) || esito_token="${esito_token:-}"$'\n'"controllo del token nei messaggi di rendezvous non eseguito: python3 terminato con errore"
if [ -z "$esito_token" ]; then
  ok "il token dell'account non entra in PunchHoleRequest ne' in RequestRelay (REM-2026-001)"
else
  while IFS= read -r r; do
    [ -n "$r" ] && errore "$r: uscirebbe il token dell'account del tecnico, che apre anche /api/admin/* dell'API, e RequestRelay hbbs lo inoltra al PC controllato (REM-2026-001)"
  done <<<"$esito_token"
fi
# Il rovescio dello stesso controllo: si svuotano i campi dei messaggi, mai la
# variabile token. Le due guardie !key.is_empty() && !token.is_empty() decidono
# secure_tcp, cioe' la cifratura dello scambio col server: con la variabile
# vuota il canale verso il server resterebbe in chiaro, molto peggio del difetto
# che la sezione chiude.
n_guardie=$(grep -Ec '^[[:space:]]*if !key\.is_empty\(\) && !token\.is_empty\(\) \{$' "$CLI" || true)
if [ "$n_guardie" = 2 ]; then
  ok 'le due guardie di secure_tcp leggono ancora la variabile token (lo scambio col server resta cifrato)'
else
  errore "guardie !key.is_empty() && !token.is_empty() trovate $n_guardie, attese 2: se e' stata svuotata la variabile token invece dei campi dei messaggi, lo scambio col server non e' piu' cifrato  [$CLI]"
fi

# --- 12. Un permesso bloccato non lo riaccende un messaggio di rendezvous -------
# PunchHole, RequestRelay e FetchLocalAddr portano un bitmap ControlPermissions
# che upstream fa vincere sulla chiave locale: Connection::permission() guarda
# prima il bitmap e chiama is_permission_enabled_locally() (cioe' access-mode
# piu' la chiave enable-*) solo se il bitmap non risponde. Il bitmap puo' dire
# "enable" e arriva da messaggi ricevuti, senza firma e senza verifica del
# mittente (src/rendezvous_mediator.rs, socket UDP in bind e non connect):
# senza guardia videocamera, registrazione e modalita' privacy si riaccendono
# senza che nessuna delle sei chiavi di OVERWRITE_SETTINGS venga letta, e la
# finestra di accettazione mostra il permesso gia' concesso. La guardia lascia
# restringere e non allargare: per una chiave bloccata nel binario vale l'AND
# fra il bitmap e il valore locale. Sono poche righe dentro una funzione
# upstream, quindi a un merge tornano indietro senza conflitti e nessun test
# le vede; il controllo pretende il testo esatto, spazi e a capo a parte.
CONN=src/server/connection.rs
esito_perm=$(CONN="$CONN" python3 - <<'PY'
import os, re

percorso = os.environ["CONN"]
try:
    with open(percorso, encoding="utf-8", errors="replace") as fh:
        testo = fh.read()
except OSError as e:
    print("%s non leggibile (%s): la guardia dei permessi non e' controllata" % (percorso, e))
    raise SystemExit

firme = list(re.finditer(r"\bfn permission\s*\(", testo))
if len(firme) != 1:
    print("in %s le definizioni di 'fn permission(' sono %d, attesa 1: "
          "il controllo non sta guardando niente" % (percorso, len(firme)))
    raise SystemExit

# Corpo della funzione: dalla graffa che apre fino a quella che chiude.
i = testo.index("{", firme[0].end())
j, liv = i + 1, 1
while j < len(testo) and liv:
    liv += {"{": 1, "}": -1}.get(testo[j], 0)
    j += 1
corpo = testo[i + 1:j - 1]
# Via i commenti di riga (il nostro perche' e' un commento: non deve reggere
# il controllo) e poi tutti gli spazi a uno solo.
senza_commenti = "\n".join(r for r in corpo.splitlines() if not r.lstrip().startswith("//"))
piatto = re.sub(r"\s+", " ", senza_commenti).strip()

GUARDIA = ("if crate::ui_interface::is_option_fixed(enable_prefix_option) { "
           "return enabled && Self::is_permission_enabled_locally(enable_prefix_option); }")
NUDO = "return enabled;"
RIPIEGO = "Self::is_permission_enabled_locally(enable_prefix_option)"

male = []
if "crate::get_control_permission(" not in piatto:
    male.append("Connection::permission() non legge piu' crate::get_control_permission: "
                "il bitmap dei permessi e' cambiato di posto, rileggere questa sezione")
if GUARDIA not in piatto:
    male.append("Connection::permission() non ha piu' la guardia esatta "
                "'%s'" % GUARDIA)
elif NUDO in piatto and piatto.index(GUARDIA) > piatto.index(NUDO):
    male.append("in Connection::permission() la guardia viene dopo '%s': "
                "il bitmap ritorna prima che si guardi la chiave locale" % NUDO)
if piatto.count(NUDO) != 1:
    male.append("in Connection::permission() le righe '%s' sono %d, attesa 1: "
                "rileggere la funzione" % (NUDO, piatto.count(NUDO)))
if not piatto.rstrip().endswith(RIPIEGO):
    male.append("Connection::permission() non finisce piu' con il ripiego locale "
                "'%s': senza, una chiave non bloccata non legge piu' access-mode "
                "ne' enable-*" % RIPIEGO)
for r in male:
    print(r)
PY
) || esito_perm="${esito_perm:-}"$'\n'"controllo della guardia dei permessi non eseguito: python3 terminato con errore"
if [ -z "$esito_perm" ]; then
  ok 'un permesso bloccato in OVERWRITE_SETTINGS non lo riaccende il bitmap dei messaggi di rendezvous'
else
  while IFS= read -r r; do
    [ -n "$r" ] && errore "$r: un messaggio di rendezvous con il bit a \"enable\" riaccenderebbe videocamera, registrazione o modalita' privacy senza leggere nessuna delle sei chiavi bloccate  [$CONN]"
  done <<<"$esito_perm"
fi

# La guardia ha due operandi e tutti e due sono codice upstream che non
# modifichiamo: crate::ui_interface::is_option_fixed (qui sotto) e
# Self::is_permission_enabled_locally (in fondo alla sezione). Di quest'ultima
# il controllo qui sopra pretende solo il NOME -- dentro il testo della guardia
# e come ultima riga di permission() -- quindi i due blocchi che seguono ne
# tengono fermo il corpo. Primo operando: is_option_fixed. Se un merge ne
# cambia il corpo -- per
# esempio restringendolo a una sola delle tre mappe OVERWRITE, o spostando le
# statiche -- la guardia torna un "return enabled;" mascherato e il controllo
# qui sopra resta verde, perche' legge solo src/server/connection.rs. REMOTEK.md
# elenca src/ui_interface.rs per il filtro delle lingue, quindi un conflitto su
# quella riga non porta nessuno a rileggere questa funzione trenta righe prima.
# Si pretende: una sola definizione, e il corpo che guarda ancora
# OVERWRITE_SETTINGS (spazi e a capo non contano).
UI=src/ui_interface.rs
esito_fixed=$(UI="$UI" python3 - <<'PY'
import os, re

percorso = os.environ["UI"]
try:
    with open(percorso, encoding="utf-8", errors="replace") as fh:
        testo = fh.read()
except OSError as e:
    print("%s non leggibile (%s): is_option_fixed non e' controllata" % (percorso, e))
    raise SystemExit

firme = list(re.finditer(r"\bfn is_option_fixed\s*\(", testo))
if len(firme) != 1:
    print("in %s le definizioni di 'fn is_option_fixed(' sono %d, attesa 1: "
          "il controllo non sta guardando niente" % (percorso, len(firme)))
    raise SystemExit

i = testo.index("{", firme[0].end())
j, liv = i + 1, 1
while j < len(testo) and liv:
    liv += {"{": 1, "}": -1}.get(testo[j], 0)
    j += 1
corpo = testo[i + 1:j - 1]
senza_commenti = "\n".join(r for r in corpo.splitlines() if not r.lstrip().startswith("//"))
compatto = re.sub(r"\s+", "", senza_commenti)

ATTESO = "config::OVERWRITE_SETTINGS.read().unwrap().contains_key(key)"
if ATTESO not in compatto:
    print("il corpo di is_option_fixed in %s non contiene piu' '%s'" % (percorso, ATTESO))
PY
) || esito_fixed="${esito_fixed:-}"$'\n'"controllo di is_option_fixed non eseguito: python3 terminato con errore"
if [ -z "$esito_fixed" ]; then
  ok 'is_option_fixed guarda ancora OVERWRITE_SETTINGS: la guardia dei permessi restringe davvero'
else
  while IFS= read -r r; do
    [ -n "$r" ] && errore "$r: la guardia di Connection::permission non restringerebbe piu' niente e il controllo qui sopra resterebbe verde  [$UI]"
  done <<<"$esito_fixed"
fi

# Secondo operando: Self::is_permission_enabled_locally, nello stesso file di
# permission(). Non e' solo meta' dell'AND: e' anche l'unica riga che legge
# access-mode e la chiave enable-* per tutte e dodici le voci quando il bitmap
# non risponde, cioe' il percorso normale, quello senza nessun messaggio che
# porti permessi. Il nostro diff su questo file e' di 9 righe, tutte la guardia
# (git diff 1.4.9 HEAD -- src/server/connection.rs), quindi questa funzione
# arriva dal prossimo merge upstream intatta e senza conflitto: se il suo corpo
# cambia -- un ramo nuovo davanti, il corto-circuito allargato oltre "full" --
# la guardia resta scritta identica, i controlli qui sopra restano verdi e
# cambia solo il risultato, sia dentro l'AND sia sul percorso senza bitmap.
# Si pretende: una sola definizione, e il corpo che legge ancora access-mode,
# esce ancora prima su "full"/"view" e finisce ancora su config::option2bool
# con la chiave enable-*. Spazi e a capo non contano, come per is_option_fixed.
esito_locale=$(CONN="$CONN" python3 - <<'PY'
import os, re

percorso = os.environ["CONN"]
try:
    with open(percorso, encoding="utf-8", errors="replace") as fh:
        testo = fh.read()
except OSError as e:
    print("%s non leggibile (%s): is_permission_enabled_locally non e' controllata" % (percorso, e))
    raise SystemExit

firme = list(re.finditer(r"\bfn is_permission_enabled_locally\s*\(", testo))
if len(firme) != 1:
    print("in %s le definizioni di 'fn is_permission_enabled_locally(' sono %d, attesa 1: "
          "il controllo non sta guardando niente" % (percorso, len(firme)))
    raise SystemExit

i = testo.index("{", firme[0].end())
j, liv = i + 1, 1
while j < len(testo) and liv:
    liv += {"{": 1, "}": -1}.get(testo[j], 0)
    j += 1
corpo = testo[i + 1:j - 1]
senza_commenti = "\n".join(r for r in corpo.splitlines() if not r.lstrip().startswith("//"))
compatto = re.sub(r"\s+", "", senza_commenti)

ATTESI = [
    ('Config::get_option("access-mode")',
     "non legge piu' access-mode"),
    ('ifaccess_mode=="full"{returntrue;}elseifaccess_mode=="view"{returnfalse;}',
     "non esce piu' prima su access-mode \"full\"/\"view\", oppure il corto-circuito e' cambiato"),
    ('config::option2bool(enable_prefix_option,&Config::get_option(enable_prefix_option)',
     "non finisce piu' su config::option2bool con la chiave enable-*"),
]
for atteso, perche in ATTESI:
    if atteso not in compatto:
        print("il corpo di is_permission_enabled_locally in %s %s (atteso '%s')"
              % (percorso, perche, atteso))
PY
) || esito_locale="${esito_locale:-}"$'\n'"controllo di is_permission_enabled_locally non eseguito: python3 terminato con errore"
if [ -z "$esito_locale" ]; then
  ok "is_permission_enabled_locally legge ancora access-mode e la chiave enable-*: il secondo operando della guardia e il ripiego di tutte e dodici le chiavi sono quelli su cui si e' ragionato"
else
  while IFS= read -r r; do
    [ -n "$r" ] && errore "$r: rileggere insieme la guardia di Connection::permission e ADR-0017 prima di toccare questa sezione, non correggerla a naso: cambia sia il secondo operando dell'AND sia il percorso senza bitmap, cioe' quello normale  [$CONN]"
  done <<<"$esito_locale"
fi

# --- 13. Il changelog cita l'hbb_common che l'exe porta con se' ----------------
# Le righe dei default nel changelog chiudono con lo sha di hbb_common: e' il
# puntatore con cui si lega l'eseguibile consegnato ai sorgenti dei suoi valori
# di fabbrica. A ogni bump del submodule quel puntatore scade in silenzio, ed e'
# gia' successo. Il changelog lo cita in piu' punti, quindi non basta che lo sha
# del submodule compaia da qualche parte: bastava una riga aggiornata e le altre
# restavano scadute con il controllo verde, che e' il difetto che questa sezione
# deve impedire. Si pretende quindi che OGNI sha citato come ``hbb_common `...` ``
# sia un prefisso del gitlink, e che ce ne sia almeno uno. La convenzione ("e'
# sempre il commit del submodule, non quello che introdusse il cambiamento") e'
# scritta in testa al changelog: se un giorno si vuole citare anche il commit di
# origine, prima si cambia quella e poi questa sezione.
SUB=libs/hbb_common
sha_sub=$(git ls-tree HEAD "$SUB" | awk '$2 == "commit" { print $3 }')
if [ -z "$sha_sub" ]; then
  errore "git ls-tree non da' lo sha del submodule $SUB: il puntatore del changelog a hbb_common non e' controllato  [$SUB]"
else
  esito_sha=$(SHA="$sha_sub" python3 - <<'PY'
import os, re

sha = os.environ["SHA"].strip().lower()
percorso = "CHANGELOG-REMOTEK.md"
try:
    with open(percorso, encoding="utf-8", errors="replace") as fh:
        testo = fh.read()
except OSError as e:
    print("%s non leggibile (%s): il puntatore a hbb_common non e' controllato" % (percorso, e))
    raise SystemExit

# "hbb_common `2b42505`" e non "`libs/hbb_common` dal fork": dopo il nome ci
# vuole almeno uno spazio, poi lo sha fra apici inversi.
citazioni = re.findall(r"hbb_common\s+`([0-9a-fA-F]{7,40})`", testo)
if not citazioni:
    print("%s non cita nessuno sha di hbb_common (atteso %s): manca il puntatore "
          "con cui si lega l'eseguibile consegnato ai sorgenti dei suoi valori "
          "di fabbrica" % (percorso, sha[:7]))
    raise SystemExit

scadute = sorted({c for c in citazioni if not sha.startswith(c.lower())})
if scadute:
    print("%s cita %d sha di hbb_common che non sono quello del submodule (%s): %s"
          % (percorso, len(scadute), sha[:7], ", ".join(scadute)))
PY
) || esito_sha="${esito_sha:-}"$'\n'"controllo del puntatore a hbb_common non eseguito: python3 terminato con errore"
  if [ -z "$esito_sha" ]; then
    ok "CHANGELOG-REMOTEK.md cita l'hbb_common del submodule (${sha_sub:0:7}) e nessun altro"
  else
    while IFS= read -r r; do
      [ -n "$r" ] && errore "$r: chi lega l'exe consegnato ai sorgenti dei default leggerebbe uno sha scaduto  [CHANGELOG-REMOTEK.md]"
    done <<<"$esito_sha"
  fi
fi

# --- 14. Il riquadro della home e i comandi lasciati senza effetto -------------
# Due cose che il cliente vede appena apre il programma e che un merge upstream
# riporta indietro senza conflitti: sono poche righe dentro widget upstream e
# nessun test le guarda. Il collaudo del 2026-09-20 le ha trovate tutte e due.
#
# (a) Il riquadro degli avvisi della home (invito a installare, installazione di
#     versione precedente, errori di sistema) deve restare a tinta piena
#     MyTheme.accent. Il gradiente magenta/salmone di RustDesk dava al testo
#     bianco che il riquadro contiene 3,66:1 e 2,78:1, sotto il 4,5:1 di WCAG
#     AA, ed era l'elemento piu' vistoso della prima schermata con i colori
#     scritti a mano fuori dal marchio (ADR-0004); gli altri letterali dello
#     stesso file (il grigio #DDDDDD delle iconcine della password, le spunte
#     del dialogo della password, borderColor) restano upstream, sempre per
#     ADR-0004.
#     Si guarda DENTRO buildInstallCard, non il file: la stringa
#     "BoxDecoration(color: MyTheme.accent)" sta anche nella barretta di
#     accento accanto alla password, quindi cercarla nel file sarebbe verde
#     anche con il riquadro tornato magenta. E si pretende che dentro quel
#     corpo non ci sia nessun gradiente e nessun colore scritto a mano, in
#     qualunque forma (Color.fromARGB, Color(0x...)): un merge upstream puo'
#     riportare gli stessi colori in esadecimale o su piu' righe, e un
#     controllo legato ai due letterali esatti non se ne accorgerebbe.
DHP=flutter/lib/desktop/pages/desktop_home_page.dart
esito_riquadro=$(python3 - <<'PY'
import re

DHP = "flutter/lib/desktop/pages/desktop_home_page.dart"
guai = []
try:
    with open(DHP, encoding="utf-8", errors="replace") as fh:
        testo = fh.read()
except OSError as e:
    print("%s non leggibile (%s): il riquadro degli avvisi non e' controllato"
          % (DHP, e))
    raise SystemExit(0)

# I nostri "perche'" sono commenti: non devono reggere il controllo.
testo = "\n".join(r for r in testo.splitlines() if not r.lstrip().startswith("//"))

def corpo_di(testo, inizio):
    # Dalla parentesi dei parametri (che contiene a sua volta le graffe dei
    # parametri con nome) al } che chiude il corpo della funzione.
    prof, i, fine_par = 0, inizio, -1
    while i < len(testo):
        if testo[i] == "(":
            prof += 1
        elif testo[i] == ")":
            prof -= 1
            if prof == 0:
                fine_par = i
                break
        i += 1
    if fine_par < 0:
        return None
    apre = testo.find("{", fine_par)
    if apre < 0:
        return None
    prof, i = 0, apre
    while i < len(testo):
        if testo[i] == "{":
            prof += 1
        elif testo[i] == "}":
            prof -= 1
            if prof == 0:
                return testo[apre:i]
        i += 1
    return None


firme = [m.end() - 1 for m in re.finditer(r"\bWidget buildInstallCard\(", testo)]
if len(firme) != 1:
    guai.append("%s: le definizioni di buildInstallCard() sono %d, attesa 1"
                % (DHP, len(firme)))
else:
    corpo = corpo_di(testo, firme[0])
    if corpo is None:
        guai.append("%s: il corpo di buildInstallCard() non si delimita" % DHP)
    else:
        if not re.search(r"decoration:\s*(?:const\s+)?BoxDecoration\(\s*"
                         r"color:\s*MyTheme\.accent\s*,?\s*\)", corpo):
            guai.append("%s: il riquadro degli avvisi della home non e' piu' a "
                        "tinta piena MyTheme.accent" % DHP)
        for regola, che in ((r"\bgradient\s*:", "un gradiente"),
                            (r"Color\.fromARGB\(", "un Color.fromARGB(...)"),
                            (r"Color\(0x", "un Color(0x...)")):
            if re.search(regola, corpo):
                guai.append("%s: dentro buildInstallCard() c'e' %s, cioe' un "
                            "colore fuori dal marchio" % (DHP, che))

print("\n".join(guai))
PY
) || esito_riquadro="${esito_riquadro:-}"$'\n'"controllo del riquadro della home non eseguito: python3 terminato con errore"
if [ -z "$esito_riquadro" ]; then
  ok 'il riquadro degli avvisi della home prende il colore dal marchio (tinta piena MyTheme.accent dentro buildInstallCard, nessun colore scritto a mano)'
else
  while IFS= read -r r; do
    [ -n "$r" ] && errore "$r: la prima schermata che vede il cliente torna fuori dal marchio (ADR-0004)"
  done <<<"$esito_riquadro"
fi
non_contiene "$DHP" 'Color\.fromARGB\(255, ?226, ?66, ?188\)|Color\.fromARGB\(255, ?244, ?114, ?124\)' \
  'il gradiente magenta/salmone di RustDesk non e'"'"' tornato nella home'
#
# (b) I due comandi che le chiavi bloccate hanno lasciato senza effetto non si
#     devono mostrare: la scheda 2FA di Impostazioni > Sicurezza (chiave "2fa"
#     bloccata vuota, ADR-0016) e "Visualizza telecamera" in tutti e tre i punti
#     che la offrono (enable-camera bloccata a N: il PC controllato risponde
#     sempre "No permission of viewing camera", per giunta non tradotto). Si
#     contano i punti, non si cerca solo la guardia: una voce che torna
#     scoperta -- per esempio una sottoclasse di BasePeerCard aggiunta da
#     upstream -- non si nota finche' non la clicca un cliente.
esito_vuoti=$(python3 - <<'PY'
import re

def senza_commenti(percorso):
    with open(percorso, encoding="utf-8", errors="replace") as fh:
        testo = fh.read()
    # I nostri "perche'" sono commenti: non devono reggere il controllo.
    return "\n".join(r for r in testo.splitlines() if not r.lstrip().startswith("//"))

guai = []
letti = {}
PERCORSI = (
    "flutter/lib/common.dart",
    "flutter/lib/common/widgets/peer_card.dart",
    "flutter/lib/common/widgets/toolbar.dart",
    "flutter/lib/desktop/pages/connection_page.dart",
    "flutter/lib/desktop/pages/desktop_setting_page.dart",
)
for p in PERCORSI:
    try:
        letti[p] = senza_commenti(p)
    except OSError as e:
        guai.append("%s non leggibile (%s): i comandi senza effetto non sono controllati" % (p, e))

# La condizione della videocamera sta in un solo posto ed e' "bloccata E
# spenta": con il solo is_option_fixed la voce resterebbe nascosta anche a un
# cliente a cui un custom.txt firmato riapre la videocamera.
CAM = "flutter/lib/common.dart"
if CAM in letti:
    testo = letti[CAM]
    if len(re.findall(r"bool isViewCameraFixedOff\(\)", testo)) != 1:
        guai.append("%s: le definizioni di isViewCameraFixedOff() non sono una" % CAM)
    elif not re.search(r"isOptionFixed\(kOptionEnableCamera\)\s*&&\s*"
                       r"!mainGetBoolOptionSync\(kOptionEnableCamera\)", testo):
        guai.append('%s: isViewCameraFixedOff() non e\' piu\' "bloccata E spenta": '
                    "o non nasconde piu' la voce, o la nasconde anche dove un "
                    "custom.txt firmato ha riaperto la videocamera" % CAM)

# Ogni punto che offre "Visualizza telecamera" ha la sua guardia.
PUNTI = (
    ("flutter/lib/common/widgets/peer_card.dart", 5,
     r"_viewCameraAction\(context\)",
     r"if \(!isViewCameraFixedOff\(\)\)\s*_viewCameraAction\(context\)"),
    ("flutter/lib/desktop/pages/connection_page.dart", 1,
     r"isViewCamera: true",
     r"if \(!isViewCameraFixedOff\(\)\)\s*\(\s*'View camera'"),
    ("flutter/lib/common/widgets/toolbar.dart", 1,
     r"isViewCamera: true",
     r"if \(!isViewCameraFixedOff\(\)\)\s*\{\s*v\.add\("),
)
for percorso, attesi, offerta, guardia in PUNTI:
    if percorso not in letti:
        continue
    n_offerte = len(re.findall(offerta, letti[percorso]))
    n_guardie = len(re.findall(guardia, letti[percorso]))
    if n_offerte != attesi or n_guardie != attesi:
        guai.append("%s: punti che aprono la videocamera %d, di cui con la "
                    "guardia isViewCameraFixedOff() %d, attesi %d e %d"
                    % (percorso, n_offerte, n_guardie, attesi, attesi))

# La scheda 2FA: una sola, e solo dietro la guardia.
SET = "flutter/lib/desktop/pages/desktop_setting_page.dart"
if SET in letti:
    testo = letti[SET]
    schede = len(re.findall(r"_Card\(title: '2FA'", testo))
    guardate = len(re.findall(r"if \(!_is2faFixedOff\)\s*_Card\(title: '2FA'", testo))
    if schede != 1 or guardate != 1:
        guai.append("%s: le schede 2FA sono %d e quelle dietro la guardia %d, "
                    "attesa 1 e 1" % (SET, schede, guardate))
    if not re.search(r"isOptionFixed\('2fa'\)\s*&&\s*!bind\.mainHasValid2FaSync\(\)", testo):
        guai.append('%s: _is2faFixedOff non e\' piu\' "bloccata E nessuna 2FA valida"' % SET)

print("\n".join(guai))
PY
) || esito_vuoti="${esito_vuoti:-}"$'\n'"controllo dei comandi senza effetto non eseguito: python3 terminato con errore"
if [ -z "$esito_vuoti" ]; then
  ok 'i comandi che le chiavi bloccate lasciano senza effetto non si mostrano (scheda 2FA, "Visualizza telecamera" nei tre punti che la offrono)'
else
  while IFS= read -r r; do
    [ -n "$r" ] && errore "$r: tornerebbe visibile un comando che non puo' funzionare (l'interruttore 2FA fallisce in silenzio, \"Visualizza telecamera\" con un messaggio in inglese non tradotto)"
  done <<<"$esito_vuoti"
fi

printf '\nverifica-patch: %s errori, %s avvisi\n' "$errori" "$avvisi"
[ "$errori" -eq 0 ]
