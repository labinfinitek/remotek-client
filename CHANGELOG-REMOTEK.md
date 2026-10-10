# Changelog Remotek (client)

Formato: Keep a Changelog 1.1.0, in italiano. Versioni: `remotek-<upstream>-<n>`.
Una riga per cambiamento visibile a chi usa o installa il client; la sezione
"Sicurezza" e' obbligatoria per ogni correzione di sicurezza.
Gli sha `hbb_common` fra parentesi sono sempre lo stesso: il commit del
submodule che l'eseguibile di questa versione porta con se', non quello che
introdusse il singolo cambiamento. `verifica-patch.sh` sez. 13 li tiene
allineati tutti al gitlink di `libs/hbb_common`.

## [Non rilasciato]

### Aggiunto
- `remotek-cli`, client a riga di comando per Linux, per l'agente AI di
  Infinitek (non per i clienti; l'exe Windows non cambia). Per chi installa
  l'agente: `echo "$PASSWORD" | remotek-cli login <utente-agente>` (la
  password dell'account dell'agente si legge da stdin, una riga),
  `remotek-cli whoami` (ID, utente e tecnico), `remotek-cli logout`.
  Accetta solo account di agenti AI; i clienti vedranno l'agente come
  "Agente AI per <nome del tecnico>". Nessuna password dei PC dei clienti.
- `remotek-cli terminal <ID>`: il terminale del PC del cliente per l'agente AI,
  solo dopo che il cliente accetta la finestra, mai come amministratore. Uso:
  `remotek-cli terminal 123456789` (stdin va al terminale, l'uscita su stdout);
  `--attesa 300` per aspettare il clic fino a 5 minuti (default 120 secondi);
  `--righe 40 --colonne 120` per la dimensione (default 24x80, al massimo
  65535 ciascuna).
  Il CLI esce col codice del terminale (un codice fuori da 0-255 diventa 255).
  La fine di stdin (Ctrl-D) chiude subito il terminale sul PC e ne termina
  la shell: il CLI esce con 255 e si perde l'uscita di un comando ancora in
  corso. Per avere il codice della shell si tiene stdin aperto e si manda
  `exit`.
- Le sessioni terminale (dell'agente AI e dei tecnici) si registrano dal PC
  del cliente: una copia va al server Remotek, dove il pannello la mostra e ne
  verifica l'integrita', e una resta sul PC, nella cartella `terminale`
  accanto a quella dei log del servizio (su Windows
  `C:\Windows\ServiceProfiles\LocalService\AppData\Roaming\Remotek\terminale`),
  un file per sessione, per 365 giorni: i file piu' vecchi si cancellano
  all'avvio del servizio.
- Il cliente vede la sessione terminale e puo' chiuderla: nella finestra
  delle connessioni in entrata, accanto alla scheda di una sessione
  terminale, si apre da solo il pannello con quello che la shell scrive,
  dalla prima riga, in sola lettura; il pulsante "Disconnetti" chiude la
  sessione e la shell sul PC.

### Modificato
- **Per i tecnici**: una sessione terminale non resta piu' aperta dopo la
  disconnessione, nemmeno se lo si chiede dalla finestra del terminale
  ("persistente"): chiusa la connessione, la shell sul PC del cliente
  termina. Una shell che sopravvive restava fuori dalla registrazione e dal
  controllo del cliente.

### Sicurezza
- Il PC firma con la sua chiave le richieste all'API (scheda del PC,
  heartbeat, registro delle connessioni e dei file) (REM-2026-006). L'uuid
  che lega l'ID al PC il client lo manda anche al server ID in chiaro: chi
  osservava la rete poteva scrivere all'API a nome del PC. Col primo
  aggiornamento della scheda l'API registra la chiave del PC e da li' accetta
  da quel PC solo richieste firmate. **Per chi installa**: l'orologio del PC
  dev'essere giusto; con piu' di 5 minuti di scarto l'API rifiuta le
  richieste del PC (la scheda non si aggiorna, le connessioni non finiscono
  nel registro). Un PC reinstallato ha una chiave nuova: si cancella dal
  pannello e si lascia che si registri di nuovo.
- Le richieste all'API non ripiegano piu' sul canale TCP verso il server
  (REM-2026-005). Se l'HTTPS falliva, il client di upstream rimandava la
  richiesta via TCP, cifrata solo se il server faceva lo scambio di chiavi:
  chi si metteva in mezzo riceveva in chiaro l'uuid del PC, il token
  dell'account e il resto della richiesta, e poteva rispondere
  all'heartbeat con una strategia. Col server Remotek quel ripiego non
  riusciva mai, quindi non si perde niente.

## [remotek-1.4.9-2] - 2026-10-01
Base upstream: RustDesk 1.4.9. Exe Windows x86_64 **non firmato**, sorgenti
al tag `remotek-1.4.9-2` di remotek-client e remotek-hbb-common
(hbb_common invariato rispetto a `remotek-1.4.9-1`).
**Per i tecnici**: con l'account Remotek collegato ci si connette di nuovo
ai PC; fino a `remotek-1.4.9-1` compreso bisognava uscire dall'account
(Corretto). Si installa sopra la versione precedente.

### Corretto
- Un tecnico collegato al suo account Remotek si connette di nuovo ai PC. Con
  l'account collegato il client chiedeva al server di cifrare lo scambio con
  cui si apre la sessione, e il server Remotek (hbbs ufficiale, open source)
  questo scambio non lo fa: il client aspettava e falliva con "Failed to secure
  tcp: deadline has elapsed", su qualunque PC, mentre senza account si
  collegava subito (collaudo del 2026-09-30, caso 19). Era cosi' anche negli
  eseguibili precedenti. Ora, collegato o no, il client apre la sessione nello
  stesso modo: quello di un client senza account, che e' il comportamento di
  RustDesk con il server open source. **Cosa resta uguale:** la sessione fra i
  due PC e' cifrata da un capo all'altro come prima; il token dell'account non
  va al server da `remotek-1.4.9-1`. **Correzione di una voce precedente:** la
  riga di `remotek-1.4.9-1` sul token (sezione Sicurezza) diceva che il
  collegamento fra client e server "resta cifrato come prima": non era vero,
  con il server Remotek non lo e' mai stato, ne' prima ne' dopo.

## [remotek-1.4.9-1] - 2026-09-29
Base upstream: RustDesk 1.4.9. Primo rilascio con tag: exe Windows x86_64
**non firmato** (Windows SmartScreen puo' avvisare al primo avvio), sorgenti
al tag `remotek-1.4.9-1` di remotek-client e remotek-hbb-common.
**Per i tecnici**: rispetto all'exe di collaudo pubblicato il 2026-09-20
(commit `33328b45`) cambiano la frase della home, che non promette piu' una
password, e spariscono i comandi 2FA e "Visualizza telecamera", che non
funzionavano (Corretto). Il riquadro degli avvisi e' blu Infinitek
(Modificato), e lo STUN interroga solo `stun.infinitek.it` (Sicurezza).
Al telefono si fa leggere al cliente solo l'ID, e il cliente accetta la
finestra che compare. Si installa sopra la versione precedente.

### Modificato
- Proprieta' dei file Windows (exe interno e autoestraente): prodotto "Remotek", societa' "Infinitek S.r.l.", copyright con attribuzione a RustDesk; cartella di estrazione del portable separata da quella di RustDesk.
- Workflow upstream solo ad avvio manuale (nightly senza cron; ci, flutter-ci,
  flutter-tag, fdroid, wf-cliprdr-ci senza trigger automatici).
- I colori del tema vengono da `brand/brand.toml`: accento Blu Infinitek `#1F6BFF`, pulsanti `#2B7FFF`, sfondo del tema chiaro `#EEF3FB` e riquadro della qualita' di connessione `#060A14` dalla palette del sito; bordi e testo secondario restano quelli di RustDesk 1.4.9, piu' leggibili su fondo chiaro.
- Il riquadro degli avvisi della home (invito a installare il programma, installazione di versione precedente, errori di sistema) e' blu Infinitek a tinta piena, al posto del gradiente magenta/salmone di RustDesk: e' l'elemento piu' vistoso della prima schermata che vede il cliente e i suoi colori erano scritti a mano, fuori dal marchio. Il testo bianco dentro il riquadro passa da 3,7:1 e 2,8:1 di contrasto a 4,6:1 su tutta la larghezza, sopra la soglia di leggibilita' WCAG AA; il riquadro resta staccato dal fondo del pannello in tutti e due i temi (4,1:1 sul chiaro, 3,4:1 sullo scuro, sopra il 3:1 che WCAG chiede agli elementi non testuali). Nella stessa schermata restano colori di RustDesk scritti a mano piu' piccoli, che non sono del marchio e non si toccano qui: il grigio delle due iconcine accanto alla password e i colori delle spunte nella finestra che cambia la password.
- Submodule `libs/hbb_common` dal fork `labinfinitek/remotek-hbb-common`.
- Il client si chiama Remotek e nasce gia' configurato: server `remote.infinitek.it` con la sua chiave pubblica, API `https://remote.infinitek.it`; l'aggiornamento automatico verso RustDesk ufficiale e' spento e non riattivabile dalle impostazioni (hbb_common `d7da153`).
- Italiano di default; nel selettore restano italiano e inglese.
- Icone Infinitek al posto di quelle di RustDesk su Windows: exe da scaricare e installato, finestra, barra delle applicazioni, area di notifica, connection manager.
- Voce Disinstalla di Windows (App installate): l'editore e' "Infinitek S.r.l." e non piu' "Remotek", anche dopo l'aggiornamento di un'installazione precedente.
- Dialogo Informazioni: il riquadro del copyright dice "Copyright © 2026 Infinitek S.r.l." seguito dall'attribuzione a RustDesk e a Purslane Tech Pte. Ltd., come le proprieta' dell'exe; tolto lo slogan di RustDesk.

### Aggiunto
- Logo Infinitek nella home: simbolo e scritta nel tema scuro, solo il simbolo nel tema chiaro.
- Dialogo Informazioni: riga "Basato su RustDesk, AGPL-3.0 - sorgenti" con link al repo; privacy e sito puntano a `remotek.infinitek.it` (anche nel dialogo di installazione).
- `SECURITY.md`, `NOTICE`, `REMOTEK.md`, template di pull request.
- Workflow `remotek-controlli.yml` e script `verifica-patch.sh`: a ogni push e PR controllano che il client sia ancora Remotek (nome, server, chiave, API, accesso presidiato, i default di privacy spenti (comprese le due chiavi della registrazione) con il preset dei permessi bloccato, metadati ed editore, link e copyright, lingua, chiave che verifica `custom.txt`, tema, nessuna configurazione dal nome del file, nessuna chiamata automatica ai server RustDesk, il token dell'account fuori dai messaggi con cui si apre una sessione, i permessi spenti di fabbrica che un messaggio di rete non puo' riaccendere, il riquadro degli avvisi della home col colore del marchio, la scheda 2FA e le voci "Visualizza telecamera" che non si mostrano, la frase della home che segue i tre casi dell'accesso presidiato con le sue tre chiavi di lingua, con `allow-logon-screen-password` che resta spenta di fabbrica, lo STUN solo sul server Infinitek), che il changelog citi l'hbb_common a cui punta il submodule, che nessun workflow upstream parta da solo e che ogni file diverso da upstream sia elencato in `REMOTEK.md`.
- Workflow `remotek-build.yml`: build manuale dell'exe Windows x86_64 come artefatto, con attestazione di provenienza; nessuna release pubblica.

### Corretto
- La prima schermata non promette piu' una password che non esiste. Fino all'eseguibile precedente la home diceva "Puoi accedere a questo desktop usando l'ID e la password indicati qui sotto" mentre nel riquadro sotto, al posto della password monouso, c'era un trattino: con l'accettazione a clic la password non serve e non funziona, e il cliente al telefono la cercava. Ora la frase e': "Un tecnico puo' collegarsi a questo computer usando l'ID qui sotto. La connessione parte solo dopo che l'hai accettata." **Cosa dire al cliente adesso:** farsi leggere solo l'ID, e avvisarlo che sul suo schermo comparira' una finestra da accettare, altrimenti non entra nessuno; il trattino al posto della password e' corretto e non e' un guasto, e lo e' anche l'etichetta "Password monouso" che resta sopra il trattino (la si toglie in una correzione a parte, perche' cambia il riquadro e va ricollaudata): non c'e' nessuna password da cercare. **Non e' una scritta fissa:** segue i tre casi dell'impostazione che decide l'accettazione, quindi su un PC a cui un `custom.txt` firmato togliera' quell'accettazione (accesso non presidiato, per ora indisponibile) la frase cambia da sola, senza un eseguibile nuovo: nomina ID e password dove la password e' l'unica via, e dice tutte e due le cose ("se ha la password entra subito, altrimenti accetti tu") dove il PC accetta in tutti e due i modi. **Limite noto:** a servizio fermo la frase dice ancora che un tecnico puo' collegarsi; e' l'imprecisione che aveva anche la frase di prima, e nel pannello di destra resta l'avviso "Service is not running" con il pulsante per riavviarlo. Scritta in italiano e in inglese; le altre lingue del selettore leggono l'inglese.
- I due comandi che non potevano funzionare non si mostrano piu'. **Verifica in due passaggi (2FA)**: l'interruttore in Impostazioni > Sicurezza c'era e non accendeva niente, perche' la 2FA e' spenta nell'eseguibile e non si riaccende; ora la scheda "2FA" non compare. La chiave non e' toccata: sparisce il comando, non la verifica, e in collaudo la prova che la 2FA resta spenta e' `Remotek.exe --option 2fa | more`, che stampa una riga vuota, al posto del clic sulla casella che non c'e' piu'. **"Visualizza telecamera"**: la voce c'era nel menu di ogni scheda PC, nel menu accanto al pulsante Connetti della home e nella barra di una sessione aperta, e apriva una connessione che il PC del cliente rifiutava sempre con un messaggio in inglese non tradotto ("No permission of viewing camera"); ora non compare in nessuno dei tre punti. **Se un giorno la videocamera si riaprisse per un cliente** con un `custom.txt` firmato, la voce torna da sola: il controllo non e' "nascondi sempre", e' "nascondi finche' la chiave `enable-camera` resta bloccata e spenta in questo eseguibile". **Nascoste nei menu, non tolte dal codice**: la sessione videocamera si apre ancora con `Remotek.exe --view-camera <ID>` o con un link `remotek://`, e il rifiuto "No permission of viewing camera" resta cosi' riproducibile in collaudo. **Limite**: quel controllo guarda la configurazione del PC di chi controlla, non quella del PC controllato, che il tecnico non puo' conoscere; siccome ogni Remotek e' lo stesso eseguibile le due coincidono, ma se la videocamera fosse riaperta per un cliente servirebbe un `custom.txt` firmato **anche per il client del tecnico**, con la **sola** chiave `enable-camera` (quello emesso per il cliente gli porterebbe dietro tutte le altre impostazioni di quel cliente), altrimenti a lui la voce resta nascosta. Finche' la videocamera resta bloccata in tutti gli eseguibili non si perde nessuna funzione: oggi i due comandi fallivano comunque.
- Lingua: tolta dal selettore la voce "Predefinita", che seguiva la lingua di Windows: sceglierla, su un Windows francese, tedesco o spagnolo, faceva parlare al client quella lingua. Restano italiano, che vale se non si sceglie nulla, e inglese.
- Installazione su Windows: il pacchetto contiene `Remotek.exe` e non piu' `rustdesk.exe`, cosi' servizio, collegamenti, disinstallazione e "e' installato?" trovano il programma in `C:\Program Files\Remotek`.

### Sicurezza
- Accesso presidiato di default: ogni sessione in entrata va accettata con un clic sul PC controllato; ID e password (monouso o permanente) da soli non bastano piu'. L'impostazione e' bloccata: non si cambia da impostazioni, riga di comando o API. La verifica in due passaggi (2FA) e' spenta, perche' con l'accettazione a clic non aggiunge protezione e il suo codice aprirebbe la sessione senza clic: l'interruttore non ha effetto e da questo eseguibile in poi non compare piu' in Impostazioni > Sicurezza (vedi "Corretto"). L'accesso non presidiato si abilitera' solo per singolo cliente con un `custom.txt` firmato; anche con un `custom.txt` la 2FA resta spenta, in ogni modalita'. L'accettazione a clic la puo' togliere solo un `custom.txt` firmato da Infinitek (riga seguente; hbb_common `d7da153`).
- Quattro funzioni spente di fabbrica e menu Permessi bloccato su "Personalizzato": **rilevamento in rete locale**, **registrazione della sessione**, **modalita' privacy** e **videocamera**. Non le riaccendono le impostazioni, la riga di comando, il servizio ne' una strategia della console; da questo eseguibile in poi nemmeno un messaggio che arriva dalla rete (ultima riga di questa sezione). Gli interruttori in Impostazioni > Sicurezza restano visibili e non si cambiano piu', e i preset "Accesso completo" e "Solo visualizzazione" non si scelgono piu' (con "Accesso completo" il PC tornava a concedere registrazione, modalita' privacy e videocamera). Chi aveva scelto uno dei due preset torna alle singole caselle, con i valori di fabbrica, e chi aveva "Accesso completo" perde anche una cosa che quel preset concedeva senza dirlo: la **modifica remota delle impostazioni** durante la sessione. Il tecnico che apre le Impostazioni o la home del PC controllato le trova coperte dalla maschera grigia di blocco — salvo il caso descritto in fondo all'ultima riga di questa sezione, in cui la maschera non compare; per riaverle basta che sia il cliente a spuntare "Abilita modifica remota configurazione", che e' una casella libera e non una delle sei bloccate. In dettaglio: **rete locale** — il PC non risponde piu' a chi lo cerca sulla rete e non compare nella scheda "Rilevate" di nessun altro: era l'unico caso in cui indirizzo MAC, ID, nome del PC e utente connesso uscivano verso chiunque fosse sulla stessa rete, senza passare dal nostro server; la scansione dal nostro client parte ancora, ma nessun Remotek risponde, quindi la scheda "Rilevate" sara' vuota su una rete di soli Remotek e il messaggio in uscita non porta dati del PC; la porta su cui il PC sta in ascolto resta pero' aperta come in RustDesk, semplicemente non risponde piu' a nessuno. **Registrazione** — il filmato dello schermo del cliente non lo avvia piu' ne' il tecnico dalla sua barra ne' il PC controllato per conto suo; resta solo il clic del cliente sulla finestra di accettazione, per la singola sessione (in fondo a questa riga). Le strade erano due: oltre al permesso di registrazione c'era la registrazione automatica delle sessioni in entrata, che partiva senza passare dai permessi, non accendeva l'icona della telecamera nella finestra di accettazione e si attivava su tutto il parco con una riga di strategia dalla console; ora e' bloccata anche quella. Il prezzo lo paga il cliente che voleva registrare per proprio audit le sessioni che riceve: la casella "Registra automaticamente le sessioni in entrata" in Impostazioni > Registrazione resta visibile e non si cambia piu', e per riaverla serve un `custom.txt` firmato. **Modalita' privacy** — il tecnico non puo' piu' oscurare lo schermo del PC controllato mentre ci lavora. **Videocamera** — la webcam del PC controllato non si apre piu': su un PC aziendale sarebbe videosorveglianza. Non c'e' modo di concederla: la finestra di accettazione non ha un'icona per la webcam. La voce "Visualizza telecamera", che fino all'eseguibile precedente restava nei menu e falliva sempre con un messaggio in inglese non tradotto, da questo eseguibile in poi non compare (vedi "Corretto"). Restano due sole strade per riaverle: per la **singola sessione**, registrazione e modalita' privacy le concede **il cliente e solo lui**, con le due icone della finestra "Connessione" che chiede di accettare (videocamera e rete locale no); per **singolo cliente**, un `custom.txt` firmato da Infinitek e una riga nel DPA, che pero' resta indisponibile finche' non c'e' lo strumento di firma. La **cattura schermata** non e' coperta da nessun default e resta possibile: si governa con la regola interna (solo se serve al ticket, allegata al ticket, 30 giorni). (hbb_common `d7da153`)
- `custom.txt` vale solo se firmato da Infinitek: il client ne verifica la firma con la chiave pubblica Remotek al posto di quella di RustDesk. Da questo eseguibile in poi un `custom.txt` firmato da RustDesk non viene piu' applicato — nessuno ne ha uno e il nostro flusso non li usa — e valgono solo i nostri; un file senza firma valida resta ignorato in silenzio e restano i default del binario. E' il primo passo dell'accesso non presidiato per singolo cliente, che resta indisponibile finche' non c'e' lo strumento di firma. Un `custom.txt` firmato vale pero' su qualunque installazione Remotek, non solo su quella del cliente per cui e' stato emesso: il client ne verifica la firma e nient'altro, non scade e non si revoca; per toglierlo di mezzo servono una chiave nuova e un eseguibile nuovo per tutti. Va trattato come una password: si consegna al cliente e non si lascia in giro.
- Il nome del file non cambia piu' server, chiave, API o relay: un exe rinominato con `host=`, `key=`, `api=` o `relay=` (o con la configurazione codificata in base64 nel nome, anche non firmata) usa comunque il server e la chiave di Remotek, anche come portable. Restano gli usi del nome che non toccano il server: supporto rapido (`-qs`) e `install.exe`.
- Il token dell'account del tecnico non viaggia piu' nei messaggi con cui si apre una sessione. Prima il client del tecnico metteva il token del suo account Remotek in due messaggi diretti al server (`PunchHoleRequest` e `RequestRelay`): quel token non apre solo la rubrica e l'elenco dei PC, apre anche il pannello di amministrazione dell'API, e il secondo dei due messaggi il server lo gira al PC del cliente, cioe' fuori dalle macchine nostre. Ora i due campi partono vuoti, esattamente come li manda un client che non ha fatto login all'API. **Cosa non cambia:** niente di quello che fa il tecnico. Il collegamento fra client e server resta cifrato come prima (il token serve anche a decidere di cifrarlo, e quel pezzo non e' stato toccato); login, rubrica, elenco dei PC, connessione diretta e connessione via relay funzionano come prima; il messaggio di controllo che il client manda solo al nostro server continua a portare il token, perche' quella connessione e' cifrata sempre e non viene girata a nessuno. Nessuna funzione persa: un client senza login all'API manda gia' quei campi vuoti ed e' il caso normale in RustDesk. **Limite:** il programma del server (hbbs) non e' nostro e i suoi sorgenti non stanno nei nostri repo, quindi non abbiamo potuto osservare cosa ne faccia; questa e' una lettura del codice del client, che pero' rende la domanda irrilevante, perche' ora non c'e' piu' niente da inoltrare.
- Un permesso spento di fabbrica non lo riaccende piu' un messaggio che arriva dalla rete. I messaggi con cui si apre una sessione (`PunchHole`, `RequestRelay`, `FetchLocalAddr`) possono portare un elenco di permessi, e RustDesk 1.4.9 lo fa vincere sulle impostazioni del PC controllato: se quell'elenco diceva "concedi", videocamera, registrazione della sessione e modalita' privacy tornavano disponibili senza che il client leggesse nessuna delle sei chiavi bloccate nell'eseguibile, e la finestra di accettazione nasceva con il permesso gia' dato, senza che il cliente avesse toccato le sue icone. Ora, per le chiavi bloccate, conta anche il valore dell'eseguibile: dalla rete un permesso si puo' solo togliere, mai dare. **Cosa non cambia:** per le chiavi non bloccate il comportamento e' quello di RustDesk; il cliente continua a concedere registrazione e modalita' privacy dalle icone della finestra di accettazione, per la singola sessione; un "non concedere" che arriva dal server continua a valere. **Sulla riga delle quattro funzioni spente di fabbrica** (la seconda di questa sezione)**:** l'elenco delle strade che non riaccendono le quattro funzioni ("le impostazioni, la riga di comando, il servizio ne' una strategia della console") non nominava questa, che era l'unica che scavalcava davvero i default; era incompleto ed e' stato corretto insieme a questa riga. **Limite:** il programma del server (hbbs) non e' nostro e i suoi sorgenti non stanno nei nostri repo, quindi non sappiamo se quel campo lo riempia o lo inoltri; questa e' una lettura del codice del client, che pero' rende la domanda irrilevante, perche' ora quel campo non puo' piu' concedere niente. **Un secondo limite, che questa correzione non chiude:** lo stesso elenco di permessi ha un'altra strada, che non passa da qui e puo' dire "concedi la modifica remota della configurazione". Quando lo dice, durante la sessione le Impostazioni e la home del PC controllato non si coprono piu' con la maschera grigia di blocco di cui parla quella stessa riga. Nessuna delle quattro funzioni spente di fabbrica si riaccende da li' — quelle sei chiavi restano non scrivibili, e il cliente resta l'unico a poter concedere registrazione e modalita' privacy — ma una protezione che quella riga da' per sempre attiva non lo e' sempre; si chiude, se si chiude, in una correzione a parte.
- L'indirizzo IP pubblico del PC non va piu' a Google, Cloudflare e Nextcloud. Per scoprire il proprio indirizzo pubblico (serve alla connessione diretta fra i due PC) il client interrogava tre server STUN pubblici quando si apre una sessione, dal PC del tecnico e da quello del cliente; ora interroga solo `stun.infinitek.it`, sul server Remotek. Se quel server non risponde la connessione diretta puo' non riuscire e la sessione passa dal relay, come gia' succede dietro reti che non la permettono. Nell'informativa e nella SBOM i tre fornitori di STUN escono con questo eseguibile.
