// Remotek (ADR-0021, regola 6): nel connection manager il cliente vede in
// diretta, in sola lettura, l'output delle sessioni terminale aperte sul suo
// PC, dalla prima riga. I dati arrivano dal servizio con l'evento
// `cm_file_transfer_log` e l'azione `remotek-terminale`
// (src/remotek/vista_cm.rs): un JSON con conn_id, terminal_id, dir e data in
// base64. Nessun input parte da qui; per chiudere c'e' il pulsante
// "Disconnect" del CM.

import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:get/get.dart';
import 'package:xterm/xterm.dart';

import '../../common.dart';

// La chiave del riquadro dello script di exec: vista_cm::RIQUADRO_SCRIPT
// (src/remotek/vista_cm.rs), fuori dai terminal_id possibili (int32), cosi'
// nessun output ci arriva.
const _riquadroScript = -2147483649;
// Il terminal_id di stderr di exec: exec::TERMINAL_ID_STDERR
// (src/remotek/exec.rs).
const _riquadroStderr = -2;

// Il titolo di un riquadro. Con exec (la connessione ha il riquadro dello
// script) i riquadri sono lo script, l'output (stdout, il terminal_id che
// apre il CLI) e gli errori (stderr); col terminale il numero del terminale,
// se ce n'e' piu' d'uno.
String _titoloRiquadro(String titolo, int riquadro, List<int> riquadri) {
  if (riquadri.contains(_riquadroScript)) {
    if (riquadro == _riquadroScript) return '$titolo - Script';
    if (riquadro == _riquadroStderr) return '$titolo - ${translate('Error')}';
    return '$titolo - Output';
  }
  return riquadri.length > 1 ? '$titolo ($riquadro)' : titolo;
}

class _TerminaleCm {
  final terminal = Terminal(maxLines: 10000);
  late final ByteConversionSink _utf8;

  _TerminaleCm() {
    // I pezzi possono spezzare un carattere UTF-8: la decodifica e' a flusso.
    _utf8 = const Utf8Decoder(allowMalformed: true)
        .startChunkedConversion(_ScriviTerminale(terminal));
  }

  void scrivi(List<int> dati) => _utf8.add(dati);
}

class _ScriviTerminale extends StringConversionSinkBase {
  final Terminal terminal;
  _ScriviTerminale(this.terminal);

  @override
  void addSlice(String str, int start, int end, bool isLast) {
    terminal.write(str.substring(start, end));
  }

  @override
  void close() {}
}

class RemotekTerminaleCm {
  RemotekTerminaleCm._();
  static final istanza = RemotekTerminaleCm._();

  static const azione = 'remotek-terminale';

  // conn_id -> terminal_id -> terminale, nell'ordine in cui sono arrivati.
  final Map<int, Map<int, _TerminaleCm>> _terminali = {};
  // Cambia quando compare un terminale nuovo, per ridisegnare la vista.
  final versione = 0.obs;

  void onEvento(Map<String, dynamic> evt) {
    final testo = evt[azione];
    if (testo == null) return;
    try {
      final d = jsonDecode(testo);
      if (d['dir'] != 'out') return;
      final int connId = d['conn_id'];
      final int terminalId = d['terminal_id'];
      final perConn = _terminali.putIfAbsent(connId, () => {});
      var t = perConn[terminalId];
      if (t == null) {
        t = _TerminaleCm();
        perConn[terminalId] = t;
        versione.value++;
        _apriPannello(connId);
      }
      t.scrivi(base64Decode(d['data']));
    } catch (e) {
      debugPrint('remotek-terminale: $e');
    }
  }

  // Il cliente vede la sessione senza dover cliccare: si apre il pannello a
  // lato, se e' chiuso e la scheda scelta e' quella della connessione.
  void _apriPannello(int connId) {
    final scelta = gFFI.serverModel.tabController.state.value.selectedTabInfo;
    if (!gFFI.chatModel.isShowCMSidePage && scelta.key == connId.toString()) {
      gFFI.chatModel.toggleCMFilePage();
    }
  }

  List<MapEntry<int, Terminal>> terminali(int connId) =>
      (_terminali[connId] ?? {})
          .entries
          .map((e) => MapEntry(e.key, e.value.terminal))
          .toList();
}

/// La vista nel pannello a lato del CM per un cliente di tipo terminale.
class RemotekVistaTerminale extends StatelessWidget {
  final int connId;
  const RemotekVistaTerminale({Key? key, required this.connId})
      : super(key: key);

  @override
  Widget build(BuildContext context) {
    final modello = RemotekTerminaleCm.istanza;
    return Obx(() {
      modello.versione.value;
      final terminali = modello.terminali(connId);
      final titolo = '${translate('Terminal')} - ${translate('Read-only')}';
      if (terminali.isEmpty) {
        return Center(child: Text(titolo));
      }
      return Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          for (final t in terminali) ...[
            Text(_titoloRiquadro(
                    titolo, t.key, [for (final r in terminali) r.key]))
                .paddingSymmetric(horizontal: 8, vertical: 4),
            Expanded(
              child: TerminalView(
                t.value,
                readOnly: true,
                hardwareKeyboardOnly: true,
                backgroundOpacity: 0.9,
              ),
            ),
          ],
        ],
      );
    });
  }
}
