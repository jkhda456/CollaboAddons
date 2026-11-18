// doomterm: the parts of DeHackEd an IWAD's own DEHACKED lump needs — Freedoom's: its texts (the
// BEX [STRINGS] section: level names, messages, the story between episodes), changed frames
// (Frame: sprite, subnumber, duration, next frame) and par times ([PARS]). doomgeneric leaves
// Chocolate Doom's dehacked code out; DEH_String (deh_str.h) is this file's.
//
// Copyright (C) 2026 the collaboCore add-ons' authors. GPL-3.0-only (addons/LICENSE).
#include <ctype.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>

#include "doomtype.h"
#include "d_englsh.h"
#include "deh_str.h"
#include "info.h"
#include "w_wad.h"
#include "z_zone.h"

extern int pars[4][10];
extern int cpars[32];

// The BEX names of DOOM's texts: every string of d_englsh.h (made by build.sh from it).
static const struct {
    const char *name;
    const char *text;
} bex[] = {
#include "deh_names.h"
};

#define NREP 1024
static struct {
    const char *from;
    char *to;
} rep[NREP];
static int nrep;

static unsigned hash(const char *s)
{
    unsigned h = 5381;
    while (*s)
        h = (h * 33) ^ (unsigned char)*s++;
    return h;
}

static void add_replacement(const char *from, char *to)
{
    unsigned i = hash(from) & (NREP - 1);
    while (rep[i].from && strcmp(rep[i].from, from) != 0)
        i = (i + 1) & (NREP - 1);
    if (!rep[i].from) {
        if (nrep >= NREP / 2)
            return;
        nrep++;
    }
    rep[i].from = from;
    rep[i].to = to;
}

char *DEH_String(char *s)
{
    if (nrep == 0 || s == NULL)
        return s;
    unsigned i = hash(s) & (NREP - 1);
    while (rep[i].from) {
        if (strcmp(rep[i].from, s) == 0)
            return rep[i].to;
        i = (i + 1) & (NREP - 1);
    }
    return s;
}

static char *trim(char *s)
{
    while (isspace((unsigned char)*s))
        s++;
    char *e = s + strlen(s);
    while (e > s && isspace((unsigned char)e[-1]))
        *--e = 0;
    return s;
}

// "\n" in a BEX string: a new line
static char *unescape(const char *s)
{
    char *out = malloc(strlen(s) + 1), *o = out;
    while (*s) {
        if (s[0] == '\\' && s[1] == 'n') {
            *o++ = '\n';
            s += 2;
        } else if (s[0] == '\\' && s[1] == '\\') {
            *o++ = '\\';
            s += 2;
        } else {
            *o++ = *s++;
        }
    }
    *o = 0;
    return out;
}

static int block_header(const char *line, int *number)
{
    static const char *blocks[] = {"Thing", "Frame", "Pointer", "Sound", "Ammo", "Weapon", "Cheat", "Misc", "Text", "Sprite"};
    if (strchr(line, '='))
        return -1;
    for (int i = 0; i < (int)(sizeof blocks / sizeof *blocks); i++) {
        size_t n = strlen(blocks[i]);
        if (strncasecmp(line, blocks[i], n) == 0 && isspace((unsigned char)line[n])) {
            *number = atoi(line + n);
            return i;
        }
    }
    return -1;
}

void DEH_LoadIwadLump(void)
{
    if (W_CheckNumForName("FREEDOOM") < 0)
        return;
    int lump = W_CheckNumForName("DEHACKED");
    if (lump < 0)
        return;
    int len = W_LumpLength((unsigned)lump);
    char *text = malloc((size_t)len + 1);
    memcpy(text, W_CacheLumpNum(lump, PU_STATIC), (size_t)len);
    text[len] = 0;
    W_ReleaseLumpNum(lump);

    enum { NONE, FRAME, STRINGS, PARS, OTHER } section = NONE;
    int frame = -1, nstrings = 0, nframes = 0, npars = 0;
    char *save = NULL;
    for (char *raw = strtok_r(text, "\n", &save); raw; raw = strtok_r(NULL, "\n", &save)) {
        char *line = trim(raw);
        if (*line == 0 || *line == '#')
            continue;
        if (*line == '[') {
            section = strncasecmp(line, "[STRINGS]", 9) == 0 ? STRINGS
                    : strncasecmp(line, "[PARS]", 6) == 0   ? PARS
                                                           : OTHER;
            continue;
        }
        int number, b = block_header(line, &number);
        if (b >= 0) {
            section = b == 1 ? FRAME : OTHER;
            frame = number;
            continue;
        }
        if (section == STRINGS) {
            char *eq = strchr(line, '=');
            if (!eq)
                continue;
            *eq = 0;
            char *name = trim(line), *value = trim(eq + 1);
            // a value ending in a backslash goes on in the next line
            char *joined = strdup(value);
            while (strlen(joined) > 0 && joined[strlen(joined) - 1] == '\\') {
                char *next = strtok_r(NULL, "\n", &save);
                if (!next)
                    break;
                joined[strlen(joined) - 1] = 0;
                next = trim(next);
                char *j = malloc(strlen(joined) + strlen(next) + 1);
                strcpy(j, joined);
                strcat(j, next);
                free(joined);
                joined = j;
            }
            for (size_t i = 0; i < sizeof bex / sizeof *bex; i++) {
                if (strcasecmp(bex[i].name, name) == 0) {
                    add_replacement(bex[i].text, unescape(joined));
                    nstrings++;
                    break;
                }
            }
            free(joined);
        } else if (section == PARS) {
            char *hash_ = strchr(line, '#');
            if (hash_)
                *hash_ = 0;
            int a, b2, c;
            int n = sscanf(line, "par %d %d %d", &a, &b2, &c);
            if (n == 3 && a >= 1 && a <= 3 && b2 >= 1 && b2 <= 9) {
                pars[a][b2] = c;
                npars++;
            } else if (n == 2 && a >= 1 && a <= 32) {
                cpars[a - 1] = b2;
                npars++;
            }
        } else if (section == FRAME && frame >= 0 && frame < NUMSTATES) {
            char *eq = strchr(line, '=');
            if (!eq)
                continue;
            *eq = 0;
            char *field = trim(line);
            int v = atoi(trim(eq + 1));
            state_t *st = &states[frame];
            if (strcasecmp(field, "Sprite number") == 0 && v >= 0 && v < NUMSPRITES)
                st->sprite = (spritenum_t)v;
            else if (strcasecmp(field, "Sprite subnumber") == 0)
                st->frame = v;
            else if (strcasecmp(field, "Duration") == 0)
                st->tics = v;
            else if (strcasecmp(field, "Next frame") == 0 && v >= 0 && v < NUMSTATES)
                st->nextstate = (statenum_t)v;
            else if (strcasecmp(field, "Unknown 1") == 0)
                st->misc1 = v;
            else if (strcasecmp(field, "Unknown 2") == 0)
                st->misc2 = v;
            else
                continue;
            nframes++;
        }
    }
    free(text);
    printf("DEH_LoadIwadLump: the IWAD's DEHACKED: %d texts, %d frame changes, %d par times\n",
           nstrings, nframes, npars);
}
