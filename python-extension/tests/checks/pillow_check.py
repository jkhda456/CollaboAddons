"""Pillow, its compiled parts: _imaging (drawing, resampling, filters, conversions, the codecs:
JPEG/libjpeg-turbo, JPEG 2000/openjpeg, TIFF/libtiff, PNG/zlib), _webp, _imagingft (FreeType),
_imagingcms (LittleCMS), _imagingmath, _imagingmorph.

Prints pixel hashes where the result is exact (drawing, lossless codecs and their round trips,
fixed-point resampling, lookup tables) and rounded statistics where Pillow computes in floating
point (kernel filters, blurs, blending, transforms, colour management): there a compiler may fuse
a multiply and an add (GCC does, for Pillow's aarch64 wheels), and a few pixels come out one off. Only Pillow's basic text layout: the guest
has no raqm (tests/test_guest.py checks that). tests/reference.py runs it with Pillow's own wheel.
"""
import hashlib
import io

import numpy as np
from PIL import (Image, ImageChops, ImageCms, ImageDraw, ImageEnhance, ImageFilter, ImageFont, ImageMath,
                 ImageMorph, ImageOps, ImageSequence, ImageStat)


def h(im):
    return hashlib.sha256(im.tobytes()).hexdigest()[:16]


def stats(im):
    s = ImageStat.Stat(im)
    return [round(m, 1) for m in s.mean], s.extrema


print("version", Image.__version__)

# a test picture: shapes, lines, a gradient, text
im = Image.new("RGB", (160, 120), (250, 250, 245))
d = ImageDraw.Draw(im)
for x in range(160):
    d.line([(x, 0), (x, 20)], fill=(x, 255 - x, (x * 3) % 256))
d.rectangle([10, 30, 70, 90], fill=(200, 40, 40), outline=(0, 0, 120), width=3)
d.ellipse([60, 40, 150, 110], fill=(40, 160, 70, 128))
d.polygon([(20, 110), (80, 95), (50, 60)], fill=(30, 30, 200))
d.arc([90, 25, 150, 85], 30, 300, fill="black", width=2)
d.rounded_rectangle([100, 90, 155, 115], radius=6, outline="purple")
font = ImageFont.load_default(size=13).font_variant(layout_engine=ImageFont.Layout.BASIC)
d.text((8, 4), "Pillow 12 on wasm32", font=font, fill=(0, 0, 0))
print("picture", h(im), d.textbbox((8, 4), "Pillow 12 on wasm32", font=font), font.getlength("wasm32"))
mask = font.getmask("ABC xyz", mode="L")
print("glyphs", mask.size, hashlib.sha256(bytes(mask)).hexdigest()[:16])

# codecs: lossless round trips must give the pixels back; lossy ones their decoder's output
for fmt, kw in [("PNG", {}), ("PNG", {"optimize": True}), ("TIFF", {}), ("TIFF", {"compression": "tiff_lzw"}),
                ("TIFF", {"compression": "tiff_adobe_deflate"}), ("TIFF", {"compression": "packbits"}),
                ("WEBP", {"lossless": True}), ("JPEG2000", {}), ("BMP", {}), ("TGA", {}), ("PPM", {}), ("QOI", {})]:
    buf = io.BytesIO()
    im.save(buf, fmt, **kw)
    buf.seek(0)
    back = Image.open(buf)
    print("lossless", fmt, kw, back.format, back.size, h(back.convert("RGB")) == h(im))
for fmt, kw in [("JPEG", {"quality": 85}), ("JPEG", {"quality": 95, "subsampling": 0, "progressive": True}),
                ("WEBP", {"quality": 75}), ("TIFF", {"compression": "jpeg"}), ("JPEG2000", {"irreversible": True, "quality_layers": [20]})]:
    buf = io.BytesIO()
    im.save(buf, fmt, **kw)
    buf.seek(0)
    back = Image.open(buf).convert("RGB")
    diff = np.abs(np.asarray(back, dtype=np.int64) - np.asarray(im, dtype=np.int64)).mean()
    print("lossy", fmt, kw, back.size, round(float(diff), 1))
buf = io.BytesIO()
im.save(buf, "JPEG", quality=85)
print("jpeg bytes", len(buf.getvalue()), hashlib.sha256(buf.getvalue()).hexdigest()[:16])
buf.seek(0)
print("jpeg decode", h(Image.open(buf)))

# palettes, GIF (Pillow's own LZW), animation
p = im.quantize(colors=16, method=Image.Quantize.MEDIANCUT)
print("quantize", h(p), len(p.getpalette()) // 3)
frames = [im.rotate(a, resample=Image.NEAREST) for a in (0, 90, 180)]
for fmt in ("GIF", "WEBP", "PNG"):
    buf = io.BytesIO()
    frames[0].save(buf, fmt, save_all=True, append_images=frames[1:], duration=80, loop=0,
                   **({"lossless": True} if fmt == "WEBP" else {}))
    buf.seek(0)
    a = Image.open(buf)
    print("animation", fmt, a.n_frames, [h(f.convert("RGB"))[:8] for f in ImageSequence.Iterator(a)][:1])

# geometry, resampling, filters (fixed-point for 8-bit images)
for name, flt in (("nearest", Image.NEAREST), ("bilinear", Image.BILINEAR), ("bicubic", Image.BICUBIC),
                  ("lanczos", Image.LANCZOS), ("box", Image.BOX), ("hamming", Image.HAMMING)):
    print("resize", name, h(im.resize((97, 61), flt)), h(im.reduce(3)) if name == "box" else "")
print("transpose", h(im.transpose(Image.Transpose.ROTATE_90)), h(im.transpose(Image.Transpose.FLIP_LEFT_RIGHT)))
print("crop paste", h(im.crop((5, 5, 105, 75))), h(ImageOps.expand(im, 4, fill="red")))
for name in ("BLUR", "CONTOUR", "DETAIL", "EDGE_ENHANCE", "EMBOSS", "FIND_EDGES", "SHARPEN", "SMOOTH"):
    print("filter", name, stats(im.filter(getattr(ImageFilter, name))))
print("blur", stats(im.filter(ImageFilter.GaussianBlur(3))), stats(im.filter(ImageFilter.BoxBlur(2))),
      stats(im.filter(ImageFilter.UnsharpMask(2, 150, 3))), h(im.filter(ImageFilter.MedianFilter(3))))
print("rotate", stats(im.rotate(17, resample=Image.BICUBIC, expand=True)))
print("affine", stats(im.transform((80, 60), Image.AFFINE, (2, 0.3, 0, -0.2, 2, 10), resample=Image.BILINEAR)))

# modes and conversions
for mode in ("L", "LA", "1", "P", "RGBA", "CMYK", "YCbCr", "HSV", "I", "F", "I;16"):
    c = im.convert(mode)
    print("convert", mode, c.mode, h(c))
print("split merge", [h(b)[:8] for b in im.split()], h(Image.merge("RGB", im.split()[::-1])))
print("ops", h(ImageOps.autocontrast(im, cutoff=2)), h(ImageOps.equalize(im)), h(ImageOps.posterize(im, 3)),
      h(ImageOps.solarize(im, 128)), h(ImageOps.invert(im)), h(ImageOps.grayscale(im)))
print("chops", h(ImageChops.difference(im, frames[2])), h(ImageChops.multiply(im, frames[2])), ImageChops.difference(im, im).getbbox())
print("enhance", stats(ImageEnhance.Contrast(im).enhance(1.7)), stats(ImageEnhance.Color(im).enhance(0.3)))
print("histogram", sum(im.histogram()), im.getbbox(), im.getcolors(maxcolors=100000)[:1] if im.getcolors(maxcolors=100000) else None)

# numpy
arr = np.asarray(im)
print("numpy", arr.shape, arr.dtype.str, int(arr.astype(np.int64).sum()), h(Image.fromarray(arr[::-1])))
print("from float", h(Image.fromarray((np.linspace(0, 1, 64 * 32).reshape(32, 64) * 255).astype(np.uint8))))

# colour management (LittleCMS)
srgb = ImageCms.createProfile("sRGB")
lab = ImageCms.createProfile("LAB")
to_lab = ImageCms.buildTransform(srgb, lab, "RGB", "LAB")
labim = ImageCms.applyTransform(im, to_lab)
print("cms", labim.mode, stats(labim)[0], ImageCms.getProfileDescription(srgb).strip())
print("cms profile", len(ImageCms.ImageCmsProfile(srgb).tobytes()) > 0)

# ImageMath, ImageMorph
L = im.convert("L")
print("math", h(ImageMath.lambda_eval(lambda a: (a["x"] * 2 - 100) & 255, x=L).convert("L")))
binary = L.point(lambda v: 255 if v > 128 else 0)
for op in ("erosion4", "dilation8", "edge"):
    print("morph", op, ImageMorph.MorphOp(op_name=op).apply(binary)[0])
print("done")
