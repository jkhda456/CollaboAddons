# ImageMagick in this sandbox — a guide for agents

`magick` here is ImageMagick 7.1.2-34 (Q16 HDRI, OpenMP), built for collaboCore's WebAssembly
guest; for the same input and command it writes the same pixels as ImageMagick built natively.
Use it to edit, convert, compose and inspect images from the command line.

| | |
|---|---|
| Read and write | PNG, JPEG, GIF (animated), WebP (animated), AVIF, TIFF, JPEG 2000, BMP, ICO, PSD, TGA, PPM/PGM/PBM, QOI, MIFF, and many older formats (`magick -list format`) |
| HEIC (iPhone photos) | **read** (`magick IMG_0001.HEIC photo.jpg`), not written: no HEVC encoder — write JPEG or AVIF |
| SVG | read with ImageMagick's own renderer (shapes, paths, gradients, text, inline styles; SVG filters and masks may come out incomplete: look at the result), written as MVG/SVG |
| PDF, PostScript | **written** (`magick a.png b.png out.pdf`), **not read**: there is no Ghostscript |
| Not here | JPEG XL, camera RAW, video (no ffmpeg), URLs (`magick https://…` needs curl: download the file first) |
| Fonts | DejaVu Sans / Serif / Sans Mono (Latin, Greek, Cyrillic), NanumGothic and NanumMyeongjo (Korean) |
| Tools | `magick`, and `convert`, `identify`, `mogrify`, `composite`, `compare`, `montage` as links to it |

## Working with images

1. **Look before you edit**, cheaply: `magick identify -format '%m %wx%h %[colorspace] %[channels] %b\n' in.jpg`
   (format, size, colorspace, channels such as `srgba` for alpha, file size). Add `-ping` for
   size only without decoding. Avoid `identify -verbose` on large photos: it computes statistics
   (20 s on a 12-megapixel photo).
2. **Write a new file**; keep the original unless asked to replace it. `magick mogrify` edits files
   **in place** — use `magick mogrify -path outdir …` to write elsewhere.
3. **Look at the result** if you can view images (the Read tool shows PNG, JPEG, GIF and WebP):
   make a small preview first, `magick out.png -resize '1024x1024>' preview.png`, or put
   before and after side by side, `magick before.jpg after.jpg -resize x400 +append compare.jpg`.
   Without viewing, check numbers: size and format with `identify`, how much changed with
   `magick compare -metric RMSE a.png b.png null:` (prints `absolute (normalized)` on stderr;
   0 = the same).
4. Tell the user the output path and what changed (size, format, file size).

## Command syntax (ImageMagick 7)

```
magick [settings] input... [settings] operators... output
```

- Read first, then operate, then the output file last. **Settings** (`-gravity`, `-font`,
  `-pointsize`, `-fill`, `-stroke`, `-background`, `-quality`, `-fuzz`, `-density`) stay in force
  for the operators after them; **operators** (`-resize`, `-crop`, `-annotate`, `-composite`)
  act at once. `-gravity center` before `-annotate`/`-extent`/`-composite`, not after.
- Settings that describe an image as it is read — `-delay`, `-label`, `-size`, `-page`, input
  `-density` — go **before** the files they are for: `magick -delay 50 a.png b.png anim.gif`
  (after the files they do nothing; `-set delay 50` changes images already read).
- **Geometry**: `800x600` fits inside (keeps the aspect ratio), `800x600^` fills (then `-extent`
  to crop), `800x600!` exact (distorts), `800x600>` only shrinks, `800x600<` only enlarges,
  `50%`, `800x` (width only), `x600` (height only), `+X+Y` offsets. **Quote** geometries in the
  shell: `>` and `<` are redirections, `!` and `^` can be special: `-resize '800x600>'`.
- **Parentheses** group a side image: `\( logo.png -resize 30% \)`; `+clone` copies the last
  image, `+swap` swaps the last two, `+delete` drops one, `-delete 0`.
- `-crop` keeps the old canvas offset: always follow it with `+repage`.
- Output format from the name (`out.webp`) or a prefix: `PNG8:out.png`, `PNG32:out.png`,
  `jpg:-` (standard output), `-` alone reads standard input.
- Frames: `anim.gif[0]` is the first frame, `in.tif[1-3]` some pages; `-coalesce` before editing
  an animation, `-layers Optimize` after.
- Print instead of writing: `-format '%w %h\n' info:`, with `%[fx:expression]`,
  `%[pixel:p{x,y}]`, `%[mean]`. `-define` passes coder options (`-define jpeg:extent=200kb`).
- `convert` (ImageMagick 6 syntax) works but use `magick`.

## Recipes

Sizes and shapes:

```sh
magick in.jpg -resize 1200x1200 out.jpg                      # fit in 1200×1200
magick in.jpg -resize '1200x1200>' out.jpg                   # ... only if larger
magick in.jpg -thumbnail 300x300 -quality 85 thumb.jpg       # small, metadata stripped
magick in.jpg -resize '400x400^' -gravity center -extent 400x400 square.jpg   # fill and crop
magick in.png -background white -gravity center -extent 1200x630 padded.png   # pad to a size
magick in.jpg -crop 800x600+100+50 +repage part.jpg          # a region: WxH+X+Y
magick in.png -trim +repage trimmed.png                      # cut uniform borders
magick in.png -fuzz 10% -trim +repage trimmed.png            # ... nearly uniform ones
magick in.jpg -auto-orient upright.jpg                       # apply the camera's EXIF rotation
magick in.jpg -rotate 90 rotated.jpg                         # also -flip, -flop
magick in.png -bordercolor '#333' -border 10 bordered.png
```

Formats and file size:

```sh
magick in.png out.jpg                                        # convert (by extension)
magick in.png -background white -flatten -quality 85 out.jpg # alpha onto white (JPEG has no alpha)
magick in.jpg -quality 80 -strip smaller.jpg                 # recompress, drop metadata
magick in.jpg -define jpeg:extent=200kb capped.jpg           # at most 200 KB
magick in.png -quality 80 out.webp                           # lossy WebP
magick in.png -define webp:lossless=true out.webp            # lossless WebP
magick in.png -quality 50 out.avif                           # AVIF (add -define heic:speed=8 to write faster)
magick photo.heic -quality 90 photo.jpg                      # an iPhone photo to JPEG
magick -define heic:primary-image=true photo.heic main.jpg   # only its main image (see below)
magick in.png -colors 64 PNG8:small.png                      # palette PNG
magick logo.png -define icon:auto-resize=256,64,48,32,16 favicon.ico
magick page1.png page2.png -density 150 doc.pdf              # images into a PDF
magick -delay 50 frame1.png frame2.png frame3.png -loop 0 anim.gif   # delay in 1/100 s
magick anim.gif -coalesce frame-%02d.png                     # an animation's frames
magick in.svg out.png                                        # SVG (add -density 300 for more pixels)
```

Color and tone:

```sh
magick in.jpg -colorspace Gray gray.jpg
magick in.jpg -modulate 110,130,100 vivid.jpg                # brightness, saturation, hue (%)
magick in.jpg -brightness-contrast 10x15 brighter.jpg
magick in.jpg -auto-level -sharpen 0x1 fixed.jpg
magick in.jpg -sepia-tone 80% sepia.jpg
magick in.png -fuzz 8% -transparent white transparent.png    # white background away
magick in.jpg -blur 0x3 blurred.jpg                          # or -gaussian-blur 0x3
```

Text (Korean and Latin):

```sh
magick in.jpg -gravity south -font NanumGothic-Bold -pointsize 48 -fill white \
  -stroke black -strokewidth 2 -annotate +0+30 '2026 봄 신제품' captioned.jpg
magick -background '#1e3a5f' -fill white -font NanumGothic-Bold -pointsize 64 \
  label:'공지사항 Notice' banner.png                       # the canvas fits the text
magick -size 600x -background white -font NanumGothic -pointsize 28 \
  caption:'긴 문장은 너비에 맞춰 줄바꿈됩니다. Long text wraps.' box.png
magick -size 800x200 xc:white -font DejaVu-Sans -pointsize 40 -fill '#333' \
  -gravity center -annotate +0+0 'Hello, 안녕하세요' hello.png
```

Composing:

```sh
magick photo.jpg \( logo.png -resize 20% \) -gravity southeast -geometry +20+20 -composite marked.jpg
magick composite -dissolve 30 -gravity center logo.png photo.jpg faint.jpg   # see-through
magick a.jpg b.jpg -resize x400 +append side-by-side.jpg     # -append: top to bottom
magick montage -label '%f' -font DejaVu-Sans *.jpg -tile 4x -geometry 200x200+4+4 sheet.jpg
magick in.png \( +clone -background black -shadow 60x8+10+10 \) +swap -background none \
  -layers merge +repage shadow.png                           # drop shadow
magick -size 400x300 xc:white -fill '#90caf9' -stroke '#1565c0' -strokewidth 3 \
  -draw 'roundrectangle 20,20 380,280 25,25' -draw 'circle 200,150 200,100' shapes.png
```

Inspecting and comparing:

```sh
magick identify -format '%f: %wx%h %m %[colorspace] %Q\n' *.jpg
magick in.jpg -format '%[pixel:p{10,10}]\n' info:          # a pixel's color
magick in.jpg -resize 64x64 -colors 5 -format '%c' histogram:info:-   # dominant colors
magick in.jpg -format '%[fx:mean]\n' info:                   # mean brightness 0..1
magick compare -metric SSIM a.png b.png null:                # similarity (1 = the same)
magick compare a.png b.png diff.png                          # differences in red
magick identify -format '%[EXIF:*]' photo.jpg                # camera metadata (nothing if none)
```

Many files:

```sh
mkdir -p small && magick mogrify -path small -resize 50% -quality 85 *.jpg
for f in *.png; do magick "$f" -quality 85 "${f%.png}.webp"; done
```

## Fonts

`magick -list font | grep Font:` lists the names; `-font NAME` or a font file's path
(`-font ./MyFont.ttf`, for any TrueType/OpenType font the user gives you).

| Name | |
|---|---|
| `NanumGothic`, `NanumGothic-Bold` | Korean sans-serif (with Latin) |
| `NanumMyeongjo`, `NanumMyeongjo-Bold` | Korean serif |
| `DejaVu-Sans`, `DejaVu-Sans-Bold`, `-Oblique`, `-Condensed`, `-ExtraLight` | Latin, Greek, Cyrillic |
| `DejaVu-Serif`, `DejaVu-Serif-Bold`, `-Italic` | |
| `DejaVu-Sans-Mono`, `DejaVu-Sans-Mono-Bold` | monospace |
| `Helvetica`, `Arial`, `Sans`; `Times-Roman`, `Serif`; `Courier`, `Monospace` | DejaVu Sans, Serif, Sans Mono under the usual names |

Text without `-font` is DejaVu Sans. Characters a font lacks (Hangul in DejaVu Sans) are drawn
with NanumGothic (then DejaVu Sans), so mixed Korean and English text works with any of these
fonts; `-define type:fallback-font=NanumMyeongjo` picks another fallback, `none` turns it off.
**No font here has Chinese characters (Hanja), Japanese kana or emoji**: they come out blank. For
those, ask for a font file (e.g. Noto Sans CJK) and pass it with `-font`.

## Good to know

- **Speed** (2 CPUs, a 12-megapixel photo): resize, thumbnail, JPEG 1–1.5 s; WebP 3 s; AVIF
  4 s (1.3 s with `-define heic:speed=8`); rotate 4 s; `-gaussian-blur 0x3` 12 s;
  `identify -verbose` 20 s. Work on a smaller copy while trying settings. ImageMagick uses
  every CPU of the sandbox (`-limit thread 1` for one).
- **Several images in one file** (a HEIC with more than its main image, a multi-page TIFF, a GIF)
  give one output per image, numbered: `out-0.jpg`, `out-1.jpg`. Take one with `in.heic[0]`
  (quote it: `"in.heic[0]"`) or, for HEIC, `-define heic:primary-image=true`; check with
  `magick identify in.heic` (one line per image).
- **Memory**: pixels take 16 bytes each (Q16 HDRI, RGBA): a 12-megapixel image is ~200 MB.
  Up to 2 GiB of pixels stay in memory, more go to a cache in `/tmp` (slower). `-limit memory`
  and `-limit disk` change that (`magick -list resource`).
- **Bit depth**: images made from scratch (`label:`, `gradient:`, `xc:`) are 16-bit; add
  `-depth 8` for smaller PNGs.
- **Random effects** (`+noise`, `-spread`, `plasma:`) differ each run unless `-seed N` is given.
- `%[EXIF:Model]` on an image without that tag warns `unknown image property`: list what is
  there with `%[EXIF:*]` first.
- **Errors**: `magick` exits non-zero and says why on stderr: `unable to open image` (path),
  `no decode delegate for this image format` (unsupported or not an image),
  `unable to read font` (name: see Fonts). Warnings (exit 0) still deserve a look.
- A write to a pipe that closes early (`| head`) reports `unable to write file '-'`: write to a
  file and read that instead.
- Configuration: `/etc/ImageMagick-7` (policy.xml: limits and the fallback font; type.xml: the
  font names; sRGB.icm for `-profile`). The full manual is at https://imagemagick.org/script/command-line-options.php.
