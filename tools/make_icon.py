"""Renders the Ada app icon (Turkish red/white, Dynamic-Island mascot) at any size."""
import math
import sys
from PIL import Image, ImageDraw, ImageFilter

RED_TOP = (240, 50, 60)
RED_BOTTOM = (196, 10, 22)
WHITE = (255, 255, 255)


def squircle_mask(s, radius_frac=0.235):
    m = Image.new("L", (s, s), 0)
    ImageDraw.Draw(m).rounded_rectangle([0, 0, s - 1, s - 1], radius=int(s * radius_frac), fill=255)
    return m


def vertical_gradient(s, top, bottom):
    g = Image.new("RGB", (1, s))
    for y in range(s):
        t = y / (s - 1)
        g.putpixel((0, y), tuple(int(a + (b - a) * t) for a, b in zip(top, bottom)))
    return g.resize((s, s))


def render(s, variant="red"):
    S = s * 4  # supersample
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))

    # Background squircle.
    if variant == "red":
        bg = vertical_gradient(S, RED_TOP, RED_BOTTOM).convert("RGBA")
    else:
        bg = vertical_gradient(S, (250, 250, 252), (232, 233, 238)).convert("RGBA")
    img.paste(bg, (0, 0), squircle_mask(S))

    # Soft top sheen.
    sheen = Image.new("L", (S, S), 0)
    ImageDraw.Draw(sheen).ellipse([-S * 0.3, -S * 0.75, S * 1.3, S * 0.42], fill=40 if variant == "red" else 25)
    sheen = sheen.filter(ImageFilter.GaussianBlur(S * 0.06))
    img = Image.composite(Image.new("RGBA", (S, S), WHITE + (255,)), img, Image.fromarray_from_mask(sheen) if False else sheen.point(lambda v: v)) if False else img
    white_layer = Image.new("RGBA", (S, S), WHITE + (0,))
    white_layer.putalpha(Image.composite(sheen, Image.new("L", (S, S), 0), squircle_mask(S)))
    img = Image.alpha_composite(img, white_layer)

    # Island (the notch character): glossy black pill.
    pw, ph = S * 0.70, S * 0.40
    px, py = (S - pw) / 2, S * 0.33
    box = [px, py, px + pw, py + ph]
    r = ph * 0.48

    # Drop shadow.
    sh = Image.new("L", (S, S), 0)
    ImageDraw.Draw(sh).rounded_rectangle([box[0], box[1] + S * 0.035, box[2], box[3] + S * 0.035], radius=r, fill=110)
    sh = sh.filter(ImageFilter.GaussianBlur(S * 0.03))
    shadow = Image.new("RGBA", (S, S), (60, 0, 5, 0))
    shadow.putalpha(sh)
    img = Image.alpha_composite(img, shadow)

    pill_mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(pill_mask).rounded_rectangle(box, radius=r, fill=255)

    pill = Image.new("RGBA", (S, S), (14, 14, 18, 255))
    # Bottom glow inside the pill: Turkish red rising from the bottom edge.
    gmask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(gmask).ellipse([box[0] - pw * 0.1, box[3] - ph * 0.42, box[2] + pw * 0.1, box[3] + ph * 0.55], fill=235)
    gmask = gmask.filter(ImageFilter.GaussianBlur(S * 0.045))
    glow = Image.new("RGBA", (S, S), (255, 45, 60, 0))
    glow.putalpha(gmask)
    pill = Image.alpha_composite(pill, glow)
    rim = Image.new("L", (S, S), 0)
    ImageDraw.Draw(rim).ellipse([box[0] + pw * 0.12, box[3] - ph * 0.16, box[2] - pw * 0.12, box[3] + ph * 0.2], fill=150)
    rim = rim.filter(ImageFilter.GaussianBlur(S * 0.02))
    rim_layer = Image.new("RGBA", (S, S), (255, 190, 195, 0))
    rim_layer.putalpha(rim)
    pill = Image.alpha_composite(pill, rim_layer)
    # Glossy top highlight.
    hl = Image.new("L", (S, S), 0)
    ImageDraw.Draw(hl).rounded_rectangle([box[0] + pw * 0.08, box[1] + ph * 0.06, box[2] - pw * 0.08, box[1] + ph * 0.38],
                                         radius=r * 0.7, fill=38)
    hl = hl.filter(ImageFilter.GaussianBlur(S * 0.012))
    hlayer = Image.new("RGBA", (S, S), WHITE + (0,))
    hlayer.putalpha(hl)
    pill = Image.alpha_composite(pill, hlayer)
    pill.putalpha(pill_mask)
    img = Image.alpha_composite(img, pill)

    # Eyes: happy "^ ^" arcs with a soft glow.
    cx, cy = S / 2, py + ph * 0.50
    dx = pw * 0.17
    stroke = S * 0.052
    eye_r = S * 0.075
    eyes = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ed = ImageDraw.Draw(eyes)
    for sx in (-1, 1):
        ex = cx + sx * dx
        bb = [ex - eye_r, cy - eye_r * 0.55, ex + eye_r, cy - eye_r * 0.55 + 2 * eye_r]
        ed.arc(bb, start=205, end=335, fill=WHITE + (255,), width=int(stroke))
        # round caps
        for ang in (205, 335):
            a = math.radians(ang)
            ccx = (bb[0] + bb[2]) / 2 + eye_r * math.cos(a) - 0
            ccy = (bb[1] + bb[3]) / 2 + eye_r * math.sin(a)
            # arc is drawn inside the bbox with width inward; approximate cap center
            rr = eye_r - stroke / 2
            ccx = (bb[0] + bb[2]) / 2 + rr * math.cos(a)
            ccy = (bb[1] + bb[3]) / 2 + rr * math.sin(a)
            ed.ellipse([ccx - stroke / 2, ccy - stroke / 2, ccx + stroke / 2, ccy + stroke / 2], fill=WHITE + (255,))
    halo = eyes.split()[3].filter(ImageFilter.GaussianBlur(S * 0.025))
    halo_layer = Image.new("RGBA", (S, S), (255, 210, 210, 0))
    halo_layer.putalpha(halo.point(lambda v: int(v * 0.8)))
    img = Image.alpha_composite(img, halo_layer)
    img = Image.alpha_composite(img, eyes)

    # Small white crescent + star (flag motif) top-right on the background.
    if variant == "red" and s > 24:
        mx, my, mr = S * 0.765, S * 0.185, S * 0.075
        moon = Image.new("L", (S, S), 0)
        md = ImageDraw.Draw(moon)
        md.ellipse([mx - mr, my - mr, mx + mr, my + mr], fill=255)
        md.ellipse([mx - mr * 0.55, my - mr * 0.8, mx + mr * 1.05, my + mr * 0.8], fill=0)
        star = []
        sr, sx0, sy0 = mr * 0.42, mx + mr * 0.62, my
        for k in range(10):
            ang = -math.pi / 2 + k * math.pi / 5 + math.pi / 10 * 0  # point up, then rotate
            rad = sr if k % 2 == 0 else sr * 0.42
            ang = math.radians(-90 + 36 * k + 18 * 0) + math.radians(-90 + 90)  # first point faces the crescent opening (left)
            star.append((sx0 + rad * math.cos(ang + math.pi), sy0 + rad * math.sin(ang + math.pi)))
        md.polygon(star, fill=255)
        ml = Image.new("RGBA", (S, S), WHITE + (0,))
        ml.putalpha(moon)
        img = Image.alpha_composite(img, ml)

    return img.resize((s, s), Image.LANCZOS)


if __name__ == "__main__":
    out = sys.argv[1] if len(sys.argv) > 1 else "."
    for v in ("red", "light"):
        render(256, v).save(f"{out}/ada_{v}_256.png")
    print("ok")
