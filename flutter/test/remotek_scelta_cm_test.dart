import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_hbb/desktop/pages/remotek_scelta_cm.dart';

// Remotek (ADR-0028, caso 2): la finestra della scelta dell'amministratore.
void main() {
  final json = <String, dynamic>{
    'agente': 'agente-mario',
    'tecnico': 'mario',
    'testo':
        'agente-mario, agente AI di mario, chiede i permessi di amministratore',
    'accetta_amministratore': 'Accetta come amministratore',
    'accetta_utente': 'Accetta come utente',
    'rifiuta': 'Rifiuta',
  };

  // Il campo `remotek` del client: `Scheda` di scelta_terminale.rs.
  Map<String, dynamic> scheda(Map<String, dynamic>? finestra,
          {String? etichetta}) =>
      {'etichetta': etichetta, 'finestra': finestra};

  test('senza il campo remotek la finestra resta quella di upstream', () {
    expect(RemotekScelta.daJson(null), isNull);
    expect(RemotekScelta.daJson(scheda({'testo': 'x'})), isNull);
    expect(RemotekScelta.daJson(json), isNull);
    expect(RemotekScelta.daJson(scheda(json)), isNotNull);
  });

  test('con exec la scheda dice cosa chiede l\'agente', () {
    final exec = scheda(null, etichetta: 'Esecuzione di comandi');
    expect(remotekEtichetta(exec), 'Esecuzione di comandi');
    expect(RemotekScelta.daJson(exec), isNull);
    expect(remotekEtichetta(scheda(json)), isNull);
    expect(remotekEtichetta(null), isNull);
    expect(remotekEtichetta({'etichetta': 1}), isNull);
  });

  testWidgets('tre scelte, e solo la prima manda la richiesta di SYSTEM',
      (tester) async {
    final eventi = <String>[];
    await tester.pumpWidget(MaterialApp(
      home: Scaffold(
        body: remotekSceltaCm(
          scelta: RemotekScelta.daJson(scheda(json))!,
          pulsante: (
                  {required color,
                  required onClick,
                  required text,
                  required textColor,
                  border}) =>
              TextButton(onPressed: onClick, child: Text(text)),
          coloreAccetta: Colors.blue,
          sceltaAmministratore: () async => eventi.add('amministratore'),
          accetta: () => eventi.add('accetta'),
          rifiuta: () => eventi.add('rifiuta'),
        ),
      ),
    ));

    expect(find.text(json['testo']), findsOneWidget);
    await tester.tap(find.text('Accetta come amministratore'));
    expect(eventi, ['amministratore', 'accetta']);
    eventi.clear();
    await tester.tap(find.text('Accetta come utente'));
    expect(eventi, ['accetta']);
    eventi.clear();
    await tester.tap(find.text('Rifiuta'));
    expect(eventi, ['rifiuta']);
  });

  testWidgets('sul portatile elevato due scelte, senza accetta come utente',
      (tester) async {
    final due = Map<String, dynamic>.from(json)..['accetta_utente'] = null;
    expect(RemotekScelta.daJson(scheda(due))!.accettaUtente, isNull);
    expect(RemotekScelta.daJson(scheda(Map.of(due)..['accetta_utente'] = 1)), isNull);
    await tester.pumpWidget(MaterialApp(
      home: Scaffold(
        body: remotekSceltaCm(
          scelta: RemotekScelta.daJson(scheda(due))!,
          pulsante: (
                  {required color,
                  required onClick,
                  required text,
                  required textColor,
                  border}) =>
              TextButton(onPressed: onClick, child: Text(text)),
          coloreAccetta: Colors.blue,
          sceltaAmministratore: () async {},
          accetta: () {},
          rifiuta: () {},
        ),
      ),
    ));
    expect(find.byType(TextButton), findsNWidgets(2));
    expect(find.text('Accetta come utente'), findsNothing);
  });

  testWidgets('l\'accettazione parte solo dopo la scelta di amministratore',
      (tester) async {
    final eventi = <String>[];
    final scelta = Completer<void>();
    await tester.pumpWidget(MaterialApp(
      home: Scaffold(
        body: remotekSceltaCm(
          scelta: RemotekScelta.daJson(scheda(json))!,
          pulsante: (
                  {required color,
                  required onClick,
                  required text,
                  required textColor,
                  border}) =>
              TextButton(onPressed: onClick, child: Text(text)),
          coloreAccetta: Colors.blue,
          sceltaAmministratore: () {
            eventi.add('amministratore');
            return scelta.future;
          },
          accetta: () => eventi.add('accetta'),
          rifiuta: () => eventi.add('rifiuta'),
        ),
      ),
    ));
    await tester.tap(find.text('Accetta come amministratore'));
    await tester.pump();
    expect(eventi, ['amministratore']);
    scelta.complete();
    await tester.pump();
    expect(eventi, ['amministratore', 'accetta']);
  });
}
