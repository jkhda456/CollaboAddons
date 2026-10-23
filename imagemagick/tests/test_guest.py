#!/usr/bin/env python3
"""The imagemagick add-on in the guest: what it installs, and what agents do with it — the same
commands run in the guest and by the native reference (the same sources built for this machine,
build/native), their output files and printed text compared:

  same bytes    the files are equal
  same pixels   the files differ only where the format records the time (PNG's date and time
                chunks, PDF's dates): the pixels' signature (%#) and the image's attributes agree
  rounding      the pixels differ by a rounding of the C library's math (musl's in the guest,
                glibc's natively): RMSE below 1e-5 of the range, listed by name

Random numbers with -seed are the same from run to run, but not the native ones: the seed is
hashed as an unsigned long, 4 bytes on wasm32 (as in any 32-bit build), 8 on this machine: the
guest's seeded noise is checked against a second run of its own.

Then what only the guest can show: the fonts and the fallback for characters a font lacks,
external programs started without fork (a delegate), write errors, pipes and descriptors, the
pixel cache on disk, OpenMP's threads, and speed. Run by tests/run.sh; see common.py.
"""
import os
import re
import shutil
import subprocess
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from common import (A, B, CONFIG, MAGICK, Timer, check, guest, native, present, skip,  # noqa: E402
                    summary, workdir)

PERL_T = os.path.join(B, "src", "PerlMagick", "t")
MARK = "=== case "
END = "=== status "

SVG = """<svg xmlns="http://www.w3.org/2000/svg" width="320" height="200" viewBox="0 0 320 200">
  <defs><linearGradient id="g" x1="0" x2="1"><stop offset="0" stop-color="#1e88e5"/>
    <stop offset="1" stop-color="#43a047"/></linearGradient></defs>
  <rect x="10" y="10" width="300" height="180" rx="20" fill="url(#g)"/>
  <circle cx="80" cy="100" r="40" fill="#fff" fill-opacity="0.6" stroke="#333" stroke-width="3"/>
  <path d="M150 150 L200 50 L250 150 Z" fill="#ffca28"/>
  <text x="160" y="185" font-family="DejaVu Sans" font-size="18" text-anchor="middle" fill="#fff">SVG 그림</text>
</svg>
"""

MVG = """viewbox 0 0 200 120
fill '#eee' rectangle 0,0 200,120
stroke '#d32f2f' stroke-width 4 fill none bezier 10,100 60,10 140,10 190,100
fill '#1976d2' stroke none polygon 20,110 60,70 100,110
fill '#388e3c' ellipse 150,80 30,20 0,360
"""

# The inputs, made by the reference before the cases run (deterministic: plasma and noise seeded).
INPUTS = r"""
magick -size 1200x800 -seed 7 plasma:green-blue \( -size 1200x800 radial-gradient:white-none \) \
  -compose over -composite -fill '#c33' -draw 'circle 600,400 600,250' -fill '#ffd54f' \
  -draw 'rectangle 100,100 300,300' -quality 92 photo.jpg
magick logo: logo.png
magick -size 400x300 xc:none -fill '#2e7d32' -draw 'roundrectangle 20,20 380,280 30,30' \
  -fill '#ffffff80' -draw 'circle 200,150 200,60' alpha.png
magick -delay 20 -size 120x80 xc:'#e53935' xc:'#43a047' xc:'#1e88e5' xc:'#fdd835' -loop 0 anim.gif
magick photo.jpg -resize 400x300 -orient RightTop -set comment 'collabo test' orient.jpg
magick -size 300x200 gradient:'#ffffff-#000000' gray.png
magick rose: -resize 400% rose.png
magick photo.jpg -resize 50% -colorspace CMYK cmyk.jpg
"""

# (name, command): run in out/<name>, with $IN the inputs' folder; what it writes there is
# compared, and so is what it prints.
CASES = [
    # resizing
    ("resize-percent", "magick $IN/photo.jpg -resize 50% o.jpg"),
    ("resize-box", "magick $IN/photo.jpg -resize 640x480 o.png"),
    ("resize-exact", "magick $IN/photo.jpg -resize '300x300!' o.png"),
    ("resize-shrink-only", "magick $IN/logo.png -resize '2000x2000>' o.png && magick $IN/logo.png -resize '100x100<' o2.png"),
    ("resize-fill-crop", "magick $IN/photo.jpg -resize '400x400^' -gravity center -extent 400x400 o.jpg"),
    ("thumbnail", "magick $IN/photo.jpg -thumbnail 160x160 -quality 80 o.jpg"),
    ("sample-scale", "magick $IN/rose.png -sample 25% o1.png && magick $IN/rose.png -scale 200% o2.png"),
    ("resize-filters", "for f in Lanczos Mitchell Point Catrom; do magick $IN/rose.png -filter $f -resize 173% o-$f.png; done"),
    ("liquid-free-distort", "magick $IN/photo.jpg -resize 25% -distort Perspective '0,0 30,10 299,0 270,20 0,199 0,199 299,199 299,199' o.png"),
    # geometry
    ("crop", "magick $IN/photo.jpg -crop 400x300+100+50 +repage o.png"),
    ("crop-tiles", "magick $IN/rose.png -crop 2x2@ +repage tile-%d.png"),
    ("trim", "magick $IN/logo.png -trim +repage o.png && magick $IN/logo.png -fuzz 10% -trim -format '%wx%h%O' info:"),
    ("rotate", "magick $IN/photo.jpg -resize 25% -background white -rotate 30 o.png && magick $IN/logo.png -rotate 90 o90.png"),
    ("flip-flop-transpose", "magick $IN/rose.png -flip o1.png && magick $IN/rose.png -flop o2.png && magick $IN/rose.png -transpose o3.png"),
    ("auto-orient", "magick $IN/orient.jpg -format '%[orientation] %wx%h\\n' -write info: -auto-orient -format '%[orientation] %wx%h\\n' -write info: o.jpg"),
    ("border-frame-extent", "magick $IN/rose.png -bordercolor '#333' -border 10 o1.png && magick $IN/rose.png -mattecolor gray -frame 8x8+3+3 o2.png && magick $IN/rose.png -background '#09f' -gravity center -extent 400x300 o3.png"),
    ("shave-chop-splice", "magick $IN/rose.png -shave 10x5 o1.png && magick $IN/rose.png -gravity east -chop 20x0 o2.png && magick $IN/rose.png -background red -splice 10x10 o3.png"),
    ("distort-srt-barrel", "magick $IN/rose.png -virtual-pixel transparent -distort SRT '1.2 25' o1.png && magick $IN/rose.png -distort Barrel '0.1 0 0 0.9' o2.png && magick $IN/rose.png +distort Arc 120 o3.png"),
    # color and tone
    ("modulate", "magick $IN/photo.jpg -resize 25% -modulate 110,130,90 o.png"),
    ("brightness-contrast", "magick $IN/photo.jpg -resize 25% -brightness-contrast 10x20 o.png"),
    ("levels", "magick $IN/photo.jpg -resize 25% -level 10%,90%,1.2 o1.png && magick $IN/photo.jpg -resize 25% -auto-level o2.png && magick $IN/photo.jpg -resize 25% -normalize o3.png && magick $IN/photo.jpg -resize 25% -gamma 1.6 o4.png"),
    ("grayscale-sepia-negate", "magick $IN/photo.jpg -resize 25% -colorspace Gray o1.png && magick $IN/photo.jpg -resize 25% -sepia-tone 80% o2.png && magick $IN/photo.jpg -resize 25% -negate o3.png"),
    ("threshold-posterize", "magick $IN/gray.png -threshold 50% o1.png && magick $IN/photo.jpg -resize 25% -posterize 4 o2.png && magick $IN/gray.png -ordered-dither o4x4 o3.png"),
    ("quantize", "magick $IN/photo.jpg -resize 25% -colors 16 o1.png && magick $IN/photo.jpg -resize 25% +dither -colors 8 PNG8:o2.png && magick $IN/photo.jpg -resize 25% -remap netscape: o3.gif"),
    ("tint-colorize", "magick $IN/photo.jpg -resize 25% -fill '#0066ff' -tint 60 o1.png && magick $IN/photo.jpg -resize 25% -fill orange -colorize 30% o2.png"),
    ("transparent", "magick $IN/logo.png -fuzz 8% -transparent white o.png && magick o.png -format '%[opaque] %[channels]\\n' info:"),
    ("alpha", "magick $IN/alpha.png -background '#ffeb3b' -alpha remove -alpha off o1.png && magick $IN/alpha.png -background white -flatten o2.jpg && magick $IN/alpha.png -alpha extract o3.png"),
    ("colorspaces", "for c in Lab HSL YCbCr CMYK LCHab XYZ; do magick $IN/rose.png -colorspace $c -separate +append o-$c.png; done"),
    ("icc-profile", "magick $IN/cmyk.jpg -format '%[colorspace]\\n' -write info: -colorspace sRGB o1.jpg && magick $IN/rose.png -profile $IN/sRGB.icm o2.png && magick o2.png -format '%[profiles] %[icc:description]\\n' info: && magick o2.png -profile $IN/sRGB.icm -colorspace Gray o3.png"),
    ("clut-hald", "magick $IN/photo.jpg -resize 25% \\( -size 1x256 gradient:'#203060-#ffe0a0' \\) -clut o1.png && magick hald:8 -modulate 100,150 hald.png && magick $IN/photo.jpg -resize 25% hald.png -hald-clut o2.png"),
    # filters and effects
    ("blur-sharpen", "magick $IN/photo.jpg -resize 25% -blur 0x2 o1.png && magick $IN/photo.jpg -resize 25% -gaussian-blur 0x1.5 o2.png && magick $IN/photo.jpg -resize 25% -sharpen 0x1 o3.png && magick $IN/photo.jpg -resize 25% -unsharp 0x1+1.2+0.05 o4.png"),
    ("motion-radial-blur", "magick $IN/rose.png -motion-blur 0x8+30 o1.png && magick $IN/rose.png -rotational-blur 10 o2.png"),
    ("edge-emboss", "magick $IN/rose.png -edge 2 o1.png && magick $IN/rose.png -emboss 1 o2.png && magick $IN/rose.png -canny 0x1+10%+30% o3.png"),
    ("artistic", "magick $IN/rose.png -charcoal 2 o1.png && magick $IN/rose.png -paint 3 o2.png && magick $IN/rose.png -swirl 90 o3.png && magick $IN/rose.png -implode 0.5 o4.png && magick $IN/rose.png -wave 5x40 o5.png && magick $IN/rose.png -kuwahara 3 o6.png"),
    ("vignette-shadow", "magick $IN/photo.jpg -resize 25% -background black -vignette 0x40 o1.png && magick $IN/alpha.png \\( +clone -background black -shadow 60x6+8+8 \\) +swap -background none -layers merge +repage o2.png"),
    ("median-despeckle", "magick $IN/rose.png -statistic Median 3x3 o1.png && magick $IN/rose.png -despeckle o2.png"),
    ("morphology", "magick $IN/gray.png -threshold 50% -morphology Dilate Disk:3 o1.png && magick $IN/rose.png -morphology Convolve Gaussian:0x2 o2.png && magick $IN/rose.png -morphology EdgeOut Diamond o3.png"),
    ("fx-evaluate", "magick $IN/rose.png -fx 'u*0.5 + 0.25*sin(i/5)' o1.png && magick $IN/rose.png -evaluate Pow 0.7 o2.png && magick $IN/rose.png -function Polynomial 2,-1,0.1 o3.png"),
    ("connected-components", "magick $IN/gray.png -threshold 50% -define connected-components:verbose=true -connected-components 4 o.png"),
    # compositing
    ("watermark", "magick $IN/photo.jpg -resize 50% $IN/alpha.png -resize 30% -gravity southeast -geometry +10+10 -composite o1.jpg && magick composite -dissolve 40 -gravity center $IN/alpha.png $IN/photo.jpg o2.jpg"),
    ("compose-modes", "for m in Multiply Screen Overlay Difference Darken Lighten; do magick $IN/rose.png $IN/gray.png -resize 280x184! -compose $m -composite o-$m.png; done"),
    ("append", "magick $IN/rose.png $IN/logo.png -resize 200x150 -background white -gravity center +append o1.png && magick $IN/rose.png $IN/rose.png -append o2.png && magick $IN/rose.png -background white -splice 0x10 \\( $IN/rose.png \\) -smush 5 o3.png"),
    ("rounded-corners", "magick $IN/photo.jpg -resize 25% \\( +clone -alpha extract -draw 'fill black polygon 0,0 0,20 20,0 fill white circle 20,20 20,0' \\( +clone -flip \\) -compose Multiply -composite \\( +clone -flop \\) -compose Multiply -composite \\) -alpha off -compose CopyOpacity -composite o.png"),
    ("mask-composite", "magick $IN/photo.jpg -resize 25% $IN/rose.png -resize 300x200! $IN/gray.png -resize 300x200! -composite o.png"),
    ("layers-flatten", "magick -page +0+0 $IN/rose.png -page +100+50 $IN/alpha.png -background white -layers flatten o1.png && magick -page +0+0 $IN/rose.png -page +100+50 $IN/alpha.png -background none -layers mosaic o2.png"),
    # text and drawing
    ("annotate-en", "magick -size 480x160 xc:white -font DejaVu-Sans -pointsize 32 -fill '#1a237e' -annotate +20+60 'Quarterly Report' -font DejaVu-Serif-Bold -pointsize 22 -annotate +20+110 'Revenue +12% (Q3)' o.png"),
    ("annotate-ko", "magick -size 480x160 xc:white -font NanumGothic -pointsize 32 -fill black -annotate +20+60 '분기 보고서' -font NanumMyeongjo-Bold -pointsize 26 -annotate +20+120 '매출 12% 증가' o.png"),
    ("annotate-fallback", "magick -size 520x120 xc:white -font DejaVu-Sans -pointsize 30 -annotate +10+50 'Hello 안녕하세요' -font DejaVu-Sans-Bold -annotate +10+100 'Bold 굵게 Ωμέγα' o.png"),
    ("annotate-gravity-stroke", "magick $IN/photo.jpg -resize 50% -gravity south -font DejaVu-Sans-Bold -pointsize 36 -stroke black -strokewidth 2 -fill white -annotate +0+20 '캡션 Caption' o.jpg"),
    ("label-caption", "magick -background lightblue -fill navy -font NanumGothic-Bold -pointsize 40 label:'라벨 Label' o1.png && magick -size 300x -background white -font DejaVu-Sans -pointsize 18 caption:'긴 캡션 텍스트는 상자 너비에 맞춰 줄바꿈됩니다. Long captions wrap to the width.' o2.png && magick -size 200x100 -gravity center caption:'Fit me to the box' o3.png"),
    ("text-metrics", "magick -font DejaVu-Sans -pointsize 24 label:'Size Wy' -format '%wx%h\\n' info: && magick -debug annotate xc: -font NanumGothic -pointsize 24 -annotate 0 '한글 텍스트' null: 2>&1 | grep -o 'Metrics: .*' | head -1"),
    ("default-font-aliases", "magick -pointsize 20 label:'Default font' o1.png && magick -font Helvetica-Bold -pointsize 20 label:'Helvetica' o2.png && magick -font Times-Roman -pointsize 20 label:'Times' o3.png && magick -font Courier -pointsize 20 label:'Courier' o4.png && magick -family 'DejaVu Sans' -weight Bold -stretch Condensed -pointsize 20 label:'Family' o5.png"),
    ("font-file", "magick -font $IN/DejaVuSansMono.ttf -pointsize 20 label:'mono 0O1lI' o.png"),
    ("draw-shapes", "magick -size 300x200 xc:white -stroke '#222' -strokewidth 3 -fill '#90caf9' -draw 'rectangle 10,10 140,90' -fill '#ef9a9a' -draw 'circle 220,60 220,20' -fill none -draw 'line 10,180 290,120' -fill '#a5d6a7' -draw 'polygon 160,190 200,110 240,190' -fill '#fff59d' -draw 'roundrectangle 20,110 140,190 15,15' o.png"),
    ("draw-mvg", "magick -size 200x120 mvg:$IN/sample.mvg o1.png && magick -size 200x120 xc:white -draw @$IN/sample.mvg o2.png"),
    ("gradients-patterns", "magick -size 200x100 gradient:'#ff0000-#0000ff' o1.png && magick -size 200x200 radial-gradient:white-'#004' o2.png && magick -size 200x100 pattern:checkerboard o3.png && magick -size 200x100 -define gradient:angle=45 gradient:yellow-purple o4.png"),
    ("svg-render", "magick $IN/text.svg o1.png && magick -density 150 $IN/text.svg o2.png && magick -background none $IN/text.svg -resize 50% o3.png"),
    ("svg-write", "magick $IN/text.svg o.mvg && magick msvg:$IN/text.svg o2.png"),
    # formats
    ("png-variants", "magick $IN/alpha.png PNG8:o8.png && magick $IN/photo.jpg -resize 25% PNG24:o24.png && magick $IN/alpha.png PNG32:o32.png && magick $IN/gray.png -depth 16 o16.png && magick $IN/photo.jpg -resize 25% -define png:compression-level=9 -strip o9.png && magick identify -format '%f %m %z %[channels]\\n' o8.png o24.png o32.png o16.png"),
    ("jpeg-options", "magick $IN/photo.jpg -quality 60 -sampling-factor 4:2:0 -strip -interlace Plane o1.jpg && magick $IN/photo.jpg -quality 95 -define jpeg:dct-method=float o2.jpg && magick $IN/photo.jpg -define jpeg:extent=50kb o3.jpg && magick identify -format '%f %Q %[interlace] %[jpeg:sampling-factor]\\n' o1.jpg o2.jpg o3.jpg && test $(wc -c < o3.jpg) -le 51200"),
    ("webp", "magick $IN/photo.jpg -resize 50% -quality 80 o1.webp && magick $IN/alpha.png -define webp:lossless=true o2.webp && magick o2.webp o2.png && magick identify -format '%f %m %[channels]\\n' o1.webp o2.webp"),
    ("gif-anim", "magick $IN/anim.gif -coalesce -resize 50% -layers Optimize o.gif && magick identify -format '%s %T %wx%h %D\\n' o.gif && magick '$IN/anim.gif[2]' o2.png && magick $IN/anim.gif -coalesce +append o3.png"),
    ("gif-make", "magick -delay 50 $IN/rose.png \\( $IN/rose.png -negate \\) \\( $IN/rose.png -flop \\) -loop 0 o.gif && magick -delay 10 $IN/rose.png $IN/gray.png -resize 280x184! -morph 3 o2.gif && magick identify -format '%s %T\\n' o.gif o2.gif"),
    ("webp-anim", "magick $IN/anim.gif -quality 70 o.webp && magick identify -format '%s %T %wx%h\\n' o.webp"),
    ("tiff", "magick $IN/photo.jpg -resize 25% -compress LZW o1.tif && magick $IN/photo.jpg -resize 25% -compress Zip o2.tif && magick $IN/photo.jpg -resize 25% -compress JPEG o3.tif && magick $IN/rose.png $IN/gray.png o4.tif && magick identify -format '%f %C %s\\n' o1.tif o2.tif o3.tif o4.tif"),
    ("heic-avif-read", "magick identify -format '%m %wx%h %z %[colorspace]\\n' $IN/example.heic $IN/example.avif && magick $IN/example.heic -resize 50% o1.png && magick \"$IN/example.heic[1]\" -resize 25% o2.jpg && magick $IN/example.avif o3.png"),
    ("avif-write", "magick $IN/rose.png -quality 60 o1.avif && magick o1.avif o1.png && magick $IN/photo.jpg -resize 25% -quality 50 -define heic:speed=8 o2.avif && magick $IN/alpha.png o3.avif && magick identify -format '%f %m %wx%h %z %[channels]\\n' o1.avif o2.avif o3.avif"),
    ("jpeg2000", "magick $IN/photo.jpg -resize 25% -quality 40 o.jp2 && magick o.jp2 o.png && magick identify -format '%m %wx%h\\n' o.jp2"),
    ("ico-bmp-tga-ppm", "magick $IN/logo.png -define icon:auto-resize=64,32,16 o.ico && magick identify -format '%wx%h ' o.ico && magick $IN/rose.png o.bmp && magick $IN/rose.png o.tga && magick $IN/rose.png o.ppm && magick $IN/rose.png -compress none o.pgm && magick $IN/rose.png o.qoi"),
    ("pdf-write", "magick $IN/rose.png $IN/logo.png -resize 300x200 -density 72 o.pdf && magick $IN/rose.png -compress jpeg -quality 70 o2.pdf"),
    ("psd-miff-misc", "magick $IN/alpha.png o.psd && magick o.psd o-psd.png && magick $IN/rose.png o.miff && magick o.miff o-miff.png && magick $IN/rose.png -depth 8 o.rgb && magick -size 280x184 -depth 8 rgb:o.rgb o-rgb.png"),
    ("base64-inline", "magick $IN/rose.png -resize 20% -define png:exclude-chunks=date,time inline:o.txt && magick inline:o.txt o.png"),
    ("txt-sparse-color", "magick $IN/rose.png -resize 5% txt:o.txt && head -5 o.txt && magick $IN/rose.png -resize 5% sparse-color:o2.txt && cut -c1-80 o2.txt"),
    # information
    ("identify-format", "magick identify -format '%f %m %wx%h %z-bit %[colorspace] %[type] %Q %[channels] %k\\n' $IN/photo.jpg $IN/logo.png $IN/alpha.png $IN/anim.gif"),
    ("identify-ping", "magick identify -ping -format '%wx%h %b\\n' $IN/photo.jpg"),
    ("identify-verbose", "magick identify -verbose $IN/rose.png | grep -v -e 'date:' -e 'Filename' -e 'Elapsed' -e 'User time' -e 'Pixels per second' -e 'Version' -e 'Artifacts' -e 'filename' -e 'File size' -e 'Number pixels'"),
    ("exif-metadata", "magick identify -format '%[orientation] %c %[EXIF:Orientation]\\n' $IN/orient.jpg && magick $IN/orient.jpg -strip o.jpg && magick identify -format '%[orientation] (%c)\\n' o.jpg"),
    ("statistics", "magick $IN/photo.jpg -format '%[fx:mean] %[fx:standard_deviation] %[fx:minima] %[fx:maxima] %[mean] %[kurtosis]\\n' info: && magick $IN/rose.png -format '%[pixel:p{10,10}] %[hex:p{10,10}] %[fx:mean.r]\\n' info: && magick $IN/rose.png -colorspace Gray -format '%[entropy]\\n' info:"),
    ("histogram", "magick $IN/photo.jpg -resize 10% -colors 6 -format '%c' histogram:info:h.txt && sort -n h.txt && magick $IN/rose.png -format '%k\\n' info:"),
    ("compare-metrics", "magick $IN/rose.png -blur 0x1 b.png && for m in AE RMSE PSNR MAE NCC SSIM DSSIM PHASH; do printf '%s ' $m; magick compare -metric $m $IN/rose.png b.png null: 2>&1; echo; done; magick compare -fuzz 5% $IN/rose.png b.png diff.png; echo \" status $?\"; magick compare -metric AE $IN/rose.png $IN/rose.png null: 2>&1; echo \" status $?\""),
    ("compare-subimage", "magick $IN/rose.png -crop 40x30+100+60 +repage part.png && magick compare -metric RMSE -subimage-search $IN/rose.png part.png null: 2>&1; echo"),
    ("montage", "magick montage $IN/rose.png $IN/logo.png $IN/alpha.png $IN/gray.png -font DejaVu-Sans -pointsize 12 -label '%f' -tile 2x -geometry 160x120+6+6 -background '#f5f5f5' o.png && magick montage -font NanumGothic -title '몽타주 제목' $IN/rose.png $IN/gray.png -geometry +4+4 o2.png"),
    ("mogrify-batch", "cp $IN/rose.png a.png && cp $IN/gray.png b.png && magick mogrify -resize 50% -quality 90 a.png b.png && mkdir conv && magick mogrify -path conv -format jpg a.png b.png && ls conv && magick identify -format '%f %wx%h\\n' a.png b.png conv/a.jpg"),
    ("convert-legacy", "convert $IN/rose.png -resize 50% o1.png && identify o1.png | cut -d' ' -f1-4 && composite -gravity center $IN/alpha.png $IN/rose.png o2.png"),
    ("fx-info", "magick xc: -format '%[fx:4*atan(1)] %[fx:hypot(3,4)] %[fx:floor(2.7)] %[fx:3^2]\\n' info: && magick -size 1x1 xc:'#ff8000' -format '%[pixel:u] %[fx:int(255*u.g)]\\n' info:"),
    ("sequence-ops", "magick $IN/rose.png -write mpr:r +delete mpr:r \\( mpr:r -negate \\) \\( mpr:r -flip \\) -evaluate-sequence mean o1.png && magick $IN/anim.gif -evaluate-sequence max o2.png && magick $IN/rose.png \\( +clone -flop \\) -compose difference -composite -format '%[fx:maxima]\\n' info:"),
    ("print-features", "magick xc: -print '%[magick] ok\\n' null: && magick -list format | grep -E ' (PNG|JPEG|GIF|WEBP|TIFF|JP2|SVG|PDF|ICO|BMP)  ' | sed 's/  */ /g'"),
]

GUEST_ONLY = r"""
check() { if eval "$2" >/dev/null 2>&1; then echo "ok $1"; else echo "not ok $1"; fi; }
# installed
for f in /usr/bin/magick /etc/ImageMagick-7/policy.xml /etc/ImageMagick-7/type.xml /etc/ImageMagick-7/delegates.xml \
         /etc/ImageMagick-7/sRGB.icm /usr/share/ImageMagick-7/english.xml /usr/share/fonts/dejavu/DejaVuSans.ttf \
         /usr/share/fonts/nanum/NanumGothic-Regular.ttf /usr/share/fonts/nanum/NanumMyeongjo-Bold.ttf \
         /usr/share/doc/imagemagick/AGENTS.md /root/.agent/skills/imagemagick/SKILL.md /root/.claude/skills/imagemagick/SKILL.md \
         /etc/codex/skills/imagemagick/SKILL.md; do
  check "installed $f" "test -s $f"
done
for t in convert identify mogrify composite compare montage stream conjure magick-script; do
  check "link $t" "test \"\$(readlink /usr/bin/$t)\" = magick"
done
check "no X11 tools" "test ! -e /usr/bin/display -a ! -e /usr/bin/animate -a ! -e /usr/bin/import"
magick -version > version.txt; check "version 7.1.2-34 Q16-HDRI" "grep -q 'ImageMagick 7.1.2-34 Q16-HDRI' version.txt"
check "features OpenMP HDRI" "grep -q 'Features: .*HDRI.*OpenMP\|Features: .*OpenMP.*HDRI\|Features: Cipher HDRI OpenMP' version.txt"
for d in freetype heic jng jp2 jpeg lcms lzma png tiff webp xml zlib; do check "delegate $d" "grep -q 'Delegates.*\b$d\b' version.txt"; done
magick -list format > formats.txt
for f in 'PNG\* rw-' 'JPEG\* rw-' 'GIF\* rw+' 'WEBP\* rw+' 'TIFF\* rw+' 'JP2\* rw-' 'SVG  rw+' 'PDF  rw+' 'ICO\* rw+' 'PSD\* rw+' 'MSVG\* rw+' 'AVIF  rw+' 'HEIC  r--'; do
  check "format $f" "grep -q '$f' formats.txt"
done
magick -list resource > resource.txt
check "threads: every CPU ($(nproc))" "grep -q 'Thread: $(nproc)\$' resource.txt"
check "memory limit 2GiB" "grep -q 'Memory: 2GiB' resource.txt"
check "fallback-font policy" "magick -list policy | grep -q 'value: NanumGothic,DejaVu Sans'"
magick -list font | grep -c 'Font:' > fonts.txt; check "fonts listed ($(cat fonts.txt))" "test $(cat fonts.txt) -ge 60"
# the fallback font draws what DejaVu Sans has no glyph for; -define type:fallback-font=none draws spaces
magick -size 200x60 xc:white -font DejaVu-Sans -pointsize 30 -annotate +10+40 '안녕' fb.png
magick -size 200x60 xc:white -font DejaVu-Sans -pointsize 30 -define type:fallback-font=none -annotate +10+40 '안녕' nofb.png
magick -size 200x60 xc:white -font NanumGothic -pointsize 30 -annotate +10+40 '안녕' nanum.png
check "fallback font draws Hangul" "test \"\$(magick fb.png -format '%[fx:mean<0.99]' info:)\" = 1"
check "fallback: the same glyphs as NanumGothic" "test \"\$(magick compare -metric AE fb.png nanum.png null: 2>&1 | cut -d' ' -f1)\" = 0"
check "fallback-font=none draws nothing" "test \"\$(magick nofb.png -format '%[fx:mean]' info:)\" = 1"
magick -size 200x60 xc:white -font DejaVu-Sans -pointsize 30 -define type:fallback-font=NanumMyeongjo -annotate +10+40 '안녕' my.png
check "fallback font from -define" "test \"\$(magick compare -metric AE my.png fb.png null: 2>&1 | cut -d' ' -f1)\" != 0"
# external programs: a delegate is started with posix_spawn (the guest has no fork)
mkdir -p conf && cp /etc/ImageMagick-7/*.xml conf/ && sed -i 's|</delegatemap>|<delegate decode="cpytest" command="\&quot;cp\&quot; \&quot;%i\&quot; \&quot;%o\&quot;"/></delegatemap>|' conf/delegates.xml
magick rose: rose.png
check "delegate program spawned" "MAGICK_CONFIGURE_PATH=$PWD/conf magick cpytest:rose.png spawned.png && test \"\$(magick compare -metric AE rose.png spawned.png null: 2>&1 | cut -d' ' -f1)\" = 0"
# write errors are reported (a full device, directly and through stdout)
check "write error reported (file)" "! magick -size 1x1 xc:white MIFF:/dev/full"
check "write error reported (stdout)" "! magick -size 1x1 xc:white MIFF:- > /dev/full"
magick -size 1x1 xc:white MIFF:/dev/full 2> full.txt; check "write error message" "grep -q 'unable to write file' full.txt"
# pipes and descriptors
cat rose.png | magick - -resize 50% png:- | magick identify -format '%wx%h' - > pipe.txt
check "stdin to stdout pipe ($(cat pipe.txt))" "test \"\$(cat pipe.txt)\" = 35x23"
echo "-read rose.png -resize 10x10 -write info:" > script.txt
magick -script fd:5 5<script.txt > script-out.txt; check "-script from a descriptor" "grep -q '10x7' script-out.txt"
check "reading a missing file fails" "! magick nothere.png x.png"
magick nothere.png x.png 2> missing.txt; check "missing file message" "grep -q 'unable to open image' missing.txt"
echo 'not an image' > junk.bin
check "unknown format fails" "! magick junk.bin x.png"
magick junk.bin x.png 2> junk.txt; check "unknown format message" "grep -q 'no decode delegate' junk.txt"
# the pixel cache on disk (beyond the memory limit), in the guest's /tmp
check "pixel cache on disk" "magick -limit memory 64MiB -limit disk 4GiB -size 4000x3000 gradient:red-blue -blur 0x1 -resize 10% big.png && test \"\$(magick identify -format %wx%h big.png)\" = 400x300"
# OpenMP: the same pixels with one thread
magick -size 1200x800 -seed 1 plasma: p.png
magick p.png -gaussian-blur 0x2 -resize 50% -unsharp 0x1 t2.png
magick -limit thread 1 p.png -gaussian-blur 0x2 -resize 50% -unsharp 0x1 t1.png
check "threads: the same result as one thread" "test \"\$(magick compare -metric AE t1.png t2.png null: 2>&1 | cut -d' ' -f1)\" = 0"
# random numbers with -seed: the same from run to run
magick rose.png -seed 3 -attenuate 0.5 +noise Gaussian n1.png; magick rose.png -seed 3 -attenuate 0.5 +noise Gaussian n2.png
check "-seed: the same noise twice" "test \"\$(magick compare -metric AE n1.png n2.png null: 2>&1 | cut -d' ' -f1)\" = 0"
magick rose.png -seed 4 -attenuate 0.5 +noise Gaussian n3.png
check "-seed: another seed, other noise" "test \"\$(magick compare -metric AE n1.png n3.png null: 2>&1 | cut -d' ' -f1)\" != 0"
# SVG without Inkscape: ImageMagick's own renderer, without a word about the missing program
magick -size 10x10 xc:red r.png
printf '<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="blue"/></svg>' > s.svg
magick s.svg s.png 2> svg.txt; check "SVG read quietly (no inkscape message)" "test ! -s svg.txt && test \"\$(magick identify -format %wx%h s.png)\" = 20x10"
# what this build does not do: read PDF/PostScript (no Ghostscript), write HEIC (no HEVC encoder)
magick -size 10x10 xc:white x.pdf
check "PDF written, not read" "test -s x.pdf && ! magick x.pdf y.png 2>/dev/null"
magick rose.png x.heic 2> heic.txt; check "HEIC not written, and said so" "grep -q 'no encode delegate' heic.txt"
"""

TIMING = r"""
magick -size 4000x3000 -seed 1 plasma:fractal -quality 90 photo12.jpg
t() { s=$(cut -d' ' -f1 /proc/uptime); "$@" >/dev/null 2>&1; e=$(cut -d' ' -f1 /proc/uptime); echo "=== $s $e $*"; }
t magick photo12.jpg -resize 1600x1200 o1.jpg
t magick photo12.jpg -thumbnail 256x256 o2.png
t magick photo12.jpg -resize 25% -quality 85 o3.webp
t magick photo12.jpg -gaussian-blur 0x3 o4.jpg
t magick photo12.jpg -rotate 15 o5.jpg
t magick identify -format '%wx%h %[colorspace]' photo12.jpg
t magick photo12.jpg -resize 50% -font DejaVu-Sans -pointsize 60 -fill white -annotate +40+120 'Caption 캡션' o6.jpg
"""


GUIDE_INPUTS = r"""
magick -size 1600x1200 -seed 2 plasma:orange-purple -quality 90 in.jpg
cp in.jpg photo.jpg; magick in.jpg -resize 50% a.jpg; magick in.jpg -resize 40% -flop b.jpg
cp a.jpg before.jpg; magick a.jpg -modulate 120 after.jpg
magick logo: -resize 50% -fuzz 5% -transparent white logo.png; cp logo.png in.png
magick a.jpg a.png; magick b.jpg -resize 800x600! b.png; cp a.png out.png
magick -size 400x300 xc:'#e3f2fd' page1.png; magick -size 400x300 xc:'#fce4ec' page2.png
for i in 1 2 3; do magick -size 120x80 xc:"rgb($((i*80)),90,160)" frame$i.png; cp frame$i.png f$i.png; done
magick -delay 30 frame1.png frame2.png frame3.png anim.gif
printf '<svg xmlns="http://www.w3.org/2000/svg" width="200" height="120"><rect width="200" height="120" fill="#4caf50"/><circle cx="100" cy="60" r="40" fill="#fff"/></svg>' > in.svg
"""


def guide_commands(path):
    """The commands of a guide's ```sh blocks, continuation lines joined."""
    text = open(path, encoding="utf-8").read()
    commands = []
    for block in re.findall(r"```sh\n(.*?)```", text, re.S):
        joined = re.sub(r"\\\n\s*", "", block)
        for line in joined.splitlines():
            line = re.sub(r"\s+#\s.*$", "", line).strip()
            if line:
                commands.append(line)
    return commands


def check_guides():
    """Every command of the guide and the skill, in the guest, on sample files: it must run
    (a compare may say the images differ: status 1) without errors or warnings."""
    w = workdir("im-guide-")
    heif = [d for d in os.listdir(os.path.join(B, "wasm", "deps-src")) if d.startswith("libheif-")][0]
    shutil.copy(os.path.join(B, "wasm", "deps-src", heif, "examples", "example.heic"), os.path.join(w, "photo.heic"))
    status, out = native(GUIDE_INPUTS, cwd=w)
    if not check("the guide's sample files", status == 0, out):
        return
    for doc in ("doc/AGENTS.md", "skill/SKILL.md"):
        commands = guide_commands(os.path.join(A, "share", doc))
        parts = []
        for i, c in enumerate(commands):
            parts.append(f"echo '=== ex {i}'; ({c}) 2>&1; echo \"=== st $?\"")
        _, out = guest("\n".join(parts), mounts=[(w, "/work")], timeout=1800)
        results = dict((int(m.group(1)), (m.group(2), int(m.group(3))))
                       for m in re.finditer(r"=== ex (\d+)\n(.*?)=== st (\d+)", out, re.S))
        bad = []
        for i, c in enumerate(commands):
            printed, st = results.get(i, ("(no result)", -1))
            ok_status = st == 0 or (st == 1 and "compare" in c)
            noise = re.search(r"(?i)\b(error|warning|unable|unrecognized|no decode)\b", printed)
            if not ok_status or noise:
                bad.append(f"{c}\n    status {st}: {printed.strip()[:300]}")
        check(f"{doc}: its {len(commands)} example commands run in the guest", not bad and len(results) == len(commands),
              "\n".join(bad) or out[-1500:])
    # what some of them made
    _, out = guest("magick identify -format '%f %wx%h %T\\n' anim.gif favicon.ico sheet.jpg square.jpg captioned.jpg "
                   "capped.jpg doc.pdf 2>&1; wc -c < capped.jpg", mounts=[(w, "/work")])
    check("the examples' results: an animation of 50/100 s frames, a 400x400 square, a capped JPEG",
          "anim.gif 120x80 50" in out and "square.jpg 400x400" in out and "sheet.jpg" in out
          and int((re.findall(r"^\s*(\d+)\s*$", out, re.M) or ["999999"])[-1]) <= 200 * 1024, out)


def run_cases(runner, root, inputs):
    """The cases in root/out/<name>: {name: (status, printed)}."""
    parts = [f"export IN='{inputs}'; cd '{root}' || exit 1"]
    for name, command in CASES:
        parts.append(f"echo '{MARK}{name}'; mkdir -p 'out/{name}'; (cd 'out/{name}' && {command}) 2>&1; "
                     f"s=$?; echo; echo \"{END}$s\"")
    status, out = runner("\n".join(parts))
    found, current, lines = {}, None, []
    for line in out.splitlines():
        if line.startswith(MARK):
            current, lines = line[len(MARK):].strip(), []
        elif line.startswith(END) and current:
            found[current] = (int(line[len(END):].strip() or -1), "\n".join(lines))
            current = None
        elif current:
            lines.append(line)
    return found, out


def ref_magick(*args):
    p = subprocess.run([MAGICK, *args], capture_output=True, env=dict(os.environ, MAGICK_CONFIGURE_PATH=CONFIG))
    return p.returncode, p.stdout.decode(errors="replace"), p.stderr.decode(errors="replace")


def normalize(text):
    """What a run prints, without what differs from run to run (times, temporary names)."""
    text = re.sub(r"\d+\.\d+u \d+:\d+\.\d+", "T", text)
    text = re.sub(r"/tmp/magick-[A-Za-z0-9_-]+", "/tmp/magick-X", text)
    text = re.sub(r"(?m)^\s*(Elapsed time|User time|Pixels per second).*\n?", "", text)
    return text.replace("\r", "")


PDF_VARYING = re.compile(rb"/(CreationDate|ModDate) \([^)]*\)|<xmp:[A-Za-z]+Date>[^<]*</xmp:[A-Za-z]+Date>"
                         rb"|<xmpMM:(Document|Instance)ID>[^<]*</xmpMM:\w+ID>|/ID \[<[^>]*> <[^>]*>\]")


def same_file(a, b):
    """'bytes', 'pixels', ('rounding', rmse) or a difference (str)."""
    da, db = open(a, "rb").read(), open(b, "rb").read()
    if da == db:
        return "bytes"
    if a.endswith(".pdf"):
        return "pixels" if PDF_VARYING.sub(b"", da) == PDF_VARYING.sub(b"", db) else "PDFs differ beyond their dates"
    fmt = "%m %wx%h %z %[colorspace] %[channels] %[type] %#\n"
    sa, sb = ref_magick("identify", "-format", fmt, a)[1], ref_magick("identify", "-format", fmt, b)[1]
    if sa and sa == sb:
        return "pixels"
    if not sa or not sb or sa.split()[:5] != sb.split()[:5]:
        return f"guest: {sa.strip()[:200]} / reference: {sb.strip()[:200]}"
    # differ in pixels: by how much (normalized RMSE of the first frame)
    _, _, err = ref_magick("compare", "-metric", "RMSE", a + "[0]", b + "[0]", "null:")
    m = re.search(r"\(([\d.e+-]+)\)", err)
    if m and float(m.group(1)) < 1e-5:
        return ("rounding", float(m.group(1)))
    return f"pixels differ: RMSE {err.strip()[:120]}"


def main():
    if not present():
        return 1
    w = workdir("im-guest-")
    inputs = os.path.join(w, "in")
    os.makedirs(inputs)
    with open(os.path.join(inputs, "text.svg"), "w") as f:
        f.write(SVG)
    with open(os.path.join(inputs, "sample.mvg"), "w") as f:
        f.write(MVG)
    shutil.copy(os.path.join(B, "src", "config", "sRGB.icm"), inputs)
    shutil.copy(os.path.join(B, "fonts", "dejavu", "DejaVuSansMono.ttf"), inputs)
    for example in ("example.heic", "example.avif"):   # libheif's sample images
        heif = [d for d in os.listdir(os.path.join(B, "wasm", "deps-src")) if d.startswith("libheif-")][0]
        shutil.copy(os.path.join(B, "wasm", "deps-src", heif, "examples", example), inputs)
    status, out = native(INPUTS, cwd=inputs)
    if not check("inputs made by the reference", status == 0, out):
        return summary()
    for d in ("g", "n"):
        os.makedirs(os.path.join(w, d))

    with Timer() as tn:
        ref, ref_out = run_cases(lambda c: native(c, cwd=w, timeout=1800), os.path.join(w, "n"), inputs)
    with Timer() as tg:
        got, got_out = run_cases(lambda c: guest(c, mounts=[(w, "/work")], timeout=3600), "/work/g", "/work/in")
    print(f"({len(CASES)} cases: reference {tn.seconds:.1f} s, guest {tg.seconds:.1f} s)")

    counts = {"bytes": 0, "pixels": 0, "rounding": 0}
    rounding = []
    for name, _ in CASES:
        if name not in got:
            check(name, False, "no result from the guest\n" + got_out[-1500:])
            continue
        gs, gp = got[name]
        ns, np_ = ref.get(name, (None, ""))
        problems = []
        if gs != ns:
            problems.append(f"status {gs} (reference {ns})")
        if normalize(gp) != normalize(np_):
            problems.append(f"printed:\n--- guest\n{gp[-1500:]}\n--- reference\n{np_[-1500:]}")
        gdir, ndir = os.path.join(w, "g", "out", name), os.path.join(w, "n", "out", name)
        gfiles = sorted(os.listdir(gdir)) if os.path.isdir(gdir) else []
        nfiles = sorted(os.listdir(ndir)) if os.path.isdir(ndir) else []
        if gfiles != nfiles:
            problems.append(f"files {gfiles} (reference {nfiles})")
        how = []
        for f in gfiles:
            pg, pn = os.path.join(gdir, f), os.path.join(ndir, f)
            if os.path.isdir(pg):
                continue
            if not os.path.exists(pn):
                continue
            r = same_file(pg, pn)
            if isinstance(r, tuple):
                counts["rounding"] += 1
                rounding.append(f"{name}/{f} ({r[1]:.1e})")
                how.append("rounding")
            elif r in counts:
                counts[r] += 1
                how.append(r)
            else:
                problems.append(f"{f}: {r}")
        label = f"{name}" + (f" [{', '.join(sorted(set(how)))}]" if how else "")
        if ns != 0 and gs == ns:
            label += f" (exit {ns} on both)"
        check(label, not problems, "\n".join(problems))
    print(f"\noutput files: {counts['bytes']} the same bytes, {counts['pixels']} the same pixels "
          f"(their formats record the time), {counts['rounding']} within rounding"
          + (": " + ", ".join(rounding) if rounding else ""))

    # PerlMagick's sample files of many formats: decoded alike in the guest and natively
    perl = sorted(f for f in os.listdir(PERL_T) if f.startswith("input") and not f.endswith((".t",)))
    perl += sorted(os.path.join(d, f) for d in ("png", "tiff", "jpeg", "openjp2", "jng", "ttf", "zlib", "rad", "jbig", "bzlib")
                   if os.path.isdir(os.path.join(PERL_T, d))
                   for f in os.listdir(os.path.join(PERL_T, d)) if f.startswith("input"))
    fmt = "%m %wx%h %z %[colorspace] %#\\n"
    script = "; ".join(f"echo '=== {p}'; magick identify -format '{fmt}' '{p}[0]' 2>&1" for p in perl)
    _, gout = guest(f"{{ {script}; }}", mounts=[(PERL_T, "/perl")], cwd="/perl")
    _, nout = native(f"{{ {script}; }}", cwd=PERL_T)

    def split(text):
        out, cur = {}, None
        for line in text.splitlines():
            if line.startswith("=== "):
                cur = line[4:]
                out[cur] = []
            elif cur and not line.startswith("root@collabo"):
                out[cur].append(line)
        return {k: "\n".join(v) for k, v in out.items()}
    gp, npr = split(gout), split(nout)
    same = [p for p in perl if p in gp and gp[p] == npr.get(p)]
    decoded = [p for p in same if not re.search(r"(?i)error|unable|no decode|delegate", gp[p])]
    diff = [f"{p}: guest {gp.get(p, '?')[:150]!r} / reference {npr.get(p, '?')[:150]!r}" for p in perl if p not in same]
    check(f"PerlMagick's sample files: {len(same)} of {len(perl)} read alike ({len(decoded)} decoded, "
          f"the rest refused alike)", not diff, "\n".join(diff))

    check_guides()

    # what only the guest shows
    g = workdir("im-guest-only-")
    status, out = guest(GUEST_ONLY, mounts=[(g, "/work")], timeout=1800)
    for line in out.splitlines():
        m = re.match(r"(not ok|ok) (.*)", line)
        if m:
            check(m.group(2), m.group(1) == "ok")
    if "ok " not in out:
        check("guest-only checks ran", False, out[-2000:])

    # speed: a 12-megapixel photo (informational)
    t = workdir("im-speed-")
    with Timer() as ts:
        status, out = guest(TIMING, mounts=[(t, "/work")], timeout=1800)
    print(f"speed in the guest ({ts.seconds:.1f} s with the boot), a 12-megapixel photo:")
    for m in re.finditer(r"^=== ([\d.]+) ([\d.]+) (.*)$", out, re.M):
        print(f"  {float(m.group(2)) - float(m.group(1)):6.1f} s  {m.group(3)}")
    return summary()


if __name__ == "__main__":
    sys.exit(main())
