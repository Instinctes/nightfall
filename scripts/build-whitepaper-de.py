#!/usr/bin/env python3
"""Build the NIGHTFALLCOIN whitepaper (DE + EN).

Dark, branded, honest. Writes NIGHTFALLCOIN-Whitepaper-{DE,EN}.pdf
at the workspace root. The cover is a raw canvas page so the coin
mark actually lands; Platypus onPage silently dropped drawImage.
"""

from __future__ import annotations

import os
import tempfile

from PIL import Image as PILImage
from reportlab.lib.colors import Color, HexColor
from reportlab.lib.enums import TA_CENTER, TA_JUSTIFY, TA_LEFT, TA_RIGHT
from reportlab.lib.pagesizes import A4
from reportlab.lib.styles import ParagraphStyle
from reportlab.lib.units import mm
from reportlab.lib.utils import ImageReader
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont
from reportlab.pdfgen import canvas as pdfcanvas
from reportlab.platypus import (
    BaseDocTemplate,
    CondPageBreak,
    Flowable,
    Frame,
    KeepTogether,
    NextPageTemplate,
    PageBreak,
    PageTemplate,
    Paragraph,
    Spacer,
    Table,
    TableStyle,
)

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), os.pardir))
FONT_DIR = os.path.join(os.path.dirname(__file__), "whitepaper-fonts")
LOGO_SRC = os.path.join(ROOT, "assets", "logo", "coin-1024.png")
LOGO = os.path.join(tempfile.gettempdir(), "nightfall-logo-flat.png")
LANG = "de"


def _flatten_logo() -> None:
    """Composite the transparent coin onto the page colour so reportlab
    does not have to honour a PNG mask inside onPage."""
    src = PILImage.open(LOGO_SRC).convert("RGBA")
    bg = PILImage.new("RGBA", src.size, (14, 11, 24, 255))  # #0e0b18
    PILImage.alpha_composite(bg, src).convert("RGB").save(LOGO, "PNG")

PAGE_W, PAGE_H = A4

# Palette — website / Core Wallet, exact.
BG = HexColor("#0e0b18")
BG2 = HexColor("#141022")
SURFACE = HexColor("#1a1430")
SURFACE2 = HexColor("#221a3c")
BORDER = HexColor("#2e2648")
BORDER_HI = HexColor("#45396a")
TEXT = HexColor("#f2eefc")
DIM = HexColor("#a89fc6")
FAINT = HexColor("#6f668f")
VIOLET = HexColor("#7c5cff")
VIOLET_HI = HexColor("#9b83ff")
MAGENTA = HexColor("#b845d8")
TEAL = HexColor("#35a3c4")
SUCCESS = HexColor("#4ae0a8")
WARN = HexColor("#ffc85c")
CYAN = HexColor("#35a3c4")

ML, MR, MT, MB = 22 * mm, 22 * mm, 24 * mm, 18 * mm
CONTENT_W = PAGE_W - ML - MR

GENESIS = "061a052d49607ff8f4b306c75d622ebd230cff4ec3a45a6dffc2f7738d4b20de"


def _register_fonts() -> None:
    faces = {
        "Inter": "Inter-Regular.ttf",
        "InterMed": "Inter-Medium.ttf",
        "InterSemi": "Inter-SemiBold.ttf",
        "InterBold": "Inter-Bold.ttf",
        "InterI": "Inter-Italic.ttf",
        "InterSemiI": "Inter-SemiBoldItalic.ttf",
    }
    for name, fn in faces.items():
        pdfmetrics.registerFont(TTFont(name, os.path.join(FONT_DIR, fn)))
    pdfmetrics.registerFont(TTFont("SFMono", "/System/Library/Fonts/SFNSMono.ttf"))
    pdfmetrics.registerFontFamily(
        "Inter",
        normal="Inter",
        bold="InterBold",
        italic="InterI",
        boldItalic="InterSemiI",
    )


def S() -> dict[str, ParagraphStyle]:
    return {
        "body": ParagraphStyle(
            "body",
            fontName="Inter",
            fontSize=10.2,
            leading=15.4,
            textColor=TEXT,
            alignment=TA_JUSTIFY,
            spaceAfter=9,
        ),
        "lead": ParagraphStyle(
            "lead",
            fontName="Inter",
            fontSize=12.2,
            leading=18.4,
            textColor=TEXT,
            alignment=TA_JUSTIFY,
            spaceAfter=12,
        ),
        "kicker": ParagraphStyle(
            "kicker",
            fontName="InterSemi",
            fontSize=8,
            leading=11,
            textColor=VIOLET_HI,
            alignment=TA_LEFT,
            tracking=1.6,
            spaceAfter=4,
        ),
        "h2": ParagraphStyle(
            "h2",
            fontName="InterSemi",
            fontSize=13.2,
            leading=17,
            textColor=TEXT,
            spaceBefore=12,
            spaceAfter=8,
        ),
        "cell": ParagraphStyle(
            "cell",
            fontName="Inter",
            fontSize=8.4,
            leading=11.6,
            textColor=TEXT,
        ),
        "cellh": ParagraphStyle(
            "cellh",
            fontName="InterSemi",
            fontSize=8.0,
            leading=11.2,
            textColor=VIOLET_HI,
        ),
        "celldim": ParagraphStyle(
            "celldim",
            fontName="Inter",
            fontSize=8.4,
            leading=11.6,
            textColor=DIM,
        ),
        "call": ParagraphStyle(
            "call",
            fontName="Inter",
            fontSize=10.0,
            leading=14.8,
            textColor=TEXT,
        ),
        "callb": ParagraphStyle(
            "callb",
            fontName="InterSemi",
            fontSize=10.4,
            leading=15.2,
            textColor=TEXT,
        ),
        "quote": ParagraphStyle(
            "quote",
            fontName="InterI",
            fontSize=13.4,
            leading=19.2,
            textColor=TEXT,
            alignment=TA_LEFT,
        ),
        "caption": ParagraphStyle(
            "caption",
            fontName="Inter",
            fontSize=8.2,
            leading=11.4,
            textColor=FAINT,
            alignment=TA_LEFT,
            spaceBefore=2,
            spaceAfter=12,
        ),
        "mono": ParagraphStyle(
            "mono",
            fontName="SFMono",
            fontSize=8.6,
            leading=13.2,
            textColor=VIOLET_HI,
            alignment=TA_CENTER,
        ),
        "tocn": ParagraphStyle(
            "tocn",
            fontName="InterSemi",
            fontSize=9,
            leading=14,
            textColor=VIOLET_HI,
        ),
        "toct": ParagraphStyle(
            "toct",
            fontName="Inter",
            fontSize=9.4,
            leading=14,
            textColor=TEXT,
        ),
        "bullet": ParagraphStyle(
            "bullet",
            fontName="Inter",
            fontSize=10.2,
            leading=15.2,
            textColor=TEXT,
            leftIndent=14,
            bulletIndent=0,
            spaceAfter=4,
        ),
        "small": ParagraphStyle(
            "small",
            fontName="Inter",
            fontSize=8.6,
            leading=12.4,
            textColor=DIM,
            alignment=TA_LEFT,
        ),
    }


STY: dict[str, ParagraphStyle] = {}


def P(text: str, style: str = "body") -> Paragraph:
    return Paragraph(text, STY[style])


def tracked(c, text, x, y, font, size, color, tracking=1.4, align="left"):
    c.setFont(font, size)
    c.setFillColor(color)
    widths = [c.stringWidth(ch, font, size) for ch in text]
    total = sum(widths) + tracking * (len(text) - 1 if text else 0)
    if align == "center":
        cx = x - total / 2
    elif align == "right":
        cx = x - total
    else:
        cx = x
    for ch, w in zip(text, widths):
        c.drawString(cx, y, ch)
        cx += w + tracking


def paint_bg(c):
    c.setFillColor(BG)
    c.rect(0, 0, PAGE_W, PAGE_H, fill=1, stroke=0)
    # faint washes — never visible as shapes, only as depth
    c.saveState()
    c.setFillColor(Color(0.486, 0.361, 1.0, alpha=0.055))
    c.circle(0, PAGE_H, 280, fill=1, stroke=0)
    c.setFillColor(Color(0.722, 0.271, 0.847, alpha=0.04))
    c.circle(PAGE_W, PAGE_H - 40, 240, fill=1, stroke=0)
    c.setFillColor(Color(0.208, 0.639, 0.769, alpha=0.03))
    c.circle(PAGE_W * 0.55, 0, 220, fill=1, stroke=0)
    c.restoreState()


def glow(c, cx, cy, r):
    c.saveState()
    for i in range(16, 0, -1):
        a = 0.018 * (i / 16) ** 1.6
        c.setFillColor(Color(0.486, 0.361, 1.0, alpha=a))
        c.circle(cx, cy, r * (1 + i * 0.085), fill=1, stroke=0)
    c.restoreState()


def draw_cover(c, lang: str):
    paint_bg(c)
    cx = PAGE_W / 2
    logo_s = 128
    logo_y = PAGE_H - 70 - logo_s
    glow(c, cx, logo_y + logo_s / 2, logo_s * 0.48)
    c.setFillColor(Color(1, 1, 1, alpha=1))
    c.drawImage(
        LOGO_SRC,
        cx - logo_s / 2,
        logo_y,
        width=logo_s,
        height=logo_s,
        mask="auto",
    )

    tracked(
        c,
        "NIGHTFALLCOIN",
        cx,
        logo_y - 36,
        "InterBold",
        26,
        TEXT,
        tracking=3.2,
        align="center",
    )
    c.setFillColor(DIM)
    c.setFont("InterI", 12)
    c.drawCentredString(cx, logo_y - 58, "Money that refuses to snitch.")

    # accent rule
    y = logo_y - 84
    c.setStrokeColor(VIOLET)
    c.setLineWidth(1.2)
    c.line(cx - 28, y, cx + 28, y)
    c.setStrokeColor(TEAL)
    c.setLineWidth(0.7)
    c.line(cx + 30, y, cx + 78, y)
    c.setStrokeColor(MAGENTA)
    c.line(cx - 78, y, cx - 30, y)

    tracked(
        c,
        "WHITEPAPER",
        cx,
        y - 28,
        "InterSemi",
        8.5,
        VIOLET_HI,
        tracking=3.8,
        align="center",
    )
    subtitle = (
        ("Protokoll und Horizont", "Was diese Kette ist. Was sie nicht ist. Wohin sie gehen darf.")
        if lang == "de"
        else ("Protocol and horizon", "What this chain is. What it is not. Where it may go.")
    )
    c.setFillColor(TEXT)
    c.setFont("Inter", 13)
    c.drawCentredString(cx, y - 50, subtitle[0])
    c.setFillColor(DIM)
    c.setFont("Inter", 9.5)
    c.drawCentredString(cx, y - 68, subtitle[1])

    # pills
    pills = [
        ("LIVE", "Mainnet v8"),
        ("FAIR", "0 Premine"),
        ("POW", "Argon2id 32 MiB"),
    ]
    pw, ph, gap = 118, 28, 10
    total = 3 * pw + 2 * gap
    x0 = cx - total / 2
    py = 168
    for i, (k, v) in enumerate(pills):
        x = x0 + i * (pw + gap)
        c.setFillColor(SURFACE)
        c.setStrokeColor(BORDER)
        c.setLineWidth(0.7)
        c.roundRect(x, py, pw, ph, 8, fill=1, stroke=1)
        c.setFillColor(VIOLET_HI)
        c.setFont("InterSemi", 6.5)
        c.drawCentredString(x + pw / 2, py + 16.5, k)
        c.setFillColor(TEXT)
        c.setFont("Inter", 8)
        c.drawCentredString(x + pw / 2, py + 6, v)

    c.setFillColor(FAINT)
    c.setFont("InterSemi", 7)
    c.drawCentredString(cx, 132, "GENESIS")
    c.setFillColor(VIOLET_HI)
    c.setFont("SFMono", 7.2)
    g1, g2 = GENESIS[:32], GENESIS[32:]
    c.drawCentredString(cx, 116, g1)
    c.drawCentredString(cx, 104, g2)

    c.setFillColor(FAINT)
    c.setFont("Inter", 8)
    c.drawCentredString(
        cx, 72, "Software 0.9.2  ·  September 2026  ·  Protocol v8 · Wire v6 · NFL2"
    )
    c.setFont("Inter", 8)
    c.drawCentredString(cx, 40, "nightfallcoin.org")


def draw_body(c, doc):
    paint_bg(c)
    # header
    c.setStrokeColor(BORDER)
    c.setLineWidth(0.5)
    c.line(ML, PAGE_H - 16 * mm, PAGE_W - MR, PAGE_H - 16 * mm)
    c.setStrokeColor(VIOLET)
    c.setLineWidth(1.1)
    c.line(ML, PAGE_H - 16 * mm, ML + 28, PAGE_H - 16 * mm)

    mark = 13
    c.saveState()
    c.setFillColor(Color(1, 1, 1, alpha=1))
    c.drawImage(LOGO_SRC, ML, PAGE_H - 16.6 * mm, width=mark, height=mark, mask="auto")
    c.restoreState()
    tracked(
        c,
        "NIGHTFALLCOIN",
        ML + 15,
        PAGE_H - 14.4 * mm,
        "InterSemi",
        7.2,
        DIM,
        tracking=1.6,
        align="left",
    )
    c.setFillColor(FAINT)
    c.setFont("Inter", 7.2)
    c.drawRightString(PAGE_W - MR, PAGE_H - 14.4 * mm, "Whitepaper")

    # footer
    c.setStrokeColor(BORDER)
    c.setLineWidth(0.5)
    c.line(ML, 12 * mm, PAGE_W - MR, 12 * mm)
    c.setFillColor(FAINT)
    c.setFont("Inter", 7.2)
    c.drawString(ML, 7.2 * mm, "nightfallcoin.org")
    c.drawRightString(PAGE_W - MR, 7.2 * mm, f"{doc.page}")
    c.setFillColor(VIOLET)
    c.setFont("InterSemi", 7.2)
    c.drawCentredString(PAGE_W / 2, 7.2 * mm, "NIGHT")


class Chapter(Flowable):
    def __init__(self, num: str, title: str):
        super().__init__()
        self.num = num
        self.title = title

    def wrap(self, aw, _ah):
        self.aw = aw
        return aw, 58

    def draw(self):
        c = self.canv
        kicker = "KAPITEL" if LANG == "de" else "CHAPTER"
        tracked(
            c,
            f"{kicker} {self.num}",
            0,
            42,
            "InterSemi",
            7.5,
            VIOLET_HI,
            tracking=2.2,
        )
        c.setFillColor(TEXT)
        c.setFont("InterBold", 18)
        c.drawString(0, 18, self.title)
        c.setStrokeColor(BORDER)
        c.setLineWidth(0.6)
        c.line(0, 4, self.aw, 4)
        c.setStrokeColor(VIOLET)
        c.setLineWidth(1.5)
        c.line(0, 4, 34, 4)


class Callout(Flowable):
    def __init__(self, text: str, kind: str = "note"):
        super().__init__()
        self.text = text
        self.kind = kind
        bar = {"note": VIOLET, "warn": WARN, "ok": SUCCESS, "quote": MAGENTA}[kind]
        self.bar = bar

    def wrap(self, aw, ah):
        self.aw = aw
        style = STY["callb"] if self.kind in ("quote", "ok") else STY["call"]
        self._p = Paragraph(self.text, style)
        _w, h = self._p.wrap(aw - 22, ah)
        self.h = h + 18
        return aw, self.h

    def draw(self):
        c = self.canv
        c.setFillColor(SURFACE)
        c.roundRect(0, 0, self.aw, self.h, 7, fill=1, stroke=0)
        c.setFillColor(self.bar)
        c.rect(0, 0, 3.2, self.h, fill=1, stroke=0)
        self._p.drawOn(c, 14, 9)


class Equation(Flowable):
    def wrap(self, aw, _ah):
        self.aw = aw
        return aw, 44

    def draw(self):
        c = self.canv
        c.setFillColor(SURFACE)
        c.roundRect(0, 0, self.aw, 40, 7, fill=1, stroke=0)
        c.setFillColor(VIOLET_HI)
        c.setFont("SFMono", 9.2)
        c.drawCentredString(
            self.aw / 2,
            16,
            "Σ UTXO  −  Σ kernel_excess  =  (minted − burned) · G",
        )


class Rule(Flowable):
    def wrap(self, aw, _ah):
        self.aw = aw
        return aw, 10

    def draw(self):
        c = self.canv
        c.setStrokeColor(BORDER)
        c.setLineWidth(0.5)
        c.line(0, 5, self.aw, 5)


def grid(rows, cols, has_header=True):
    """rows: list of list of str. First row is header if has_header."""
    styles_row0 = STY["cellh"]
    body = STY["cell"]
    dim = STY["celldim"]
    data = []
    for r_i, row in enumerate(rows):
        out = []
        for c_i, cell in enumerate(row):
            st = styles_row0 if (has_header and r_i == 0) else (dim if c_i == 0 else body)
            out.append(Paragraph(cell, st))
        data.append(out)
    t = Table(data, colWidths=cols, repeatRows=1 if has_header else 0)
    cmds = [
        ("VALIGN", (0, 0), (-1, -1), "TOP"),
        ("LEFTPADDING", (0, 0), (-1, -1), 8),
        ("RIGHTPADDING", (0, 0), (-1, -1), 8),
        ("TOPPADDING", (0, 0), (-1, -1), 6.5),
        ("BOTTOMPADDING", (0, 0), (-1, -1), 6.5),
        ("BACKGROUND", (0, 0), (-1, -1), SURFACE),
        ("BOX", (0, 0), (-1, -1), 0.5, BORDER),
        ("LINEBELOW", (0, 0), (-1, -2), 0.4, BORDER),
    ]
    if has_header:
        cmds += [
            ("BACKGROUND", (0, 0), (-1, 0), SURFACE2),
            ("LINEBELOW", (0, 0), (-1, 0), 0.8, VIOLET),
        ]
    t.setStyle(TableStyle(cmds))
    return t


def bullets(items: list[str]) -> list:
    out = []
    for it in items:
        out.append(Paragraph(f'<font color="#7c5cff"><b>•</b></font>  {it}', STY["bullet"]))
    return out


def story_de() -> list:
    W = CONTENT_W
    c3 = [W * 0.28, W * 0.36, W * 0.36]
    c2 = [W * 0.30, W * 0.70]
    toc = [
        ["01", "Die These"],
        ["02", "Die Kette, wie sie läuft"],
        ["03", "Kryptographie, ohne Theater"],
        ["04", "Privacy, ehrlich"],
        ["05", "Geld und Mining"],
        ["06", "Was man heute in der Hand hält"],
        ["07", "Atomic Swap — und der Wermutstropfen"],
        ["08", "Was fehlt, und warum das so gesagt wird"],
        ["09", "Unverhandelbar"],
        ["10", "Horizont"],
        ["11", "Was wir nicht bauen"],
        ["12", "Schluss"],
    ]
    toc_tbl = Table(
        [[Paragraph(n, STY["tocn"]), Paragraph(t, STY["toct"])] for n, t in toc],
        colWidths=[28, W - 28],
    )
    toc_tbl.setStyle(
        TableStyle(
            [
                ("VALIGN", (0, 0), (-1, -1), "MIDDLE"),
                ("TOPPADDING", (0, 0), (-1, -1), 3.5),
                ("BOTTOMPADDING", (0, 0), (-1, -1), 3.5),
                ("LEFTPADDING", (0, 0), (-1, -1), 0),
            ]
        )
    )

    facts = grid(
        [
            ["", "Wert", "Bedeutung"],
            ["Asset", "NIGHTFALLCOIN (NIGHT)", "Native L1, kein Token auf fremder Kette"],
            ["Genesis", "061a052d…20de", "16. August 2026. Münzen älterer Genesise existieren hier nicht."],
            ["Protokoll / Wire", "v8 / v6, Magic NFL2", "WIRE sperrt alte Nodes. PROTOCOL im Genesis — anfassen = neue Kette."],
            ["Datadir", "nightfall/&lt;net&gt;/n8/", "v7-Daten bleiben liegen. Nichts löschen."],
            ["Cap", "90 000 000 NIGHT", "Kein Tail. Terminal 89 999 999,25 — 0,75 unter der Decke."],
            ["Block", "6 NIGHT, Halving 7 500 000, Ziel 15 s", "Etwa 3,56 Jahre bis zur ersten Halbierung."],
            ["PoW", "Nighthash-v2, Argon2id 32 MiB", "CPU. Verifikation kostet denselben Hash. Kein GPU-/ASIC-Vorteil."],
            ["Difficulty", "LWMA-1, Fenster 90, Floor 2000", "Kumulierte Arbeit entscheidet, nie die Blockzahl."],
            ["Fees", "Brennen, solange Subsidy &gt; 0", "Danach Coinbase = Fees. Öffentlich, damit der Burn prüfbar bleibt."],
            ["Premine / Admin", "0 / keiner", "Kein Freeze, kein Mint, keine Treasury-Münzen."],
            ["Reorg-Tiefe", "500 Blöcke", "Tiefer wird abgelehnt. Checkpoint bei Höhe 25 000."],
            ["Software", "0.9.2 (September 2026)", "Konsens unverändert seit v8. Dieselbe Kette."],
        ],
        [W * 0.22, W * 0.38, W * 0.40],
    )

    suite = grid(
        [
            ["Stück", "Wahl", "Warum"],
            ["Kurve", "Ristretto (curve25519), überall", "Eine Kurve, eine Kodierung. v4 mischte ed25519 und x25519."],
            ["Hash", "Blake3, domain-separated", "Längepräfix. Kein stilles Zusammensetzen."],
            ["Commitments", "Pedersen C = v·G + b·H", "H ist NUMS. Soundness hängt daran, dass niemand x mit H = x·G kennt."],
            ["Range proofs", "Bulletproofs, 64-bit, je Output", "v ∈ [0, 2⁶⁴). Netzgebunden, nicht replaybar."],
            ["Signaturen", "Schnorr auf Ristretto", "Kernel über Generator H, Inputs über Ko, Outputs über Ke."],
            ["Payload", "XChaCha20-Poly1305", "Feste Länge. Ciphertext verrät das Memo nicht."],
            ["Adresse", "nf1 + Scan-PK + Spend-PK", "Erscheint nie on-chain. View-Key: nfview1, sieht, kann nicht signieren."],
        ],
        c3,
    )

    priv = grid(
        [
            ["Eigenschaft", "Stand"],
            ["Beträge", "Verborgen (Pedersen + Bulletproofs)"],
            ["Empfängeradresse", "Erscheint nie. nf1 wird zur One-Time-Key Ko."],
            ["Absenderadresse", "Erscheint nie."],
            ["Transparenter Pool", "Keiner. Privacy ist Pflicht, kein Schalter."],
            ["View-Key", "nfview1 — findet und öffnet jedes Output, kann nicht signieren."],
            ["Receipt", "Beweist genau eine Zahlung, ohne den View-Key."],
            ["Graph", "Durch Blockaggregation vermischt, <b>nicht gelöscht</b>."],
            ["Cut-through", "Nicht angewendet. Würde die Input-Signatur löschen, die einseitige Zahlungen sicher macht."],
            ["Dandelion", "Stem/Fluff auf der bestehenden Tx-Nachricht. 90 % Stem."],
            ["Tor / SOCKS5", "<b>Default</b> 127.0.0.1:9050. Clearnet-Fallback, wenn Tor tot ist."],
            ["Anonymitätsmenge heute", "Die anderen Transaktionen <i>im selben Block</i>. Das Netz ist klein. Nicht protocol-scale."],
            ["Externes Audit", "Keines. Internes Review: 16. August 2026."],
        ],
        c2,
    )

    hist = grid(
        [
            ["Epoche", "Genesis", "Warum sie endete"],
            ["v4 Nightproof-α", "verworfen", "Balance-Prüfung war eine Tautologie. Jeder konnte minten."],
            ["v5 / v6", "verworfen", "Krypto klang, Netz und Reorg-Wallet nicht."],
            ["v7", "c8614333…", "Emission zu frontlastig. Zwei Miner hielten den Float. Begraben."],
            ["v8 / n8 — diese Kette", "061a052d…", "Live. 6 NIGHT / 7,5 Mio. Blöcke. Magic NFL2."],
        ],
        [W * 0.24, W * 0.22, W * 0.54],
    )

    gaps = grid(
        [
            ["Lücke", "Was das konkret heißt"],
            [
                "Cut-through",
                "Spent Outputs bleiben sichtbar. Einseitige Zahlungen und Cut-through schließen einander aus, solange jeder Input eine öffentliche Signatur trägt.",
            ],
            [
                "Dandelion++",
                "Heute stemmt dieselbe Gossip-Menge wie Blöcke. Kein separates Stem-Graph, kein Fail-safe-Fluff bei Black-Hole-Stems.",
            ],
            [
                "Unabhängiges Audit",
                "v5/v8 hat dieselbe Partei geschrieben, die v4 zerlegt hat. Das interne Review zählt als Review, nicht als Fremdauge.",
            ],
            [
                "IBD / Verify",
                "Jeder Hash ist ~11 ms. Ein Jahr Kette ist Stunden CPU. Prune und Archive gibt es; Headers-first und MMR nicht.",
            ],
            [
                "UTXO-Root",
                "O(n log n) pro Block. Ein Merkle Mountain Range würde ihn inkrementell machen.",
            ],
            [
                "Hashrate",
                "Klein. Reorgs sind billiger als auf einer reifen Kette. Deshalb Mining-Hold auf Forks, Reorg-Cap 500, Checkpoint 25 000.",
            ],
            [
                "Seeds",
                "Zwei Maschinen, ein Operator. Discovery ist ein Single-Point. Konsens ist es nicht — jeder Node validiert allein.",
            ],
        ],
        c2,
    )

    future = grid(
        [
            ["Spur", "Was", "Was nicht"],
            [
                "Netz",
                "Echtes Dandelion++: eigener Stem-Pfad, Fail-safe-Fluff, Tor bleibt Default.",
                "Kein I2P-Versprechen. Kein zweites Overlay, bevor das erste hält.",
            ],
            [
                "Sync",
                "MMR oder verwandter Akkumulator, Headers-first, gebundene Snapshots. Prune und Wochenarchive gibt es schon.",
                "Kein UTXO-Snapshot von einem Fremden als Vertrauensanker.",
            ],
            [
                "Graph",
                "Research: Spend-Autorisierung ohne per-Input-Schnorr, die einseitige Zahlungen trotzdem hält. Erst dann Cut-through.",
                "Kein Wallet-koordiniertes CoinJoin als Ersatz. Der Block mischt schon, ohne Termin.",
            ],
            [
                "Disclosure",
                "Receipts und View-Keys existieren. Darauf: Zeitfenster, Untergrenze, maschinenlesbares Schema mit Ablauf.",
                "Kein Compliance-Mode, der Privacy zum Opt-in macht.",
            ],
            [
                "Swap",
                "Externes Review des Ristretto-DLEQ-Blatts. Dann erst die Mainnet-Sperre öffnen.",
                "Keine NIGHT-Rückerstattung erfinden. v0.1 ist zurückgezogen, weil Vorsignieren hier nicht existiert.",
            ],
            [
                "Engineering",
                "Ein Harness, der zwei bis drei Nodes startet, sie divergieren lässt und Konvergenz behauptet. Die Krypto-Suite fängt ihre Fehler. Das Netz bisher nicht.",
                "Keine Feature-Liste, die den Konsens aufbläht.",
            ],
        ],
        c3,
    )

    pos = grid(
        [
            ["Ansatz", "Was er kann", "Nightfalls Antwort"],
            [
                "Bitcoin",
                "Neutrales Settlement, kumulierte Arbeit, keine Admin-Key.",
                "Dieselbe Haltung, native Privacy, beweisbarer Supply.",
            ],
            [
                "Monero",
                "Reife Privacy-Währung, protocol-scale Anonymitätsmenge.",
                "Kein Ring-CT-Klon. Supply ist hier eine Gleichung, keine Decoy-Annahme. Graph nicht gelöscht — das wird gesagt.",
            ],
            [
                "Grin / Mimblewimble",
                "Aggregation, Cut-through, elegante Confidential Transactions.",
                "Aggregation ja. Cut-through nein, weil einseitige Zahlungen die Input-Signatur brauchen. Bewusste Wahl, kein vergessenes Feature.",
            ],
            [
                "Zcash-artig",
                "Starke ZK-Privacy, selektive Disclosure.",
                "Kein Trusted Setup. Proof-Research später, nicht als Foundation.",
            ],
            [
                "Smart-Contract-L1",
                "Programmierbarkeit, DeFi, MEV.",
                "Keine VM auf L1. Narrow waist: UTXO, vertraulicher Wert, Beweis, Signatur. Alles andere off-chain.",
            ],
        ],
        c3,
    )

    s: list = [
        # --- lesart ---
        P("WIE DIESES PAPIER ZU LESEN IST", "kicker"),
        P("Die Spec ist Gesetz. Dieses Papier ist das Argument.", "lead"),
        P(
            "NIGHTFALLCOIN läuft. Genesis <font face='SFMono' color='#9b83ff'>061a052d…</font>, "
            "Protokoll v8, Wire v6, Magic NFL2, Software 0.9.2. Die Zahlen in Kapitel 2 "
            "stehen in <font face='SFMono' color='#9b83ff'>docs/SPEC.md</font> und im Code. "
            "Wer eine Konsensregel nachschlagen will, schlägt dort nach — nicht hier."
        ),
        P(
            "Drei Ebenen, absichtlich getrennt, damit eine Vision nicht als ausgelieferte "
            "Funktion gelesen wird:"
        ),
        *bullets(
            [
                "<b>Ist.</b> Läuft. Nachprüfbar. Jede Zahl hat eine Quelle im Repository.",
                "<b>Grenze.</b> Was Nightfall bewusst nicht ist. Der Graph ist nicht gelöscht. Es gibt kein externes Audit. Der Swap ist auf Mainnet zu.",
                "<b>Horizont.</b> Vorschläge. Kein Lieferdatum, kein Versprechen. Konsens ändert sich nur mit Spec, Tests und einem unabhängigen Review.",
            ]
        ),
        Spacer(1, 8),
        Callout(
            "Dieses Dokument behauptet nicht, 100 % anonym, untraceable oder "
            "protocol-scale zu sein. Wer das braucht, ist hier falsch — und das "
            "steht so, weil genau diese Sätze das Projekt schon einmal teurer "
            "gekommen sind als jeder Bug im Miner."
        ),
        Spacer(1, 16),
        P("INHALT", "kicker"),
        toc_tbl,
        Spacer(1, 10),
        P(
            "Primärquellen: SPEC.md, PRIVACY.md, DECISIONS.md, HISTORY.md, RESET.md, "
            "MANIFESTO.md, SWAP.md, AUDIT-2026-08-16.md. "
            "Öffentliches Repo: github.com/Instinctes/nightfall.",
            "caption",
        ),
        CondPageBreak(210),
        # --- 01 ---
        Chapter("01", "Die These"),
        P(
            "Nightfall ist Geld, das nicht petzt — und bei dem niemand still "
            "nachdrucken kann.",
            "lead",
        ),
        P(
            "Transparente Ledger haben jede Wallet in einen öffentlichen Kontoauszug "
            "verwandelt. Die übliche Antwort der Branche ist ein Schalter: Privacy als "
            "Premium, als Pool, als Opt-in, das unter Druck zum Opt-out wird. Nightfall "
            "nimmt den Schalter raus. Beträge sind verborgen, Adressen berühren die "
            "Kette nicht, jeder Block ist ein CoinJoin, ohne dass sich jemand verabredet. "
            "Und nach jedem Block prüft jeder Node eine Gleichung, die Inflation "
            "ausschließt — nicht als Versprechen, als Arithmetik."
        ),
        Equation(),
        P(
            "Die Supply-Invariante. Gleichheit der Commitments allein beweist nichts; "
            "das war der v4-Fehler. Die Excess-Signatur über Generator H trägt die Soundness.",
            "caption",
        ),
        P(
            "Die Position, die daraus folgt, ist eng und deshalb haltbar: Nightfall ist "
            "eine private Settlement-Schicht. Digitales Bargeld. Selektive Offenlegung "
            "gehört dem Besitzer — View-Key, Receipt, sonst nichts. Es ist nicht die "
            "schnellere Smart-Contract-Chain, nicht Monero mit anderem Logo, nicht "
            "Grin, dem man Cut-through wieder andreht."
        ),
        Callout(
            "Die Differenzierung, die man nachprüfen kann: private amounts, no addresses "
            "on chain, a supply anyone can prove, a graph mixed per block — not erased.",
            "ok",
        ),
        CondPageBreak(210),
        # --- 02 ---
        Chapter("02", "Die Kette, wie sie läuft"),
        P(
            "Eine eigenständige Proof-of-Work-L1. Kein Premine, kein Admin-Key, kein "
            "Tail. Gebühren brennen, solange es eine Subsidy gibt; danach bezahlen sie "
            "den Miner, ohne etwas zu minten. 1 NIGHT = 10⁸ darks."
        ),
        facts,
        Spacer(1, 8),
        P("Begrabene Ketten", "h2"),
        P(
            "Nightfall ist mehr als einmal zurückgesetzt worden. Das ist kein Geheimnis, "
            "und es wird nicht als Stärke verkauft. Es ist die Liste, damit niemand sie "
            "aus Tags rekonstruieren muss. Münzen wandern nicht. Eine Balance auf "
            "c8614333… ist ein Souvenir."
        ),
        hist,
        Spacer(1, 8),
        P(
            "0.6.x kann mit dieser Kette nicht handshaken (NFL1 gegen NFL2). Alte Daten "
            "bleiben in nightfall/mainnet/. Diese Kette schreibt nightfall/mainnet/n8/.",
            "caption",
        ),
        Callout(
            "WIRE_VERSION und PROTOCOL_VERSION liegen vier Zeilen auseinander und "
            "bedeuten Gegensätzliches. Wire ist die Netzwerksperre. Protocol fließt in "
            "den Genesis. Wer „alte Wallets sollen nicht mehr minen“ mit der falschen "
            "Konstante löst, hat einen Chain-Reset gebaut."
        ),
        CondPageBreak(210),
        # --- 03 ---
        Chapter("03", "Kryptographie, ohne Theater"),
        P(
            "Die Suite ist klein, absichtlich. Eine Kurve, ein Encoding, ein Hash, "
            "ein Commitment-Schema, ein Range-Proof, ein Signaturverfahren. "
            "Mehr Primitive wären mehr Nahtstellen — und Nahtstellen waren v4."
        ),
        suite,
        Spacer(1, 10),
        P("Einseitige Stealth-Outputs", "h2"),
        P(
            "Der Sender zieht ein Ephemeral r und veröffentlicht Ke = r·G. Empfänger "
            "rechnen t = H(„shared“ ‖ a·Ke), denselben Wert, den der Sender als "
            "H(„shared“ ‖ r·A) hat. Daraus sitzen Blinding, One-Time-Offset und "
            "Payload-Key. On-chain steht kein Empfängeridentifikator."
        ),
        P(
            "Ausgeben von Ko braucht b_spend + o. Der Sender kennt o, nicht b_spend — "
            "er kann nicht zurückholen, was er geschickt hat. Das ist der Grund, warum "
            "Nightfall einseitige Zahlungen kann, und der Grund, warum Cut-through "
            "heute nicht geht: die Input-Signatur unter Ko ist die Autorisierung, die "
            "ein Block beim Zusammenlegen nicht wegwerfen darf."
        ),
        P("Was eine Signatur bindet — und was nicht", "h2"),
        P(
            "Kernel-Nachricht = H(feature, fee, reward, lock_height, excess). "
            "Input-Nachricht = H(commit). Output-Nachricht = H(features, commit, Ke, Ko, "
            "proof, payload). Keine der drei bindet einen Transaktionskörper. Das ist "
            "Absicht: Aggregation löst Transaktionen auf, eine körpergebundene Signatur "
            "würde danach nicht mehr verifizieren. Es ist auch die harte Grenze für "
            "„vorsignierte“ Verträge. Wer lock_height in einem Input-Sig vermutet, "
            "hat Bitcoin-Intuition auf die falsche Kette übertragen."
        ),
        Callout(
            "v4 lehrte die teuerste Lektion des Projekts: eine Prüfung, die immer "
            "aufgeht, ist keine Prüfung. Die Excess-Signatur über H existiert, weil "
            "Gleichheit der Summen eine Tautologie war."
        ),
        CondPageBreak(210),
        # --- 04 ---
        Chapter("04", "Privacy, ehrlich"),
        P(
            "Nightfall verbirgt Beträge und Adressen. Es löscht den Transaktionsgraphen "
            "nicht, und es versteckt den ersten Hop nicht, wenn Tor aus ist und der "
            "nächste Peer ein alter Node ist, der sofort flufft. Die ehrliche Zeile "
            "steht in PRIVACY.md. Sie wird hier nicht weicher."
        ),
        priv,
        Spacer(1, 10),
        P("Was schon in der Hand ist", "h2"),
        P(
            "Der View-Key nfview1 sieht jeden Betrag und jedes Memo, bewegt keine Münze. "
            "Ein Receipt öffnet genau ein Output (Betrag + Blinding), signiert mit dem "
            "Spend-Key der nf1-Adresse. Ein Prüfer verifiziert Opening und Signatur, "
            "ohne den Rest der Wallet zu sehen. Das ist selektive Offenlegung als "
            "Produkt, nicht als Roadmap-Stichwort."
        ),
        P(
            "Auf dem Netz: Dandelion-class Stem/Fluff auf der bestehenden Tx-Nachricht, "
            "Tor default. Eine neue Transaktion geht an genau einen zufälligen Peer. "
            "Fällt dieser Sprung aus, ist sie weg — 0.8.2 hebt das, indem die Wallet "
            "unbestätigte Sends nach jedem Sync erneut einreicht. Stem ist Herkunftsverschleierung, "
            "kein Lieferversprechen."
        ),
        Callout(
            "Die Anonymitätsmenge ist die Menge der anderen Transaktionen im selben "
            "Block, plus wie viele Menschen die Kette wirklich nutzen. Ein leerer Block "
            "mit einer Zahlung ist ein CoinJoin mit einer Person. Das zu verschweigen "
            "wäre Marketing. Nightfall macht das nicht.",
            "warn",
        ),
        CondPageBreak(210),
        # --- 05 ---
        Chapter("05", "Geld und Mining"),
        P(
            "Era-0-Reward 6 NIGHT, Halving alle 7,5 Millionen Blöcke, harte Decke "
            "90 Millionen, kein Tail. Weil jede Halbierung abschneidet "
            "(<font face='SFMono' color='#9b83ff'>reward &gt;&gt; halvings</font>), "
            "endet die Kurve bei 89 999 999,25 — 0,75 unter der Decke. Die Decke ist "
            "eine Obergrenze, kein Soll-Stand."
        ),
        P(
            "PoW ist Nighthash-v2: Argon2id, 32 MiB, eine Iteration, eine Lane. "
            "Memory-hardness ist der Punkt — zweckgebaute Hardware braucht so viel "
            "schnellen RAM pro paralleler Einheit. Ein Hash auf Mainnet kostet denselben "
            "Aufwand beim Verifizieren wie beim Minen, rund 11 ms. Deshalb führt ein "
            "Node eine lokale Validierungsmarkierung und hasht die eigene Kette nicht "
            "bei jedem Start neu."
        ),
        P(
            "Retarget jeden Block, LWMA-1 über 90 Blöcke, Ziel 15 s, Solvetime geklemmt "
            "auf [−5T, +6T], Bewegung ±2×, Floor 2000. Ein langsamer Block senkt die "
            "Difficulty um höchstens etwa 10 %. Es gibt keine zeitbasierte Notbremse. "
            "Difficulty ≈ erwartete Hashes pro Block; Netz-Hashrate ≈ Difficulty / 15."
        ),
        Callout(
            "Eine Geldpolitik, die auf den Marktpreis reagiert, ist keine Geldpolitik. "
            "Emission, Floor und Fee-Regel bleiben, solange die Kette jung ist. "
            "Änderungen nur nach Sicherheitsanalyse — nicht nach einem Chart."
        ),
        P(
            "Fees sind öffentlich im Kernel, damit der Burn auditierbar ist. Solange "
            "Subsidy fließt, brennen sie vollständig; der Miner bekommt nur die Subsidy. "
            "Die Wallet bietet deshalb keine Priority-Stufen an: der Mempool ordnet "
            "nicht nach Gebühr, und „Priority“ wäre eine Beschleunigung, die nichts "
            "liefert.",
            "body",
        ),
        CondPageBreak(210),
        # --- 06 ---
        Chapter("06", "Was man heute in der Hand hält"),
        P(
            "Core ist Full Node, Miner und Wallet in einem Fenster. nightfalld ist "
            "derselbe Node ohne GUI. Die Web-Wallet unter nightfallcoin.org/wallet/ "
            "ist ein Light-Client: sie führt keine Kettendatei, sie scannt stealth "
            "outputs über die Light-API der Seeds. APK und IPA sind von der Website "
            "genommen — unsignierte Sideload-Builds sind eine schlechtere erste "
            "Viertelstunde als eine Seite, die aufgeht."
        ),
        *bullets(
            [
                "24 Wörter (BIP-39, bijektiv zum 32-Byte-Seed). Hex darunter. Ohne Phrase gibt es die Münzen nicht zurück.",
                "Addressbuch lokal. Receipts und View-Key in der Wallet.",
                "Prune: UTXO + Header + letzte 500 Bodies. Seeds, die IBD oder Light-API geben, bleiben Archive.",
                "blocks.bin seit 0.8.0 — 72 % weniger Platte als JSON, kein Konsenswechsel. P2P bleibt zeilenweises JSON.",
                "Ein Prozess pro Datadir. Die Sperre hält das Betriebssystem (flock / LockFileEx), keine PID-Datei.",
                "Chain-Archive für den Einstieg, wöchentlich, ohne das vorige still vom Netz zu nehmen.",
            ]
        ),
        Spacer(1, 6),
        P(
            "Light-API (Port 17888, öffentlich über den Seed): status, scan_feed, "
            "submit_tx, get_utxo_root, banner, get_headers. get_blocks bleibt draußen — "
            "volle Bodies mit Range-Proofs sind ein Verstärker für jeden, der zweimal "
            "fragt. Die Allow-Liste sitzt im Knoten, nicht nur im Website-Worker. "
            "Ein Light-Client glaubt dem Node beim Anzeigen; ausgeben tut nur der "
            "Spend-Key auf dem Gerät."
        ),
        Callout(
            "Phone und Browser vertrauen dem Node bei der Anzeige, nicht beim Spend. "
            "Der Seed im localStorage der Web-Wallet ist der Seed. Wer die Seite "
            "löscht, löscht das Adressbuch; die 24 Wörter holen es nicht zurück."
        ),
        CondPageBreak(210),
        # --- 07 ---
        Chapter("07", "Atomic Swap — und der Wermutstropfen"),
        P(
            "NIGHT ↔ BTC, wallet-to-wallet, Copy-Paste-Pakete, kein Operator, kein "
            "Postfach. Bitcoin-Seite: P2WSH 2-of-2, ECDSA-Adaptor, CSV-Abort-Baum "
            "(lock / redeem / cancel / refund / punish). NIGHT-Seite: gemeinsame "
            "Stealth-Adresse, Spend s<sub>a</sub> + s<sub>b</sub> + offset. Der Adaptor "
            "lebt nur auf Bitcoin."
        ),
        P(
            "Mainnet startet keinen Swap. Testnet und Devnet tun es. Das "
            "kurvenübergreifende DLEQ benutzt ein eigenes Ristretto-Blatt in einem "
            "geprüften Kombinator-Crate. Niemand außerhalb des Projekts hat dieses "
            "Blatt signiert. Die Sperre bleibt, bis sie absichtlich fällt."
        ),
        P(
            "v0.1 wollte den bekannten Wermutstropfen von XMR↔BTC beseitigen: eine "
            "zeitgesperrte NIGHT-Rückerstattung, weil Nightfall lock_height hat und "
            "Monero nicht. Die Stelle war als angreifbar markiert. Sie wurde am selben "
            "Tag zerlegt. Die Input-Signatur bindet H(commit) und sonst nichts. Eine "
            "co-signierte Input-Signatur ist keine vorsignierte Transaktion — sie passt "
            "zu jedem balancierenden Kernel. Die Senderin des Lock-Outputs kennt den "
            "Blinding-Faktor und kann einen Kernel mit lock_height = 0 selbst setzen. "
            "Dann das BTC einlösen. Beide Seiten, kein Wettlauf."
        ),
        Callout(
            "In Mimblewimble ist „vorsignieren“ kein verfügbares Werkzeug. "
            "Bitcoin-Intuition überträgt sich nicht. v0.1 liegt als "
            "SWAP-SPEC-v0.1-withdrawn.md, damit der Fehler lesbar bleibt. "
            "v0.3 ist das bekannte Protokoll, Wermutstropfen akzeptiert und in der "
            "Oberfläche sichtbar: es gibt keine NIGHT-Rückerstattung. Wenn die "
            "Gegenseite cancelled und nie refundet, ist das gelockte NIGHT für immer fest.",
            "warn",
        ),
        P(
            "Alice darf nicht nah an H₁ einlösen. Die Bitcoin-Node würde den Redeem "
            "annehmen — nichts auf Bitcoin weiß von H₁, wenn es den Redeem sieht. "
            "Die Wallet verweigert ihn. Das ist Politik, nicht Kette, und Lauf 4 gegen "
            "bitcoind hat genau das gezeigt."
        ),
        CondPageBreak(210),
        # --- 08 ---
        Chapter("08", "Was fehlt, und warum das so gesagt wird"),
        P(
            "Die Spec führt Lücken absichtlich. Ein Whitepaper, das sie weglässt, "
            "wäre Werbung. Die folgenden Punkte sind keine Schande — sie sind die "
            "Stellen, an denen die nächste Arbeit ansetzen muss, falls sie ansetzen soll."
        ),
        gaps,
        Spacer(1, 10),
        P(
            "Was 0.8.x und 0.9.x bereits geschlossen haben, damit die Lückenliste "
            "nicht die Geschichte von vor drei Wochen erzählt: binäres Plattenformat, "
            "Prune, Introducer-Modus, Checkpoint 25 000, Mempool-Ablauf, erneutes "
            "Einreichen unbestätigter Sends, Datadir-Sperre, inkrementelle "
            "Supply-Summe, Chain-Archive, get_headers für die Kettenseite. "
            "Konsens, Genesis, Wire: unverändert."
        ),
        CondPageBreak(210),
        # --- 09 ---
        Chapter("09", "Unverhandelbar"),
        P(
            "Die Zukunft wird nicht über eine Featureliste definiert. Jede neue "
            "Funktion muss diese Sätze überleben. Sie stehen im Manifest, in der Spec "
            "und in den Stellen, an denen das Projekt schon einmal gebrannt hat."
        ),
        *bullets(
            [
                "<b>Privacy is the default.</b> Kein transparenter Basispfad, der aus Bequemlichkeit zur Norm wird.",
                "<b>Proof, not trust.</b> Geldbestand, Besitz, ausgewählte Aussagen — kryptographisch, nicht als Policy.",
                "<b>Disclosure is granular.</b> Eine Zahlung, ein Fenster, eine Untergrenze. Nie die ganze Historie als Preis für eine Aussage.",
                "<b>No privileged money.</b> Kein Premine, keine Treasury-Münzen, keine nachträgliche Sonderemission, kein Admin-Freeze.",
                "<b>Consensus minimalism.</b> So wenig Logik wie möglich in den Konsens. Anwendung off-chain oder in eigenen Beweisen.",
                "<b>The author does not self-certify.</b> Wer eine kryptographische Konstruktion schreibt, gibt nicht allein ihre Sicherheitsfreigabe.",
                "<b>Interop without custodians.</b> Atomic exchange, keine Bridge, kein wrapped NIGHT als Basis.",
                "<b>Say the limit.</b> Nie „100 % anonym“. Nie einen Preis erfinden. Nie Peer-Versionen als Tatsache ausgeben — das ist Selbstauskunft, erzwungen ist nur WIRE_VERSION.",
                "<b>Code before narrative.</b> Das Repo ist die Wahrheit. Der Thread ist Marketing.",
                "<b>Exit is sacred.</b> Wenn die Kette gekapert wird, muss sich die Community forken können, ohne zu bitten.",
            ]
        ),
        Spacer(1, 8),
        Callout(
            "Anti-MEV steht nicht auf dieser Liste, weil es auf einer 15-Sekunden-CPU-Kette "
            "ohne VM und mit brennenden Fees kein Geschäftsmodell ist. Ein Prinzip, das "
            "ein Problem beschreibt, das die Konstruktion nicht hat, ist Kostüm.",
            "note",
        ),
        CondPageBreak(210),
        # --- 10 ---
        Chapter("10", "Horizont"),
        P(
            "Fünf Jahre sind lang genug, um die Architektur zu formen, und kurz genug, "
            "dass jedes Datum in einer Tabelle zur Lüge wird. Deshalb keine Kalender-Roadmap. "
            "Capability-based: eine Stufe ist fertig, wenn das Kriterium hält — nicht "
            "wenn ein Quartal um ist. Konsensrelevante Kryptographie darf sich verspäten, "
            "wenn die Beweislage nicht reicht."
        ),
        future,
        Spacer(1, 10),
        P("Die eigentliche Forschungsfrage", "h2"),
        P(
            "Einseitige Zahlungen brauchen eine per-Input-Signatur unter der One-Time-Key. "
            "Cut-through würde genau diese Signatur löschen. Beides gleichzeitig gibt es "
            "in der heutigen Konstruktion nicht. Wer den Graphen löschen will, muss eine "
            "Spend-Autorisierung bauen, die ohne veröffentlichte per-Input-Signatur "
            "auskommt und den Sender trotzdem nicht zurückholen lässt. Das ist Research, "
            "kein Sprint. Bis dahin bleibt der Satz stehen, den die Spec schon sagt: "
            "der Graph ist verdunkelt, nicht gelöscht."
        ),
        P(
            "Was Nightfall nicht aus Monero importiert: TxTangle als Middleware, "
            "Ring-Signatures, einen Escrow-Marktplatz mit Moderator, I2P als zweiten "
            "Default. Der Block aggregiert bereits ohne Koordination. Eine Session, "
            "in der Wallets sich zu einem Join verabreden, führt einen Operator durch "
            "die Hintertür ein — genau die Klasse, die der Swap bewusst nicht hat."
        ),
        Callout(
            "Spec vor Code hat beim Swap in einem Nachmittag das zerlegt, was Monate "
            "und fremdes Geld gekostet hätte. Dieselbe Reihenfolge gilt für jede "
            "konsensnahe Idee auf diesem Horizont."
        ),
        CondPageBreak(210),
        # --- 11 ---
        Chapter("11", "Was wir nicht bauen"),
        *bullets(
            [
                "Keine allgemeine Smart-Contract-VM auf Layer 1.",
                "Keine Bridge, kein wrapped NIGHT, kein transparentes Unshield.",
                "Keine Börse, keinen Ticker, kein offizielles Pair, kein Datum dafür.",
                "Kein Premine, keine Team-Allocation, keine VC-Runde, keine Treasury.",
                "Kein Admin-Key, der einfrieren, beschlagnahmen oder umschreiben kann.",
                "Keine Pflicht-Identität, keinen Compliance-Mode, der Privacy zum Opt-in macht.",
                "Keine kurzfristige Tokenomics-Änderung, um einen Preis zu setzen — es gibt keinen offiziellen Preis.",
                "Kein zweiter Seed im selben Vultr-Account, kein Hochdrehen von MAX_PEERS als Kapazitätsersatz.",
                "Kein Claim „untraceable“ / „100 % anonym“ / „protocol-scale anonymity set“.",
            ]
        ),
        Spacer(1, 8),
        Callout(
            "Diese Begrenzungen sind keine Schwäche. Sie schützen die Kernfunktion: "
            "verlässliches privates Geld, dessen Supply jeder Node beweist.",
            "ok",
        ),
        Spacer(1, 14),
        P("Wo Nightfall steht, neben den anderen", "h2"),
        pos,
        CondPageBreak(210),
        # --- 12 ---
        Chapter("12", "Schluss"),
        P(
            "Die Grundidee ist kein Konzept mehr. Die Kette ist jung genug, dass sich "
            "die Architektur noch formen lässt, und alt genug, dass sie schon einmal "
            "an einer Tautologie gestorben ist. Die entscheidende Frage ist deshalb "
            "nicht, welche zehn Features als Nächstes kommen. Sie lautet: welche "
            "Eigenschaften in fünf Jahren noch unverhandelbar sein sollen."
        ),
        P(
            "Heute: Konsens härten, fremdes Review, Netzprivacy, Wallet-Zuverlässigkeit. "
            "Dann: State und Proofs skalieren, ohne die Invariante zu opfern. Parallel: "
            "den Swap nur öffnen, wenn das Blatt geprüft ist. Langfristig, und nur wenn "
            "die Mathematik es hergibt: eine Spend-Autorisierung, die einseitige "
            "Zahlungen und Graph-Löschung nicht mehr zu Feinden macht."
        ),
        Spacer(1, 8),
        Callout(
            "Wenn die Welt jede Zahlung zur öffentlichen Beichte macht, ist NIGHTFALLCOIN "
            "das Recht zu schweigen — erzwungen durch Kryptographie, nicht durch Policy, "
            "PR oder die Freundlichkeit von Validatoren.",
            "quote",
        ),
        Spacer(1, 10),
        P(
            "Nicht „Nightfall wird Monero 2.0“. Eine eigene Generation privater "
            "Settlement-Infrastruktur: weniger Altlast, ein expliziter Supply-Beweis, "
            "blockweite Aggregation, ein enger Konsens, und eine Research-Roadmap, "
            "die das sagt, was sie nicht kann."
        ),
        P(
            "Wenn diese Richtung hält, entsteht kein Privacy-Feature. Es entsteht ein "
            "digitales Bargeldsystem, bei dem die Standardfrage nicht mehr lautet "
            "„Warum willst du diese Zahlung verbergen?“, sondern „Welche Information "
            "möchtest du freiwillig beweisen?“."
        ),
        Spacer(1, 16),
        Rule(),
        Spacer(1, 8),
        P("Born when the lights go out.", "quote"),
        P("NIGHTFALLCOIN · nightfallcoin.org · Instinctes/nightfall", "caption"),
        CondPageBreak(210),
        # --- appendix ---
        P("ANHANG", "kicker"),
        P("Invarianten für jede künftige Consensus-Version", "h2"),
        *bullets(
            [
                "Supply kann niemals außerhalb des Emissionsschemas steigen.",
                "Jeder Spend braucht kryptographische Autorisierung unter der One-Time-Key des UTXO-Sets.",
                "Jedes Output trägt einen gültigen, netzgebundenen Range Proof.",
                "Kein Output wird zweimal ausgegeben.",
                "Σ UTXO − Σ kernel_excess = (minted − burned)·G hält nach jedem Block. Die Gleichung ist kein Ersatz für Soundness-Beweise.",
                "WIRE_VERSION ändert den Handshake, nicht den Genesis. PROTOCOL_VERSION ändert den Genesis.",
                "Disclosure-Mechanismen legen keine Spending Authority offen.",
                "Cross-chain-Adapter führen keine implizite Custody ein.",
                "Reorgs tiefer als 500 werden abgelehnt. Ein Wallet-Zustand darf nach einem Reorg nicht dauerhaft inkonsistent bleiben.",
                "Netzwerknachrichten dürfen keine unbounded Allocations auslösen.",
            ]
        ),
        Spacer(1, 8),
        P("Quellen und Abgrenzung", "h2"),
        P(
            "Ist-Zustand: docs/SPEC.md, docs/PRIVACY.md, docs/DECISIONS.md, "
            "docs/HISTORY.md, docs/RESET.md, docs/MAINNET.md, MANIFESTO.md, "
            "crates/nightfall-types, nightfall-crypto, nightfall-ledger, "
            "nightfall-consensus. Swap: docs/SWAP.md, SWAP-SPEC-DRAFT.md v0.3, "
            "SWAP-SPEC-v0.1-withdrawn.md, SWAP-LOSS.md, SWAP-ATTACKS.md. "
            "Internes Review: docs/AUDIT-2026-08-16.md. v4: docs/AUDIT-2026-08-12.md."
        ),
        P(
            "Alles, was in Kapitel 10 als Spur, Horizont oder Research bezeichnet wird, "
            "ist ein Vorschlag. Es ist keine Aussage, dass die Funktion im Repository "
            "steht. Stand der Software, die dieses Papier beschreibt: 0.9.2, "
            "September 2026, dieselbe Kette wie am Genesis-Tag."
        ),
        Spacer(1, 16),
        P(
            "Fair launch. 0 % Premine. 0 % Team. 0 % VC. Public genesis. "
            "Mine or receive.",
            "small",
        ),
    ]
    return s


def story_en() -> list:
    W = CONTENT_W
    c3 = [W * 0.28, W * 0.36, W * 0.36]
    c2 = [W * 0.30, W * 0.70]
    toc = [
        ["01", "The thesis"],
        ["02", "The chain as it runs"],
        ["03", "Cryptography, without theatre"],
        ["04", "Privacy, honestly"],
        ["05", "Money and mining"],
        ["06", "What you can hold today"],
        ["07", "Atomic swap — and the wart"],
        ["08", "What is missing, and why we say so"],
        ["09", "Non-negotiable"],
        ["10", "Horizon"],
        ["11", "What we will not build"],
        ["12", "Close"],
    ]
    toc_tbl = Table(
        [[Paragraph(n, STY["tocn"]), Paragraph(t, STY["toct"])] for n, t in toc],
        colWidths=[28, W - 28],
    )
    toc_tbl.setStyle(
        TableStyle(
            [
                ("VALIGN", (0, 0), (-1, -1), "MIDDLE"),
                ("TOPPADDING", (0, 0), (-1, -1), 3.5),
                ("BOTTOMPADDING", (0, 0), (-1, -1), 3.5),
                ("LEFTPADDING", (0, 0), (-1, -1), 0),
            ]
        )
    )

    facts = grid(
        [
            ["", "Value", "Meaning"],
            ["Asset", "NIGHTFALLCOIN (NIGHT)", "Native L1, not a token on someone else's chain"],
            ["Genesis", "061a052d…20de", "16 August 2026. Coins from earlier geneses do not exist here."],
            ["Protocol / wire", "v8 / v6, magic NFL2", "WIRE locks out old nodes. PROTOCOL is in the genesis — touch it and you have a new chain."],
            ["Datadir", "nightfall/&lt;net&gt;/n8/", "v7 data stays where it is. Delete nothing."],
            ["Cap", "90,000,000 NIGHT", "No tail. Terminal 89,999,999.25 — 0.75 short of the ceiling."],
            ["Block", "6 NIGHT, halving 7,500,000, target 15 s", "About 3.56 years to the first halving."],
            ["PoW", "Nighthash-v2, Argon2id 32 MiB", "CPU. Verifying costs the same hash. No GPU/ASIC advantage."],
            ["Difficulty", "LWMA-1, window 90, floor 2000", "Cumulative work decides, never block count."],
            ["Fees", "Burned while subsidy &gt; 0", "Afterwards coinbase = fees. Public, so the burn is auditable."],
            ["Premine / admin", "0 / none", "No freeze, no mint, no treasury coins."],
            ["Reorg depth", "500 blocks", "Deeper is refused. Checkpoint at height 25,000."],
            ["Software", "0.9.2 (September 2026)", "Consensus unchanged since v8. Same chain."],
        ],
        [W * 0.22, W * 0.38, W * 0.40],
    )

    suite = grid(
        [
            ["Piece", "Choice", "Why"],
            ["Curve", "Ristretto (curve25519), everywhere", "One curve, one encoding. v4 mixed ed25519 and x25519."],
            ["Hash", "Blake3, domain-separated", "Length-prefixed. No silent concatenation."],
            ["Commitments", "Pedersen C = v·G + b·H", "H is NUMS. Soundness rests on nobody knowing x with H = x·G."],
            ["Range proofs", "Bulletproofs, 64-bit, per output", "v in [0, 2⁶⁴). Network-bound, not replayable."],
            ["Signatures", "Schnorr on Ristretto", "Kernel over generator H, inputs over Ko, outputs over Ke."],
            ["Payload", "XChaCha20-Poly1305", "Fixed length. Ciphertext does not leak the memo."],
            ["Address", "nf1 + scan PK + spend PK", "Never appears on chain. View key: nfview1, sees, cannot sign."],
        ],
        c3,
    )

    priv = grid(
        [
            ["Property", "Status"],
            ["Amounts", "Hidden (Pedersen + Bulletproofs)"],
            ["Recipient address", "Never appears. nf1 becomes a one-time key Ko."],
            ["Sender address", "Never appears."],
            ["Transparent pool", "None. Privacy is mandatory, not a switch."],
            ["View key", "nfview1 — finds and opens every output, cannot sign."],
            ["Receipt", "Proves exactly one payment, without the view key."],
            ["Graph", "Obscured by block aggregation, <b>not erased</b>."],
            ["Cut-through", "Not applied. It would delete the input signature that makes one-sided payments safe."],
            ["Dandelion", "Stem/fluff on the existing Tx message. 90 % stem."],
            ["Tor / SOCKS5", "<b>Default</b> 127.0.0.1:9050. Clearnet fallback if Tor is down."],
            ["Anonymity set today", "The other transactions <i>in the same block</i>. The network is small. Not protocol-scale."],
            ["Independent audit", "None. Internal review: 16 August 2026."],
        ],
        c2,
    )

    hist = grid(
        [
            ["Epoch", "Genesis", "Why it ended"],
            ["v4 Nightproof-α", "discarded", "The balance check was a tautology. Anyone could mint."],
            ["v5 / v6", "discarded", "The crypto sounded. Networking and reorg-wallet did not."],
            ["v7", "c8614333…", "Emission too front-loaded. Two miners held the float. Buried."],
            ["v8 / n8 — this chain", "061a052d…", "Live. 6 NIGHT / 7.5 M blocks. Magic NFL2."],
        ],
        [W * 0.24, W * 0.22, W * 0.54],
    )

    gaps = grid(
        [
            ["Gap", "What that means in practice"],
            ["Cut-through", "Spent outputs stay visible. One-sided payments and cut-through exclude each other for as long as every input carries a public signature."],
            ["Dandelion++", "Today the stem uses the same gossip set as blocks. No separate stem graph, no fail-safe fluff on black-hole stems."],
            ["Independent audit", "v5/v8 was written by the same party that tore v4 apart. The internal review counts as a review, not as a stranger's eye."],
            ["IBD / verify", "Each hash is ~11 ms. A year of chain is hours of CPU. Prune and archives exist; headers-first and MMR do not."],
            ["UTXO root", "O(n log n) per block. A Merkle Mountain Range would make it incremental."],
            ["Hashrate", "Small. Reorgs are cheaper than on a mature chain. That is why mining holds on forks, reorg cap 500, checkpoint 25,000."],
            ["Seeds", "Two machines, one operator. Discovery is a single point. Consensus is not — every node validates alone."],
        ],
        c2,
    )

    future = grid(
        [
            ["Track", "What", "What not"],
            ["Network", "Real Dandelion++: its own stem path, fail-safe fluff, Tor stays default.", "No I2P promise. No second overlay before the first holds."],
            ["Sync", "MMR or a related accumulator, headers-first, bound snapshots. Prune and weekly archives already exist.", "No UTXO snapshot from a stranger as a trust anchor."],
            ["Graph", "Research: spend authorisation without per-input Schnorr that still keeps one-sided payments. Only then cut-through.", "No wallet-coordinated CoinJoin as a substitute. The block already mixes, without an appointment."],
            ["Disclosure", "Receipts and view keys exist. On top: time windows, lower bounds, a machine-readable schema with expiry.", "No compliance mode that turns privacy into an opt-in."],
            ["Swap", "External review of the Ristretto DLEQ leaf. Only then open the mainnet gate.", "Do not invent a NIGHT refund. v0.1 is withdrawn because pre-signing does not exist here."],
            ["Engineering", "A harness that starts two or three nodes, lets them diverge, and asserts convergence. The crypto suite catches its bugs. The network so far has not.", "No feature list that bloats consensus."],
        ],
        c3,
    )

    pos = grid(
        [
            ["Approach", "What it can do", "Nightfall's answer"],
            ["Bitcoin", "Neutral settlement, cumulative work, no admin key.", "The same stance, native privacy, a supply anyone can prove."],
            ["Monero", "Mature privacy currency, protocol-scale anonymity set.", "Not a RingCT clone. Supply here is an equation, not a decoy assumption. The graph is not erased — that is said."],
            ["Grin / Mimblewimble", "Aggregation, cut-through, elegant confidential transactions.", "Aggregation yes. Cut-through no, because one-sided payments need the input signature. A choice, not a forgotten feature."],
            ["Zcash-like", "Strong ZK privacy, selective disclosure.", "No trusted setup. Proof research later, not as the foundation."],
            ["Smart-contract L1", "Programmability, DeFi, MEV.", "No VM on L1. Narrow waist: UTXO, confidential value, proof, signature. Everything else off-chain."],
        ],
        c3,
    )

    return [
        P("HOW TO READ THIS PAPER", "kicker"),
        P("The spec is law. This paper is the argument.", "lead"),
        P(
            "NIGHTFALLCOIN is live. Genesis <font face='SFMono' color='#9b83ff'>061a052d…</font>, "
            "protocol v8, wire v6, magic NFL2, software 0.9.2. The numbers in chapter 2 "
            "live in <font face='SFMono' color='#9b83ff'>docs/SPEC.md</font> and in the code. "
            "If you need a consensus rule, look there — not here."
        ),
        P(
            "Three layers, kept apart on purpose, so a vision is not read as a shipped feature:"
        ),
        *bullets(
            [
                "<b>Is.</b> Running. Checkable. Every figure has a source in the repository.",
                "<b>Limit.</b> What Nightfall deliberately is not. The graph is not erased. There is no external audit. The swap is closed on mainnet.",
                "<b>Horizon.</b> Proposals. No delivery date, no promise. Consensus changes only with a spec, tests, and an independent review.",
            ]
        ),
        Spacer(1, 8),
        Callout(
            "This document does not claim to be 100 % anonymous, untraceable, or "
            "protocol-scale. If that is what you need, you are in the wrong place — "
            "and it is written that way because those sentences have already cost "
            "this project more than any bug in the miner."
        ),
        Spacer(1, 16),
        P("CONTENTS", "kicker"),
        toc_tbl,
        Spacer(1, 10),
        P(
            "Primary sources: SPEC.md, PRIVACY.md, DECISIONS.md, HISTORY.md, RESET.md, "
            "MANIFESTO.md, SWAP.md, AUDIT-2026-08-16.md. "
            "Public repo: github.com/Instinctes/nightfall.",
            "caption",
        ),
        CondPageBreak(210),
        Chapter("01", "The thesis"),
        P(
            "Nightfall is money that refuses to snitch — and that nobody can quietly reprint.",
            "lead",
        ),
        P(
            "Transparent ledgers turned every wallet into a public bank statement. The "
            "industry's usual answer is a switch: privacy as a premium, a pool, an opt-in "
            "that under pressure becomes an opt-out. Nightfall takes the switch out. "
            "Amounts are hidden, addresses never touch the chain, every block is a "
            "CoinJoin without anyone arranging it. And after every block every node "
            "checks an equation that rules out inflation — not as a promise, as arithmetic."
        ),
        Equation(),
        P(
            "The supply invariant. Equality of commitments proves nothing; that was the "
            "v4 bug. The excess signature over generator H carries the soundness.",
            "caption",
        ),
        P(
            "The position that follows is narrow and therefore holdable: Nightfall is a "
            "private settlement layer. Digital cash. Selective disclosure belongs to the "
            "owner — view key, receipt, nothing else. It is not the faster smart-contract "
            "chain, not Monero with a different logo, not Grin with cut-through bolted back on."
        ),
        Callout(
            "The differentiation you can check: private amounts, no addresses on chain, "
            "a supply anyone can prove, a graph mixed per block — not erased.",
            "ok",
        ),
        CondPageBreak(210),
        Chapter("02", "The chain as it runs"),
        P(
            "A sovereign proof-of-work L1. No premine, no admin key, no tail. Fees burn "
            "while a subsidy exists; afterwards they pay the miner without minting anything. "
            "1 NIGHT = 10⁸ darks."
        ),
        facts,
        Spacer(1, 8),
        P("Buried chains", "h2"),
        P(
            "Nightfall has been reset more than once. That is not a secret, and it is not "
            "sold as a strength. It is the list, so nobody has to reconstruct it from tags. "
            "Coins do not migrate. A balance on c8614333… is a souvenir."
        ),
        hist,
        Spacer(1, 8),
        P(
            "0.6.x cannot handshake with this chain (NFL1 versus NFL2). Old data stays in "
            "nightfall/mainnet/. This chain writes nightfall/mainnet/n8/.",
            "caption",
        ),
        Callout(
            "WIRE_VERSION and PROTOCOL_VERSION sit four lines apart and mean opposite "
            "things. Wire is the network lock. Protocol folds into the genesis. Anyone "
            "who solves “old wallets should stop mining” with the wrong constant has "
            "built a chain reset."
        ),
        CondPageBreak(210),
        Chapter("03", "Cryptography, without theatre"),
        P(
            "The suite is small, on purpose. One curve, one encoding, one hash, one "
            "commitment scheme, one range proof, one signature scheme. More primitives "
            "would be more seams — and seams were v4."
        ),
        suite,
        Spacer(1, 10),
        P("One-sided stealth outputs", "h2"),
        P(
            "The sender draws an ephemeral r and publishes Ke = r·G. Receivers compute "
            "t = H(“shared” ‖ a·Ke), the same value the sender has as H(“shared” ‖ r·A). "
            "Blinding, one-time offset and payload key sit on that. No recipient identifier "
            "appears on chain."
        ),
        P(
            "Spending Ko requires b_spend + o. The sender knows o, not b_spend — they "
            "cannot reclaim what they sent. That is why Nightfall can do one-sided "
            "payments, and why cut-through does not work today: the input signature "
            "under Ko is the authorisation a block must not throw away when it merges."
        ),
        P("What a signature binds — and what it does not", "h2"),
        P(
            "Kernel message = H(feature, fee, reward, lock_height, excess). "
            "Input message = H(commit). Output message = H(features, commit, Ke, Ko, "
            "proof, payload). None of the three binds a transaction body. That is "
            "deliberate: aggregation dissolves transactions, a body-bound signature "
            "would stop verifying afterwards. It is also the hard limit on “pre-signed” "
            "contracts. Anyone who expects lock_height inside an input signature has "
            "carried Bitcoin intuition onto the wrong chain."
        ),
        Callout(
            "v4 taught the project's most expensive lesson: a check that always passes "
            "is not a check. The excess signature over H exists because equality of "
            "sums was a tautology."
        ),
        CondPageBreak(210),
        Chapter("04", "Privacy, honestly"),
        P(
            "Nightfall hides amounts and addresses. It does not erase the transaction "
            "graph, and it does not hide the first hop if Tor is down and the next peer "
            "is an old node that fluffs immediately. The honest line is in PRIVACY.md. "
            "It is not softened here."
        ),
        priv,
        Spacer(1, 10),
        P("What is already in your hand", "h2"),
        P(
            "The view key nfview1 sees every amount and every memo, moves no coin. A "
            "receipt opens exactly one output (amount + blinding), signed with the spend "
            "key of the nf1 address. An auditor verifies the opening and the signature "
            "without seeing the rest of the wallet. That is selective disclosure as a "
            "product, not as a roadmap bullet."
        ),
        P(
            "On the network: Dandelion-class stem/fluff on the existing Tx message, Tor "
            "default. A new transaction goes to exactly one random peer. If that hop "
            "fails, it is gone — 0.8.2 lifts this by resubmitting unconfirmed sends after "
            "every sync. Stem is origin obfuscation, not a delivery promise."
        ),
        Callout(
            "The anonymity set is the other transactions in the same block, plus however "
            "many people actually use the chain. An empty block with one payment is a "
            "CoinJoin of one. Hiding that would be marketing. Nightfall does not.",
            "warn",
        ),
        CondPageBreak(210),
        Chapter("05", "Money and mining"),
        P(
            "Era-0 reward 6 NIGHT, halving every 7.5 million blocks, hard cap 90 million, "
            "no tail. Because each halving truncates "
            "(<font face='SFMono' color='#9b83ff'>reward &gt;&gt; halvings</font>), "
            "the curve ends at 89,999,999.25 — 0.75 below the ceiling. The cap is an "
            "upper bound, not a target."
        ),
        P(
            "PoW is Nighthash-v2: Argon2id, 32 MiB, one iteration, one lane. Memory-hardness "
            "is the point — purpose-built hardware needs that much fast RAM per parallel "
            "unit. A mainnet hash costs the same to verify as to mine, about 11 ms. That "
            "is why a node keeps a local validation marker and does not re-hash its own "
            "chain on every start."
        ),
        P(
            "Retarget every block, LWMA-1 over 90 blocks, target 15 s, solve time clamped "
            "to [−5T, +6T], movement ±2×, floor 2000. A slow block lowers difficulty by "
            "at most about 10 %. There is no time-based emergency brake. Difficulty ≈ "
            "expected hashes per block; network hashrate ≈ difficulty / 15."
        ),
        Callout(
            "A monetary policy that reacts to the market price is not a monetary policy. "
            "Emission, floor and the fee rule stay while the chain is young. Changes only "
            "after a security analysis — not after a chart."
        ),
        P(
            "Fees are public in the kernel so the burn is auditable. While subsidy flows "
            "they burn in full; the miner receives only the subsidy. The wallet therefore "
            "offers no priority tiers: the mempool does not order by fee, and “Priority” "
            "would be a speed-up that nothing delivers."
        ),
        CondPageBreak(210),
        Chapter("06", "What you can hold today"),
        P(
            "Core is a full node, a miner and a wallet in one window. nightfalld is the "
            "same node without a GUI. The web wallet at nightfallcoin.org/wallet/ is a "
            "light client: it keeps no chain file, it scans stealth outputs through the "
            "seeds' light API. APK and IPA are off the site — an unsigned sideload is a "
            "worse first fifteen minutes than a page that simply opens."
        ),
        *bullets(
            [
                "24 words (BIP-39, bijective to the 32-byte seed). Hex underneath. Without the phrase the coins do not come back.",
                "Address book local. Receipts and view key in the wallet.",
                "Prune: UTXO + headers + last 500 bodies. Seeds that serve IBD or the light API stay archives.",
                "blocks.bin since 0.8.0 — 72 % less disk than JSON, no consensus change. P2P stays newline JSON.",
                "One process per datadir. The lock is the operating system (flock / LockFileEx), not a PID file.",
                "Chain archives for newcomers, weekly, without taking the previous one off the network.",
            ]
        ),
        Spacer(1, 6),
        P(
            "Light API (port 17888, public via the seed): status, scan_feed, submit_tx, "
            "get_utxo_root, banner, get_headers. get_blocks stays out — full bodies with "
            "range proofs are an amplifier for anyone who asks twice. The allow-list sits "
            "in the node, not only in the website worker. A light client trusts the node "
            "for display; spending is only the spend key on the device."
        ),
        Callout(
            "Phones and browsers trust the node for display, not for spend. The seed in "
            "the web wallet's localStorage is the seed. Wipe the site and you wipe the "
            "address book; the 24 words do not bring it back."
        ),
        CondPageBreak(210),
        Chapter("07", "Atomic swap — and the wart"),
        P(
            "NIGHT ↔ BTC, wallet-to-wallet, copy-paste packets, no operator, no mailbox. "
            "Bitcoin side: P2WSH 2-of-2, ECDSA adaptor, CSV abort tree (lock / redeem / "
            "cancel / refund / punish). NIGHT side: a shared stealth address, spend "
            "s<sub>a</sub> + s<sub>b</sub> + offset. The adaptor lives on Bitcoin only."
        ),
        P(
            "Mainnet will not start a swap. Testnet and devnet will. The cross-curve DLEQ "
            "uses our own Ristretto leaf inside a reviewed combinator crate. Nobody "
            "outside this project has signed that leaf. The gate stays until it is "
            "opened on purpose."
        ),
        P(
            "v0.1 wanted to remove the known wart of XMR↔BTC: a time-locked NIGHT refund, "
            "because Nightfall has lock_height and Monero does not. The section was marked "
            "as the place to attack. It was broken the same day. The input signature binds "
            "H(commit) and nothing else. A co-signed input signature is not a pre-signed "
            "transaction — it pairs with any balancing kernel. The sender of the lock "
            "output knows the blinding factor and can sign a kernel with lock_height = 0. "
            "Then redeem the BTC. Both sides, no race."
        ),
        Callout(
            "In Mimblewimble, “pre-sign” is not an available tool. Bitcoin intuition does "
            "not transfer. v0.1 sits as SWAP-SPEC-v0.1-withdrawn.md so the mistake stays "
            "readable. v0.3 is the known protocol, wart accepted and visible in the UI: "
            "there is no NIGHT refund. If the other side cancels and never refunds, the "
            "locked NIGHT is stuck forever.",
            "warn",
        ),
        P(
            "Alice must not redeem close to H<sub>1</sub>. The Bitcoin node would accept "
            "the redeem — nothing on Bitcoin knows about H<sub>1</sub> when it looks at "
            "the redeem. The wallet refuses it. That is policy, not chain, and live run 4 "
            "against bitcoind showed exactly that."
        ),
        CondPageBreak(210),
        Chapter("08", "What is missing, and why we say so"),
        P(
            "The spec lists gaps on purpose. A whitepaper that omits them would be an ad. "
            "The points below are not a disgrace — they are where the next work has to "
            "start, if it starts."
        ),
        gaps,
        Spacer(1, 10),
        P(
            "What 0.8.x and 0.9.x already closed, so the gap list is not last week's "
            "history: binary disk format, prune, introducer mode, checkpoint 25,000, "
            "mempool expiry, resubmit of unconfirmed sends, datadir lock, incremental "
            "supply sum, chain archives, get_headers for the chain page. Consensus, "
            "genesis, wire: unchanged."
        ),
        CondPageBreak(210),
        Chapter("09", "Non-negotiable"),
        P(
            "The future is not defined by a feature list. Every new function has to "
            "survive these sentences. They live in the manifesto, in the spec, and in "
            "the places this project has already been burned."
        ),
        *bullets(
            [
                "<b>Privacy is the default.</b> No transparent base path that becomes the norm out of convenience.",
                "<b>Proof, not trust.</b> Supply, ownership, chosen statements — cryptographic, not policy.",
                "<b>Disclosure is granular.</b> One payment, one window, one lower bound. Never the whole history as the price of a statement.",
                "<b>No privileged money.</b> No premine, no treasury coins, no after-the-fact special emission, no admin freeze.",
                "<b>Consensus minimalism.</b> As little logic in consensus as possible. Application off-chain or in its own proofs.",
                "<b>The author does not self-certify.</b> Whoever writes a cryptographic construction does not alone grant its security release.",
                "<b>Interop without custodians.</b> Atomic exchange, no bridge, no wrapped NIGHT as the base.",
                "<b>Say the limit.</b> Never “100 % anonymous”. Never invent a price. Never report peer versions as fact — that is self-description; what is enforced is only WIRE_VERSION.",
                "<b>Code before narrative.</b> The repo is the truth. The thread is marketing.",
                "<b>Exit is sacred.</b> If the chain is captured, the community must be able to fork without asking.",
            ]
        ),
        Spacer(1, 8),
        Callout(
            "Anti-MEV is not on this list, because on a 15-second CPU chain with no VM "
            "and burning fees it is not a business model. A principle that describes a "
            "problem the construction does not have is a costume."
        ),
        CondPageBreak(210),
        Chapter("10", "Horizon"),
        P(
            "Five years is long enough to shape the architecture and short enough that "
            "every date in a table becomes a lie. So no calendar roadmap. Capability-based: "
            "a stage is done when the criterion holds — not when a quarter ends. "
            "Consensus-relevant cryptography may be late if the evidence is not enough."
        ),
        future,
        Spacer(1, 10),
        P("The actual research question", "h2"),
        P(
            "One-sided payments need a per-input signature under the one-time key. "
            "Cut-through would delete exactly that signature. Both at once do not exist "
            "in today's construction. Anyone who wants the graph gone has to build a "
            "spend authorisation that does not publish a per-input signature and still "
            "stops the sender reclaiming. That is research, not a sprint. Until then the "
            "sentence in the spec stands: the graph is obscured, not erased."
        ),
        P(
            "What Nightfall does not import from Monero: TxTangle as middleware, ring "
            "signatures, an escrow marketplace with a moderator, I2P as a second default. "
            "The block already aggregates without coordination. A session in which wallets "
            "agree to join introduces an operator through the back door — exactly the "
            "class the swap deliberately does not have."
        ),
        Callout(
            "Spec before code took one afternoon to break, on the swap, what would have "
            "cost months and other people's money. The same order applies to every "
            "consensus-adjacent idea on this horizon."
        ),
        CondPageBreak(210),
        Chapter("11", "What we will not build"),
        *bullets(
            [
                "No general smart-contract VM on layer 1.",
                "No bridge, no wrapped NIGHT, no transparent unshield.",
                "No exchange, no ticker, no official pair, no date for one.",
                "No premine, no team allocation, no VC round, no treasury.",
                "No admin key that can freeze, seize or rewrite.",
                "No mandatory identity, no compliance mode that turns privacy into an opt-in.",
                "No short-term tokenomics change to set a price — there is no official price.",
                "No second seed on the same Vultr account, no raising MAX_PEERS as a substitute for capacity.",
                "No claim of “untraceable” / “100 % anonymous” / “protocol-scale anonymity set”.",
            ]
        ),
        Spacer(1, 8),
        Callout(
            "These limits are not a weakness. They protect the core function: reliable "
            "private money whose supply every node proves.",
            "ok",
        ),
        Spacer(1, 14),
        P("Where Nightfall sits, next to the others", "h2"),
        pos,
        CondPageBreak(210),
        Chapter("12", "Close"),
        P(
            "The core idea is no longer a concept. The chain is young enough that the "
            "architecture can still be shaped, and old enough that it has already died "
            "once on a tautology. The question that matters is therefore not which ten "
            "features come next. It is: which properties should still be non-negotiable "
            "in five years."
        ),
        P(
            "Today: harden consensus, outside review, network privacy, wallet reliability. "
            "Then: scale state and proofs without sacrificing the invariant. In parallel: "
            "open the swap only if the leaf is reviewed. Long term, and only if the maths "
            "allows it: a spend authorisation that no longer makes one-sided payments and "
            "graph erasure enemies."
        ),
        Spacer(1, 8),
        Callout(
            "When the world turns every payment into a permanent public confession, "
            "NIGHTFALLCOIN is the right to remain silent — enforced by cryptography, "
            "not by policy, PR, or the kindness of validators.",
            "quote",
        ),
        Spacer(1, 10),
        P(
            "Not “Nightfall becomes Monero 2.0”. A generation of its own: private "
            "settlement infrastructure, fewer leftovers, an explicit supply proof, "
            "block-wide aggregation, a narrow consensus, and a research roadmap that "
            "says what it cannot do."
        ),
        P(
            "If this direction holds, what appears is not a privacy feature. It is a "
            "digital cash system whose default question is no longer “Why do you want "
            "to hide this payment?” but “Which information would you like to prove, "
            "voluntarily?”."
        ),
        Spacer(1, 16),
        Rule(),
        Spacer(1, 8),
        P("Born when the lights go out.", "quote"),
        P("NIGHTFALLCOIN · nightfallcoin.org · Instinctes/nightfall", "caption"),
        CondPageBreak(210),
        P("APPENDIX", "kicker"),
        P("Invariants for every future consensus version", "h2"),
        *bullets(
            [
                "Supply can never rise outside the emission schedule.",
                "Every spend needs cryptographic authorisation under the one-time key in the UTXO set.",
                "Every output carries a valid, network-bound range proof.",
                "No output is spent twice.",
                "Σ UTXO − Σ kernel_excess = (minted − burned)·G holds after every block. The equation is not a substitute for soundness proofs.",
                "WIRE_VERSION changes the handshake, not the genesis. PROTOCOL_VERSION changes the genesis.",
                "Disclosure mechanisms do not reveal spending authority.",
                "Cross-chain adapters do not introduce implicit custody.",
                "Reorgs deeper than 500 are refused. A wallet state must not stay inconsistent after a reorg.",
                "Network messages must not trigger unbounded allocations.",
            ]
        ),
        Spacer(1, 8),
        P("Sources and scope", "h2"),
        P(
            "As-is: docs/SPEC.md, docs/PRIVACY.md, docs/DECISIONS.md, docs/HISTORY.md, "
            "docs/RESET.md, docs/MAINNET.md, MANIFESTO.md, crates/nightfall-types, "
            "nightfall-crypto, nightfall-ledger, nightfall-consensus. Swap: docs/SWAP.md, "
            "SWAP-SPEC-DRAFT.md v0.3, SWAP-SPEC-v0.1-withdrawn.md, SWAP-LOSS.md, "
            "SWAP-ATTACKS.md. Internal review: docs/AUDIT-2026-08-16.md. v4: "
            "docs/AUDIT-2026-08-12.md."
        ),
        P(
            "Everything in chapter 10 called a track, a horizon or research is a proposal. "
            "It is not a claim that the function lives in the repository. Software this "
            "paper describes: 0.9.2, September 2026, the same chain as on genesis day."
        ),
        Spacer(1, 16),
        P(
            "Fair launch. 0 % premine. 0 % team. 0 % VC. Public genesis. Mine or receive.",
            "small",
        ),
    ]


def build(lang: str) -> str:
    global STY, LANG
    LANG = lang
    _flatten_logo()
    _register_fonts()
    STY = S()

    subject = "Protokoll und Horizont" if lang == "de" else "Protocol and horizon"
    out = os.path.join(ROOT, f"NIGHTFALLCOIN-Whitepaper-{lang.upper()}.pdf")
    tmp = tempfile.mkdtemp(prefix="nf-wp-")
    cover_pdf = os.path.join(tmp, "cover.pdf")
    body_pdf = os.path.join(tmp, "body.pdf")

    cover = pdfcanvas.Canvas(cover_pdf, pagesize=A4)
    cover.setTitle("NIGHTFALLCOIN — Whitepaper")
    cover.setAuthor("NIGHTFALLCOIN")
    cover.setSubject(subject)
    draw_cover(cover, lang)
    cover.save()

    doc = BaseDocTemplate(
        body_pdf,
        pagesize=A4,
        title="NIGHTFALLCOIN — Whitepaper",
        author="NIGHTFALLCOIN",
        subject=subject,
        creator="NIGHTFALLCOIN",
        leftMargin=ML,
        rightMargin=MR,
        topMargin=MT,
        bottomMargin=MB,
    )
    body_frame = Frame(ML, MB, CONTENT_W, PAGE_H - MT - MB, showBoundary=0)
    doc.addPageTemplates(
        [PageTemplate(id="body", frames=[body_frame], onPage=draw_body)]
    )
    doc.build(story_de() if lang == "de" else story_en())

    from pypdf import PdfWriter

    writer = PdfWriter()
    writer.append(cover_pdf)
    writer.append(body_pdf)
    writer.add_metadata(
        {
            "/Title": "NIGHTFALLCOIN — Whitepaper",
            "/Author": "NIGHTFALLCOIN",
            "/Subject": subject,
            "/Creator": "NIGHTFALLCOIN",
        }
    )
    with open(out, "wb") as fh:
        writer.write(fh)
    return out


if __name__ == "__main__":
    for lang in ("de", "en"):
        print(build(lang))
