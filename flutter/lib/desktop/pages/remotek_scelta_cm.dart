// Remotek (ADR-0028, caso 2): l'agente AI chiede i permessi di
// amministratore con un lasciapassare valido, e il cliente sceglie fra tre:
// accetta come amministratore (la shell gira come SYSTEM), accetta come
// utente (la shell col suo token abbassato, come oggi), rifiuta.
// Testo e pulsanti arrivano gia' nella lingua del connection manager, nel
// campo `remotek` del client (src/remotek/scelta_terminale.rs, `Scheda`), in
// `finestra` (`Finestra`); senza quel campo la finestra resta quella di
// upstream. In `etichetta` c'e' cosa dice la scheda al posto di "Terminale"
// (con exec "Esecuzione di comandi", ADR-0033), dal lasciapassare verificato. Sul client non
// installato elevato `accetta_utente` e' null: due scelte, senza "accetta
// come utente".
// Con exec (ADR-0035) in `script` c'e' lo script del login nella forma
// visibile del riquadro "Script" della vista (`vista_cm::FormaVisibile`):
// `remotekConScript` lo mette sopra i pulsanti della richiesta, sia quelli di
// upstream sia quelli della scelta dell'amministratore, cosi' il cliente lo
// legge intero prima di accettare; il PC esegue solo quello.

import 'package:flutter/material.dart';
import 'package:get/get.dart';

class RemotekScelta {
  final String testo;
  final String accettaAmministratore;
  final String? accettaUtente;
  final String rifiuta;

  RemotekScelta._(
      this.testo, this.accettaAmministratore, this.accettaUtente, this.rifiuta);

  static RemotekScelta? daJson(dynamic remotek) {
    if (remotek is! Map<String, dynamic>) return null;
    final json = remotek['finestra'];
    if (json is! Map<String, dynamic>) return null;
    final campi = [
      json['testo'],
      json['accetta_amministratore'],
      json['rifiuta'],
    ];
    final utente = json['accetta_utente'];
    if (campi.any((c) => c is! String) || (utente != null && utente is! String)) {
      return null;
    }
    return RemotekScelta._(campi[0], campi[1], utente, campi[2]);
  }
}

// Il tipo della connessione nella scheda del cliente al posto di
// "Terminale", o null.
String? remotekEtichetta(dynamic remotek) {
  if (remotek is! Map<String, dynamic>) return null;
  final etichetta = remotek['etichetta'];
  return etichetta is String ? etichetta : null;
}

typedef RemotekPulsante = Widget Function(
    {required Color? color,
    required VoidCallback onClick,
    required String text,
    required Color? textColor,
    BoxBorder? border});

// "Accetta come amministratore" manda prima la scelta al servizio
// (`sceltaAmministratore`, cioe' bind.cmSceltaAmministratore) e ne aspetta la
// fine, poi l'accettazione di sempre (`accetta`): cosi' sul canale della
// connessione la scelta arriva prima dell'Authorize. Il servizio da' la shell
// da amministratore (SYSTEM sul PC installato, il token del processo sul
// portatile elevato) solo se la connessione ha il lasciapassare valido;
// altrimenti vale come utente, e sul portatile elevato si rifiuta.
Widget remotekSceltaCm(
    {required RemotekScelta scelta,
    required RemotekPulsante pulsante,
    required Color? coloreAccetta,
    required Future<void> Function() sceltaAmministratore,
    required VoidCallback accetta,
    required VoidCallback rifiuta}) {
  return Column(
    mainAxisAlignment: MainAxisAlignment.end,
    children: [
      Text(
        scelta.testo,
        textAlign: TextAlign.center,
        style: const TextStyle(fontWeight: FontWeight.bold),
      ).marginOnly(bottom: 8),
      pulsante(
        color: Colors.red[700],
        onClick: () async {
          await sceltaAmministratore();
          accetta();
        },
        text: scelta.accettaAmministratore,
        textColor: Colors.white,
      ),
      if (scelta.accettaUtente != null)
        pulsante(
          color: coloreAccetta,
          onClick: accetta,
          text: scelta.accettaUtente!,
          textColor: Colors.white,
        ),
      pulsante(
        color: Colors.transparent,
        border: Border.all(color: Colors.grey),
        onClick: rifiuta,
        text: scelta.rifiuta,
        textColor: null,
      ),
    ],
  );
}

// Lo script di exec sopra `pulsanti`; senza script (non exec) solo
// `pulsanti`, come prima. Il riquadro prende lo spazio che resta sopra i
// pulsanti e scorre: il CM non si allarga. Le righe lunghe vanno a capo.
Widget remotekConScript(dynamic remotek, Widget pulsanti) {
  if (remotek is! Map<String, dynamic>) return pulsanti;
  final script = remotek['script'];
  if (script is! String) return pulsanti;
  final vuoto = remotek['script_vuoto'];
  return Column(
    mainAxisAlignment: MainAxisAlignment.end,
    crossAxisAlignment: CrossAxisAlignment.stretch,
    children: [
      // Il titolo del riquadro dello script della vista
      // (remotek_terminale_cm.dart), uguale nelle due lingue.
      const Text('Script', style: TextStyle(fontWeight: FontWeight.bold))
          .marginOnly(top: 4, bottom: 4),
      Flexible(
        child: _RemotekScript(
            testo: script.isEmpty && vuoto is String
                ? vuoto
                // Nella forma visibile `\r` da solo e' gia' `^M`: resta
                // solo l'a capo.
                : script.replaceAll('\r\n', '\n')),
      ),
      pulsanti.marginOnly(top: 8),
    ],
  );
}

class _RemotekScript extends StatefulWidget {
  final String testo;
  const _RemotekScript({required this.testo});

  @override
  State<_RemotekScript> createState() => _RemotekScriptState();
}

class _RemotekScriptState extends State<_RemotekScript> {
  final _scorrimento = ScrollController();

  @override
  void dispose() {
    _scorrimento.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Container(
      decoration: BoxDecoration(
        border: Border.all(color: Colors.grey),
        borderRadius: BorderRadius.circular(4),
      ),
      child: Scrollbar(
        controller: _scorrimento,
        thumbVisibility: true,
        child: SingleChildScrollView(
          controller: _scorrimento,
          padding: const EdgeInsets.all(6),
          child: SelectableText(
            widget.testo,
            style: const TextStyle(fontFamily: 'monospace', fontSize: 12),
          ),
        ),
      ),
    );
  }
}
