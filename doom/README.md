# doom add-on

DOOM in the sandbox's terminal: the engine is doomgeneric (Chocolate Doom, from id Software's
released source), and the add-on's own frontend (`src/`) draws it in the shell the guest runs —
collaboCore's console in a terminal, the web page's xterm.js, any terminal. Each 320×200 frame is
converted in real time to the terminal's size (and again when the window changes): block
characters in 24-bit colour, only the changed cells sent. Freedoom 0.13.0 — free IWADs for the
engine — is included, so `doom` plays at once; DOOM's own IWADs are found if you bring them.

```
$ doom                        # Freedoom: Phase 1
$ doom -iwad freedoom2.wad    # Phase 2
$ doom -warp 1 3 -skill 4 -fps
$ doom -h                     # keys and options
```

The guide, `share/doc/README.md`, is installed as `/usr/share/doc/doom/README.md`: keys, options,
IWADs.

## What it does

- **Picture.** The frame is box-filtered to the cells the terminal has, keeping DOOM's 4:3 (cells
  taken as twice as high as wide; `-cellaspect`, `-stretch`) and centred: `▀` half blocks with
  fore- and background colour (2 pixels a cell, default), quadrant blocks (2×2 pixels a cell in the
  two colours that fit them best: `-blocks quad`) or a brightness ramp of ASCII characters
  (`-blocks ascii`); 24-bit, xterm 256, the 16 ANSI colours or none (`-color`). `` ` `` and `~`
  switch them while playing. A terminal that draws block characters two columns wide (East Asian
  ambiguous width) is noticed at the start — the cursor's position after one — and laid out for it.
- **Bytes.** Only the cells that look different are sent, with cursor moves and the colours that
  changed, a half block or its inverse whichever needs fewer colour changes, inside DEC 2026
  synchronized output (no tearing), in raw mode (one write a frame).
- **A terminal slower than the game.** Each frame gets a budget: what the terminal takes in about
  18 ms, measured by how long writes wait (in collaboCore's guest a console write returns when the
  host has the bytes, even non-blocking) or by a frame still unsent when the next comes. Within
  the budget the cells that look most wrong go first, spread over the picture; the rest follow in
  later frames. While cells keep waiting, colour precision steps down (6 bits a channel to 3),
  then the picture is made smaller (¾, ½, ⅓ of the terminal: a lower resolution), and both come
  back once the terminal keeps up. The game runs at 35 tics a second all the while. Measured here
  (tests/): 35 frames a second at 131×49 cells through the guest's console (~5 MB/s); a terminal
  that takes 300 KB/s gets 3 bits and 64×24 cells within a few seconds.
- **Keys.** Legacy escape sequences (xterm, VT220, SS3, the Linux console, modifyOtherKeys), the
  **kitty keyboard protocol** (asked for; presses, repeats and releases, Ctrl/Shift/Alt alone) and
  Windows Terminal's **win32-input-mode** (likewise). Without releases, a key is let go when its
  repeats stop: a tapped turn key turns a little, a held one keeps turning (learned from the
  terminal's own repeat delay and rate), a held key stays down while a later one takes its repeats
  over. Every press and repeat reaches DOOM as a key-down with its character, as DOS DOOM got
  typematic repeats (cheats, save names). Playing, the keys are DOOM's plus W A S D, F/Enter to
  fire, E/Space to use, R for always-run; in menus they are DOOM's. A Korean input method's jamo
  work as the keys they are on.
- **The terminal back.** The alternate screen, the cursor, wrapping, the title, the keyboard
  modes are restored on every way out (quit, Ctrl+C, I_Error, signals), DOOM's messages while it
  runs go to `-log FILE` or nowhere, and the IWAD's ENDOOM screen (CP437, VGA colours) is printed
  at the end.
- **Freedoom.** Its DEHACKED lump is read (`src/deh_lite.c`: BEX texts — level names, messages,
  the story screens —, frame changes, par times), which doomgeneric left out.
- **Without a terminal** (`-shot`, `-shotansi`, `-keys`, `-benchrender`): frames as PPM or as a
  terminal is sent them, keys at given tics, simulated time — the same frames on every run.

## Turning it on

```json
"addons": {"doom": true}
```

or `--addon-dir addons/doom/out --addon doom` on the engine's command line. It has no settings.
The image (55 MB: the two IWADs; the program is 0.5 MB) holds `/usr/bin/doom`,
`/usr/share/games/doom/freedoom{1,2}.wad` and `/usr/share/doc/doom/` (the guide, Freedoom's
COPYING and CREDITS).

## Building

```sh
addons/doom/build.sh            # or: python3 addons/build.py build doom
addons/doom/tests/run.sh        # unit frames tty slow errors
```

It needs `./build.sh userspace tools` first (the musl sysroot, compiler-rt, binaryen),
clang-19/lld-19, make, unzip, python3, and network access for the first downloads (`sources.lock`,
sha256-checked: doomgeneric at commit e257a09, Freedoom 0.13.0's release). It takes seconds.
Stages (`FORCE=stage` redoes one): `src`, `wads`, `host`, `cross`, `stage`, `cpio`. `src/Makefile`
builds the game and the unit tests for both machines (`host`: this one, the tests' reference;
`cross`: the guest, `userspace/bin/wasm-cc` with the guest's link flags).

How it is put together:

- `src/doomterm.c` — doomgeneric's interface (`DG_DrawFrame`, `DG_GetKey`, `DG_SleepMs`, ...):
  the terminal session, frame pacing and budgets, keys to DOOM's events, headless shots, ENDOOM.
  `src/render.c` — frames to escape sequences. `src/input.c` — escape sequences to keys.
  `src/deh_lite.c` — DEHACKED for an IWAD.
- `patches/doomgeneric/` — the engine's changes: `0001` DEH_String a function and Freedoom's
  lump loaded, and `G_CheckDemoStatus` registered as an exit function through a cast — WebAssembly
  traps on such a call (the program died with SIGSEGV on every quit); `0002` I_Error gives the
  terminal back before its message, no zenity; `0003` the configuration and saves in `~/.doom/`;
  `0004` the IWAD search of Chocolate Doom on Unix (doomgeneric looks in `.` only), Freedoom
  Phase 1 before Phase 2. `tools/save-patches.sh` writes `build/src/doomgeneric`'s changes back.
- `tests/` — `test_guest.py`: the unit tests (`tests/unit/`) natively and in the guest; 50 frames
  and terminal renderings (both IWADs, demos, play with scripted keys, every mode) the same in the
  guest as natively, byte for byte, and what a terminal emulator (`tests/tools/vt.py`) shows of
  them the picture scaled as computed independently; the game in the guest's shell in a
  pseudo-terminal (`tests/tools/ptyrun.py`): menus, a game, legacy keys and their inferred releases,
  repeats, double taps, a cheat, kitty and win32 terminals answering the queries, a wide-cell
  terminal, a resize, quitting; the renderer's idea of the screen against the emulator's after the
  whole byte stream; a 300 KB/s terminal (budget, precision, size, real time kept).

## License

doomgeneric is GPL-2.0-or-later (Chocolate Doom; DOOM's source as id Software released it),
Freedoom BSD-3-Clause; their texts are in `out/licenses`. The add-on's own work — the terminal
frontend, the patches, tools and tests — is GPL-3.0-only (`../LICENSE`). DOOM is id Software's
trademark; the add-on runs its released engine and is not affiliated with id Software.
