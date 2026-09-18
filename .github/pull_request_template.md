## Cosa cambia
<una frase per il titolare; poi l'elenco puntato di cosa entra e cosa no>

## Perche'
<il motivo; riferimento a ADR-NNNN, issue #N, punto del PIANO>

## Come l'ho verificato
- CI: <link, verde>
- Test: <comando e output incollato, ridotto alle righe che contano>
- Build manuale (client): <link alla run> / non applicabile
- Collaudo su 109: <casi eseguiti ed esito, file in docs/collaudo/esiti/> / non eseguito

## Cosa NON ho verificato e perche'
- <es. test del contratto contro la 111 non raggiungibile dal runner: eseguiti a mano, output sopra>

## Cosa serve
- dal titolare: <decisione o "niente">
- da setup-lab (gia' in HANDOFF.md): <riga o "niente">

## Sicurezza (solo se la MR tocca autenticazione, rete, file, configurazione)
- [ ] input validato e limitato
- [ ] autenticazione e autorizzazione su ogni rotta nuova
- [ ] nessun `err.Error()` interno verso il client
- [ ] log senza segreti e senza dati personali superflui
- [ ] dipendenze nuove giustificate
- [ ] default sicuri; impatto sul contratto col client dichiarato

## Checklist
- [ ] un argomento, <= 300 righe (o motivo: ...)
- [ ] changelog "Non rilasciato" aggiornato (o non serve: refactor/test/ci)
- [ ] REMOTEK.md aggiornato se cambia l'inventario della patch
- [ ] ADR scritto/aggiornato se la MR decide qualcosa
- [ ] README aggiornato se cambia comando, variabile o percorso
- [ ] nessun segreto, nessun file generato dimenticato
