#!/usr/bin/env python3
"""Genera icone e loghi del client da brand/sorgenti/ (Pillow).

Uso:
    python3 brand/genera_icone.py          scrive i file elencati in uscite()
    python3 brand/genera_icone.py --check  non scrive; exit 1 se un file del
                                           repo ha pixel diversi da quelli che
                                           lo script produrrebbe

Serve Pillow 12.3.0 (python3 -m pip install pillow==12.3.0), che non e' una
dipendenza del build: lo script si lancia a mano quando cambiano le sorgenti e
i file generati si committano. --check confronta i pixel decodificati, non i
byte dei PNG (la compressione cambia con zlib); il ricampionamento puo'
cambiare con la versione di Pillow, per questo e' fissata: i file del repo
sono prodotti con la 12.3.0. Codici di uscita: 0 ok, 1 file diversi (--check),
2 sorgente mancante o non valida. Il .gitignore upstream esclude i PNG
("*png"): un PNG nuovo si aggiunge con git add -f, quelli gia' tracciati no.

Sorgenti: con il logo originale del grafico si sostituiscono questi due
file, con gli stessi nomi, e si rilancia lo script. Un logo in positivo per
il tema chiaro (oggi solo il simbolo) vuole una terza sorgente e la sua riga
in uscite().
    brand/sorgenti/marchio.png        solo il simbolo, fondo trasparente
    brand/sorgenti/logo-negativo.png  simbolo, scritta chiara e motto su
                                      fondo trasparente: va su fondo scuro
Quelli attuali vengono dal sito infinitek.it (non esistono vettoriali ne'
raster oltre i 600 px): bastano per Windows fino a 256 px, non per macOS,
iOS e Android (1024 px), che restano quelli upstream.

Chi legge i file generati (Windows):
- res/icon.ico: icona dell'exe autoestraente distribuito
  (libs/portable/build.rs) e del pacchetto MSI. build.rs la mette nella DLL
  solo con la feature inline (Sciter): nel build Flutter la DLL non ha icona.
- flutter/windows/runner/resources/app_icon.ico: IDI_APP_ICON di Runner.rc,
  cioe' l'icona di Remotek.exe installato, della finestra, della barra delle
  applicazioni e dei collegamenti.
- res/tray-icon.ico: area di notifica quando manca assets/icon.png
  (src/tray.rs, make_tray). Una sola misura, 32 px come upstream:
  image::load_from_memory decodifica solo la voce piu' grande dell'ICO.
- flutter/assets/icon.png (32 px) e flutter/assets/2.0x/icon.png (64 px):
  loadIcon() di flutter/lib/common.dart (connection manager a 30 px, barra
  delle schede a 16 px; Flutter sceglie la variante 2.0x sugli schermi
  ad alta densita'). icon.png e' anche l'icona dell'area di notifica:
  src/tray.rs la preferisce a tray-icon.ico; 32 px come quella upstream.
- flutter/assets/logo_light.png, logo_dark.png: loadLogo() della home
  (riquadro massimo 300 x 60). Tema chiaro: solo il marchio, perche' la
  scritta del logo in negativo e' bianca. Tema scuro: logo in negativo senza
  motto, che a 60 px di altezza sarebbe alto 3 px e illeggibile.
Il file flutter/assets/icon.svg resta quello upstream: loadIcon() lo usa
solo se manca icon.png, e non c'e' un vettoriale del marchio.
"""

import argparse
import io
import struct
import sys
from pathlib import Path

try:
    from PIL import Image
except ModuleNotFoundError:
    sys.exit("genera_icone: serve Pillow (python3 -m pip install pillow==12.3.0)")

RADICE = Path(__file__).resolve().parent.parent
MARCHIO = RADICE / "brand" / "sorgenti" / "marchio.png"
LOGO_NEGATIVO = RADICE / "brand" / "sorgenti" / "logo-negativo.png"

# Misure delle icone Windows: quelle che la shell chiede da 100% a 200% di
# scala (16-32 piccole, 32-64 grandi, 256 in Esplora risorse).
MISURE_ICO = (16, 20, 24, 32, 40, 48, 64, 96, 128, 256)
FIRMA_PNG = b"\x89PNG\r\n\x1a\n"


class ErroreBrand(Exception):
    """Sorgente mancante o non valida."""


def carica(percorso: Path) -> Image.Image:
    """Apre la sorgente in RGBA e la ritaglia al contenuto non trasparente."""
    try:
        with Image.open(percorso) as im:
            rgba = im.convert("RGBA")
    except OSError as e:
        raise ErroreBrand(f"{percorso.relative_to(RADICE)}: {e}") from e
    riquadro = rgba.getchannel("A").getbbox()
    if riquadro is None:
        raise ErroreBrand(f"{percorso.relative_to(RADICE)}: e' tutta trasparente")
    return rgba.crop(riquadro)


def ridimensiona(im: Image.Image, larghezza: int, altezza: int) -> Image.Image:
    # Alfa premoltiplicato: senza, i bordi prendono il colore dei pixel
    # trasparenti e l'icona ha un alone scuro.
    return (
        im.convert("RGBa")
        .resize((larghezza, altezza), Image.Resampling.LANCZOS)
        .convert("RGBA")
    )


def quadrata(simbolo: Image.Image, lato: int) -> Image.Image:
    """Simbolo centrato su un quadrato trasparente, margine di 1/32 per lato."""
    interno = lato - 2 * (lato // 32)
    scala = min(interno / simbolo.width, interno / simbolo.height)
    larghezza = max(1, round(simbolo.width * scala))
    altezza = max(1, round(simbolo.height * scala))
    tela = Image.new("RGBA", (lato, lato), (0, 0, 0, 0))
    tela.paste(
        ridimensiona(simbolo, larghezza, altezza),
        ((lato - larghezza) // 2, (lato - altezza) // 2),
    )
    return tela


def senza_motto(logo: Image.Image) -> Image.Image:
    """Tiene le prime due fasce orizzontali (simbolo e scritta).

    Le fasce sono gruppi di righe con pixel non trasparenti, separati da righe
    del tutto trasparenti. Con due fasce o meno il logo resta com'e'.
    """
    alfa = logo.getchannel("A")
    piene = [
        alfa.crop((0, y, logo.width, y + 1)).getbbox() is not None
        for y in range(logo.height)
    ]
    fasce = []
    for y, piena in enumerate(piene):
        if piena and (y == 0 or not piene[y - 1]):
            fasce.append([y, y + 1])
        elif piena:
            fasce[-1][1] = y + 1
    if len(fasce) <= 2:
        return logo
    tagliato = logo.crop((0, 0, logo.width, fasce[1][1]))
    return tagliato.crop(tagliato.getchannel("A").getbbox())


def voce_bmp(im: Image.Image) -> bytes:
    """Voce ICO in DIB a 32 bit (BGRA dal basso) con maschera AND."""
    larghezza, altezza = im.size
    rosso, verde, blu, alfa = im.split()
    bgra = Image.merge("RGBA", (blu, verde, rosso, alfa)).tobytes()
    riga = larghezza * 4
    pixel = b"".join(
        bgra[y * riga : (y + 1) * riga] for y in reversed(range(altezza))
    )
    passo = (larghezza + 31) // 32 * 4
    maschera = bytearray()
    for y in reversed(range(altezza)):
        bit = bytearray(passo)
        for x in range(larghezza):
            if alfa.getpixel((x, y)) == 0:
                bit[x // 8] |= 0x80 >> (x % 8)
        maschera += bit
    intestazione = struct.pack(
        "<IiiHHIIiiII",
        40, larghezza, 2 * altezza, 1, 32, 0,
        len(pixel) + len(maschera), 0, 0, 0, 0,
    )
    return intestazione + pixel + bytes(maschera)


def png(im: Image.Image) -> bytes:
    uscita = io.BytesIO()
    im.save(uscita, "PNG", optimize=True)
    return uscita.getvalue()


def ico(immagini: list[Image.Image]) -> bytes:
    """ICO con voci BMP sotto i 256 px e PNG a 256 (formato di Windows Vista+)."""
    voci = [png(im) if im.width >= 256 else voce_bmp(im) for im in immagini]
    testa = struct.pack("<HHH", 0, 1, len(voci))
    posizione = 6 + 16 * len(voci)
    for im, voce in zip(immagini, voci):
        testa += struct.pack(
            "<BBBBHHII",
            im.width % 256, im.height % 256, 0, 0, 1, 32, len(voce), posizione,
        )
        posizione += len(voce)
    return testa + b"".join(voci)


def uscite() -> dict[str, tuple[str, list[Image.Image]]]:
    """Percorso relativo -> (formato, immagini), nell'ordine di scrittura."""
    marchio = carica(MARCHIO)
    logo = senza_motto(carica(LOGO_NEGATIVO))
    icone = [quadrata(marchio, lato) for lato in MISURE_ICO]
    return {
        "res/icon.ico": ("ico", icone),
        "flutter/windows/runner/resources/app_icon.ico": ("ico", icone),
        "res/tray-icon.ico": ("ico", [quadrata(marchio, 32)]),
        "flutter/assets/icon.png": ("png", [quadrata(marchio, 32)]),
        "flutter/assets/2.0x/icon.png": ("png", [quadrata(marchio, 64)]),
        "flutter/assets/logo_light.png": ("png", [marchio]),
        "flutter/assets/logo_dark.png": ("png", [logo]),
    }


def decodifica(dati: bytes) -> list[tuple[tuple[int, int], bytes]]:
    """(misura, pixel RGBA) di un PNG o di ogni voce di un ICO."""
    if dati.startswith(FIRMA_PNG):
        with Image.open(io.BytesIO(dati)) as im:
            return [(im.size, im.convert("RGBA").tobytes())]
    _, tipo, numero = struct.unpack_from("<HHH", dati)
    if tipo != 1:
        raise ValueError("non e' un ICO")
    risultato = []
    for i in range(numero):
        _, _, _, _, _, _, lunghezza, inizio = struct.unpack_from(
            "<BBBBHHII", dati, 6 + 16 * i
        )
        voce = dati[inizio : inizio + lunghezza]
        if voce.startswith(FIRMA_PNG):
            risultato += decodifica(voce)
        else:
            # Le voci BMP sono scritte senza compressione: basta il confronto
            # dei byte, fatto sulla voce intera.
            risultato.append(((i, lunghezza), voce))
    return risultato


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "--check",
        action="store_true",
        help="non scrive; exit 1 se i file del repo non sono aggiornati",
    )
    args = parser.parse_args()

    try:
        attese = uscite()
    except ErroreBrand as e:
        print(f"genera_icone: {e}", file=sys.stderr)
        return 2

    diversi = 0
    for relativo, (formato, immagini) in attese.items():
        dati = ico(immagini) if formato == "ico" else png(immagini[0])
        destinazione = RADICE / relativo
        misure = ", ".join(f"{im.width}x{im.height}" for im in immagini)
        if args.check:
            try:
                uguale = decodifica(destinazione.read_bytes()) == decodifica(dati)
            except (OSError, ValueError, struct.error):
                uguale = False
            if uguale:
                print(f"ok       {relativo} ({misure})")
            else:
                print(f"DIVERSO  {relativo}", file=sys.stderr)
                diversi += 1
            continue
        destinazione.parent.mkdir(parents=True, exist_ok=True)
        destinazione.write_bytes(dati)
        print(f"scritto  {relativo} ({misure}, {len(dati)} byte)")

    if diversi:
        print(
            f"genera_icone: {diversi} file non aggiornati; eseguire "
            "python3 brand/genera_icone.py e committare il risultato",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
