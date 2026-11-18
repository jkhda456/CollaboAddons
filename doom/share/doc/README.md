# doom — DOOM in the terminal

`doom` is DOOM's engine (doomgeneric: Chocolate Doom, from id Software's released source) drawing
into the terminal it runs in. Each frame (320×200) is scaled to the terminal as it is — and again
whenever the window changes size — and drawn with block characters in 24-bit colour; only the
cells that changed are sent. When the terminal cannot take a whole frame in a frame's time, the
cells that look most wrong go first; if cells keep waiting, colour precision steps down (6 bits a
channel to 3) and then the picture is made smaller — ¾, ½, ⅓ of the terminal, a lower resolution
converted in real time — and both come back once the terminal keeps up. The game itself always
runs in real time (35 tics a second).

```sh
doom                          # Freedoom: Phase 1 (or your own IWAD, see below)
doom -iwad freedoom2.wad      # Freedoom: Phase 2 (DOOM II's maps)
doom -warp 1 3 -skill 4       # episode 1, map 3, Ultra-Violence
doom -fps                     # a status line: frames/s, bytes/s, the frame budget, the mode
doom -h                       # every key and option
```

## Keys

| Key | | Key | |
|---|---|---|---|
| arrows, W S | move, turn | A D, `,` `.` | strafe |
| F, Enter, Ctrl* | fire | Space, E | open doors, use switches |
| Shift + move* | run | R | always run on/off |
| Alt + ←/→* | strafe | 1–7 | weapons |
| Tab | map | Esc | menu (Y/N to answer it) |
| F1–F12 | help, save, load, … as in DOOM | P, Pause | pause |
| `` ` `` | picture: half blocks → quadrants → ASCII | `~` | colours: 24-bit → 256 → 16 → none |
| `\` | status line on/off | Ctrl+L | draw everything again |
| Ctrl+C | quit | | |

In menus, letters type (save-game names), arrows move and Enter chooses. Cheats work as in DOOM.

\* **Key releases.** A terminal normally sends a key's character when it goes down and again while
it repeats, and nothing when it is let go. Where the terminal can say more, `doom` asks it to:

- the **kitty keyboard protocol** (kitty, WezTerm, foot, Ghostty, iTerm2, Alacritty, …) and
  **win32-input-mode** (Windows Terminal) report presses *and* releases, and Ctrl, Shift and Alt
  alone: hold several keys at once, fire with Ctrl, run with Shift, as in DOOM;
- elsewhere a key counts as let go when its repeats stop. A tapped turn key turns a little
  (`-keyhold MS`, default 140 ms), a held one keeps turning; a move key held before another key
  stays down while that one is held (most terminals repeat only the last key). Ctrl and Shift
  work with the arrows (Ctrl+arrow fires, Shift+arrow runs), not alone.

With a Korean input method switched on, the jamo keys (ㅈ ㄴ ㅁ ㅇ …) work as the letters under
them (W S A D …).

## The picture

| Option | |
|---|---|
| `-blocks half` | ▀ with fore- and background colours: 2 pixels a cell (default) |
| `-blocks quad` | quadrant blocks (▘▝▖▗▚▞…): 2×2 pixels a cell in two colours each — twice the width's detail |
| `-blocks ascii` | a character per cell by brightness, coloured |
| `-color truecolor\|256\|16\|mono` | the colours the terminal takes (default 24-bit; 16 on the Linux console; `mono`: ASCII only) |
| `-size COLSxROWS` | the picture no larger than this, always (otherwise it shrinks by itself on a slow terminal, and grows back) |
| `-stretch` | fill the terminal instead of keeping DOOM's 4:3 |
| `-cellaspect N` | a character cell's height / width (default 2.0) |
| `-quant BITS` | bits kept per colour channel, fixed (default: 6, down to 3 under load) |
| `-bandwidth KB` | send no more than KB kilobytes a second (default: what the terminal takes) |

A terminal that draws block characters two columns wide (East Asian "ambiguous width" set to
wide) is noticed at the start (the cursor's position after one) and each cell then takes two
columns.

## IWADs

The add-on brings Freedoom 0.13.0 — free content for the DOOM engine (BSD licence):
`/usr/share/games/doom/freedoom1.wad` (Phase 1, DOOM's four episodes; the default) and
`freedoom2.wad` (Phase 2, 32 maps like DOOM II). Freedoom's own texts, level names and par times
(its DEHACKED lump) are used.

DOOM's own IWADs work too: put `doom1.wad` (shareware), `doom.wad`, `doom2.wad`, `plutonia.wad` or
`tnt.wad` in the current folder, in `$DOOMWADDIR`, in `/usr/share/games/doom`, or give
`-iwad PATH`. Without `-iwad` the first found, in that order, is played. PWADs: `-file x.wad`.

The configuration (`default.cfg`) and saved games are in `~/.doom/`. Other Chocolate Doom options
work as there: `-skill`, `-warp`, `-episode`, `-fast`, `-respawn`, `-nomonsters`, `-playdemo`,
`-timedemo`, `-record`, `-nodeh`, ….

There is no sound: the terminal has none.

## Without a terminal

For scripts, tests and agents that want to see a frame:

```sh
doom -shot frame.ppm -shottic 140                  # the 320x200 frame at tic 140 (4 s), then quit
doom -warp 1 1 -keys 35:+up,105:-up -shot a%d.ppm -shottic 70,140
doom -shot view.ans -termsize 100x30 -shottic 70   # what a 100x30 terminal would be sent
doom -timedemo demo1 -benchrender -termsize 160x50 # how fast the renderer is, bytes per frame
```

With `-shot` or `-keys`, time is simulated: every run gives the same frames.
