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
    expect(RemotekScelta.daJson(scheda(Map.of(due)..['accetta_utente'] = 1)),
        isNull);
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

  // ADR-0035: lo script di exec sopra i pulsanti della richiesta.
  Widget conScript(dynamic remotek, {double altezza = 300}) => MaterialApp(
        home: Scaffold(
          body: Align(
            alignment: Alignment.bottomCenter,
            child: SizedBox(
              width: 300,
              height: altezza,
              child: remotekConScript(remotek,
                  TextButton(onPressed: () {}, child: const Text('Accetta'))),
            ),
          ),
        ),
      );

  testWidgets('senza script solo i pulsanti, come prima', (tester) async {
    for (final remotek in [
      null,
      scheda(json),
      {'script': 1}
    ]) {
      await tester.pumpWidget(conScript(remotek));
      expect(find.text('Script'), findsNothing);
      expect(find.text('Accetta'), findsOneWidget);
    }
  });

  testWidgets('lo script sopra i pulsanti, selezionabile, vuoto lo dice',
      (tester) async {
    await tester.pumpWidget(conScript(
        {'etichetta': 'Esecuzione di comandi', 'script': 'Get-Date\r\n^[[2J'}));
    expect(find.text('Script'), findsOneWidget);
    expect(find.byType(SelectableText), findsOneWidget);
    expect(tester.widget<SelectableText>(find.byType(SelectableText)).data,
        'Get-Date\n^[[2J');
    expect(
        tester.getTopLeft(find.byType(SelectableText)).dy <
            tester.getTopLeft(find.text('Accetta')).dy,
        isTrue);
    await tester.pumpWidget(
        conScript({'script': '', 'script_vuoto': '(script vuoto)'}));
    expect(find.text('(script vuoto)'), findsOneWidget);
  });

  testWidgets('uno script lungo scorre e non allarga il CM', (tester) async {
    final lungo =
        List.generate(2000, (i) => 'riga $i ${'x' * 200}').join('\r\n');
    await tester.pumpWidget(conScript({'script': lungo}, altezza: 200));
    expect(tester.takeException(), isNull);
    expect(tester.getSize(find.byType(SingleChildScrollView)).height,
        lessThan(200));
    expect(find.text('Accetta'), findsOneWidget);
  });
}
