"""Gera assets/monitorbt.ico (logo do app). Uso: python assets/logo.py"""
import math
from pathlib import Path
from PIL import Image, ImageDraw

S = 1024  # desenha grande e reduz (antialiasing)


def draw(fill_frac=0.75):
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)

    # Fundo: quadrado arredondado grafite com leve gradiente vertical.
    bg = Image.new("RGBA", (S, S))
    gd = ImageDraw.Draw(bg)
    for y in range(S):
        t = y / S
        c = (int(42 - 14 * t), int(46 - 14 * t), int(56 - 14 * t), 255)
        gd.line([(0, y), (S, y)], fill=c)
    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, S - 1, S - 1], radius=int(S * 0.22), fill=255)
    img.paste(bg, (0, 0), mask)

    # Anel de bateria: trilha + arco verde (começa às 12h, sentido horário).
    m = S * 0.14
    w = int(S * 0.085)
    box = [m, m, S - m, S - m]
    d.arc(box, 0, 360, fill=(255, 255, 255, 40), width=w)
    start = -90
    end = start + 360 * fill_frac
    d.arc(box, start, end, fill=(61, 214, 102, 255), width=w)
    # Pontas arredondadas do arco.
    r = (S - 2 * m) / 2 - w / 2
    cx = cy = S / 2
    for a in (start, end):
        x = cx + r * math.cos(math.radians(a))
        y = cy + r * math.sin(math.radians(a))
        d.ellipse([x - w / 2, y - w / 2, x + w / 2, y + w / 2], fill=(61, 214, 102, 255))

    # Símbolo do Bluetooth.
    def p(x, y):  # coordenadas 0..100 dentro de uma caixa central
        k = S * 0.0046
        return (cx + (x - 50) * k, cy + (y - 50) * k)

    pts = [p(30, 33), p(70, 67), p(50, 85), p(50, 15), p(70, 33), p(30, 67)]
    sw = int(S * 0.05)
    d.line(pts, fill=(255, 255, 255, 255), width=sw, joint="curve")
    for q in (pts[0], pts[-1]):
        d.ellipse([q[0] - sw / 2, q[1] - sw / 2, q[0] + sw / 2, q[1] + sw / 2], fill=(255, 255, 255, 255))
    return img


if __name__ == "__main__":
    out = Path(__file__).with_name("monitorbt.ico")
    big = draw()
    sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256]
    big.resize((256, 256), Image.LANCZOS).save(out, sizes=[(s, s) for s in sizes])
    big.resize((512, 512), Image.LANCZOS).save(out.with_name("logo-preview.png"))
    print("ok", out)
