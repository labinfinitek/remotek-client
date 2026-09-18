#!/usr/bin/env python3
"""Genera flutter/lib/brand.g.dart da brand/brand.toml (ADR-0004).

Uso:
    python3 brand/genera_tema.py          scrive flutter/lib/brand.g.dart
    python3 brand/genera_tema.py --check  non scrive; exit 1 se il file
                                          committato non e' aggiornato

Codici di uscita: 0 ok, 1 file generato assente o non aggiornato (--check),
2 brand.toml non valido. Solo libreria standard, Python >= 3.11 (tomllib).
L'output e' deterministico: dipende solo da brand.toml (niente date, niente
percorsi, ordine dei campi fissato qui sotto, fine riga LF).
"""

import argparse
import difflib
import re
import sys
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # Python < 3.11
    sys.exit("genera_tema: serve Python >= 3.11 (modulo tomllib)")

RADICE = Path(__file__).resolve().parent.parent
SORGENTE = RADICE / "brand" / "brand.toml"
DESTINAZIONE = RADICE / "flutter" / "lib" / "brand.g.dart"

# Le 12 costanti di MyTheme (flutter/lib/common.dart), nello stesso ordine.
# L'ordine dell'output viene da qui, non dall'ordine delle chiavi nel TOML.
CAMPI = (
    "grayBg",
    "accent",
    "accent50",
    "accent80",
    "canvasColor",
    "border",
    "idColor",
    "darkGray",
    "cmIdColor",
    "dark",
    "button",
    "hoverBorder",
)

# Varianti trasparenti dell'accento: stesso RRGGBB di "accent".
DERIVATI_DA_ACCENT = ("accent50", "accent80")

OPACO = re.compile(r"#([0-9A-Fa-f]{6})")
CON_ALFA = re.compile(r"0[xX]([0-9A-Fa-f]{8})")

INTESTAZIONE = """\
// GENERATO da brand/brand.toml, non modificare.
// Per cambiare un colore: modificare brand/brand.toml e rigenerare con
//   python3 brand/genera_tema.py
// Il controllo `python3 brand/genera_tema.py --check` gira in CI.

import 'dart:ui' show Color;

class Brand {
  Brand._();

"""


class ErroreBrand(Exception):
    """brand.toml non valido."""


def normalizza(nome: str, valore: object) -> str:
    """Restituisce AARRGGBB maiuscolo (8 cifre) o solleva ErroreBrand."""
    if not isinstance(valore, str):
        raise ErroreBrand(f"colori.{nome}: deve essere una stringa")
    if m := OPACO.fullmatch(valore):
        return "FF" + m.group(1).upper()
    if m := CON_ALFA.fullmatch(valore):
        return m.group(1).upper()
    raise ErroreBrand(
        f'colori.{nome}: "{valore}" non valido; usare "#RRGGBB" o "0xAARRGGBB"'
    )


def leggi() -> dict[str, str]:
    try:
        with SORGENTE.open("rb") as f:
            dati = tomllib.load(f)
    except (OSError, tomllib.TOMLDecodeError) as e:
        raise ErroreBrand(f"{SORGENTE}: {e}") from e

    nome = dati.get("nome")
    if not isinstance(nome, str) or not nome.strip():
        raise ErroreBrand('manca nome = "..." (stringa non vuota)')

    colori = dati.get("colori")
    if not isinstance(colori, dict):
        raise ErroreBrand("manca la tabella [colori]")
    mancanti = [c for c in CAMPI if c not in colori]
    estranei = sorted(set(colori) - set(CAMPI))
    if mancanti:
        raise ErroreBrand("[colori]: mancano " + ", ".join(mancanti))
    if estranei:
        raise ErroreBrand("[colori]: chiavi in piu' " + ", ".join(estranei))

    valori = {c: normalizza(c, colori[c]) for c in CAMPI}
    for c in DERIVATI_DA_ACCENT:
        if valori[c][2:] != valori["accent"][2:]:
            raise ErroreBrand(
                f"colori.{c}: deve avere lo stesso RRGGBB di accent "
                f"({valori['accent'][2:]}), cambia solo l'alfa"
            )
    return valori


def genera(valori: dict[str, str]) -> str:
    righe = "".join(
        f"  static const Color {c} = Color(0x{valori[c]});\n" for c in CAMPI
    )
    return INTESTAZIONE + righe + "}\n"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--check",
        action="store_true",
        help="non scrive; exit 1 se flutter/lib/brand.g.dart non e' aggiornato",
    )
    args = parser.parse_args()

    try:
        atteso = genera(leggi())
    except ErroreBrand as e:
        print(f"genera_tema: {e}", file=sys.stderr)
        return 2

    relativo = DESTINAZIONE.relative_to(RADICE)
    if args.check:
        try:
            attuale = DESTINAZIONE.read_bytes().decode("utf-8")
        except (OSError, UnicodeDecodeError):
            attuale = None
        if attuale == atteso:
            print(f"genera_tema: {relativo} e' aggiornato")
            return 0
        if attuale is None:
            print(f"genera_tema: {relativo} manca", file=sys.stderr)
        else:
            sys.stderr.writelines(
                difflib.unified_diff(
                    attuale.splitlines(keepends=True),
                    atteso.splitlines(keepends=True),
                    f"{relativo} (committato)",
                    f"{relativo} (da brand.toml)",
                )
            )
        print(
            "genera_tema: NON aggiornato; eseguire "
            "python3 brand/genera_tema.py e committare il risultato",
            file=sys.stderr,
        )
        return 1

    DESTINAZIONE.write_bytes(atteso.encode("utf-8"))
    print(f"genera_tema: scritto {relativo}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
