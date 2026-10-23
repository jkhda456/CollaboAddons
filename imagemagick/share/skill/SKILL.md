---
name: imagemagick
description: Edit, convert and inspect images with ImageMagick (the `magick` command in this sandbox) - resize, crop, rotate, change format or quality (PNG, JPEG, WebP, AVIF, GIF, TIFF, ICO, PDF output; HEIC photos read), compress, add Korean or English text, watermark, combine, make thumbnails, contact sheets and GIFs, adjust colors, compare images, read sizes and metadata. Use when the user asks to change, create, convert or check an image file.
---

# Images with ImageMagick

The sandbox has `magick` (ImageMagick 7.1.2, also `convert`, `identify`, `mogrify`, `composite`,
`compare`, `montage`), fonts for Korean (NanumGothic, NanumMyeongjo) and Latin (DejaVu Sans,
Serif, Sans Mono). Full guide with recipes: `/usr/share/doc/imagemagick/AGENTS.md` — read it
before your first edit.

## Steps

1. Inspect: `magick identify -format '%m %wx%h %[colorspace] %[channels] %b\n' FILE`
   (not `-verbose` on big photos: slow).
2. Edit into a **new file**: `magick in.jpg [settings] [operators] out.jpg`. Keep the original
   unless told to replace it (`magick mogrify` overwrites; use `-path DIR`).
3. Check the result: if you can view images, look at it (a preview:
   `magick out.png -resize '1024x1024>' preview.png`); otherwise `identify` it and, against the
   input, `magick compare -metric RMSE in.png out.png null:`.
4. Report the output path, its size and format.

## Syntax that matters

- Order: input → settings → operators → output. Put `-gravity`, `-font`, `-pointsize`, `-fill`,
  `-background`, `-quality` **before** the operator they affect; `-delay`, `-label`, `-size`
  **before** the input files.
- Quote geometries: `'800x600>'` (shrink only), `'400x400^'` (fill), `'300x300!'` (exact);
  `50%`, `800x` (width), `x600` (height). Escape parentheses: `\( logo.png -resize 30% \)`.
- After `-crop` always `+repage`. JPEG has no alpha: `-background white -flatten` first.
- A file with several images (HEIC, multi-page TIFF, GIF) gives `out-0.jpg`, `out-1.jpg`, …:
  take one with `"in.heic[0]"` or `-define heic:primary-image=true`.
- Text: `-font NanumGothic` (Korean), `-font DejaVu-Sans`; Hangul in a Latin font falls back to
  NanumGothic. No Chinese characters, Japanese kana or emoji in these fonts (blank): use a
  font file (`-font path.ttf`) if the user provides one.
- Not supported here: reading PDF/PS (no Ghostscript), writing HEIC (read it, write JPEG/AVIF),
  JPEG XL, RAW, video, URLs.

## Common commands

```sh
magick in.jpg -resize '1600x1600>' -quality 85 out.jpg
magick in.jpg -resize '400x400^' -gravity center -extent 400x400 square.jpg
magick in.png -crop 800x600+100+50 +repage part.png
magick in.jpg -auto-orient -strip out.jpg
magick in.png -quality 80 out.webp
magick in.jpg -gravity south -font NanumGothic-Bold -pointsize 48 -fill white -stroke black -strokewidth 2 -annotate +0+30 '텍스트 Text' out.jpg
magick photo.jpg \( logo.png -resize 20% \) -gravity southeast -geometry +20+20 -composite out.jpg
magick a.jpg b.jpg -resize x400 +append side.jpg
magick -delay 50 f1.png f2.png f3.png -loop 0 anim.gif
magick montage -label '%f' -font DejaVu-Sans *.jpg -tile 4x -geometry 200x200+4+4 sheet.jpg
mkdir -p out && magick mogrify -path out -resize 50% *.jpg
```
