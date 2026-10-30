// Remotek (ADR-0028, caso 2): l'agente AI chiede i permessi di
// amministratore con un lasciapassare valido, e il cliente sceglie fra tre:
// accetta come amministratore (la shell gira come SYSTEM), accetta come
// utente (la shell col suo token abbassato, come oggi), rifiuta.
// Testo e pulsanti arrivano gia' nella lingua del connection manager, nel
// campo `remotek` del client (src/remotek/scelta_terminale.rs, `Finestra`);
// senza quel campo la finestra resta quella di upstream. Sul client non
// installato elevato `accetta_utente` e' null: due scelte, senza "accetta
// come utente".

import 'package:flutter/material.dart';
import 'package:get/get.dart';

class RemotekScelta {
  final String testo;
  final String accettaAmministratore;
  final String? accettaUtente;
  final String rifiuta;

  RemotekScelta._(
      this.testo, this.accettaAmministratore, this.accettaUtente, this.rifiuta);

  static RemotekScelta? daJson(dynamic json) {
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
