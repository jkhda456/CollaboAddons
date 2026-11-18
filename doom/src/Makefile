# The doom add-on's program: doomgeneric's engine with the terminal frontend of this folder.
# build.sh runs it twice — for the guest (wasm-cc) and for this machine (the reference):
#
#   make -f src/Makefile SRC=<doomgeneric/doomgeneric> OUT=<objects> CC=<compiler> [OPT=-O2]
#
# Writes OUT/doom, and OUT/test_render, OUT/test_input (the frontend's unit tests).
#
# Copyright (C) 2026 the collaboCore add-ons' authors. GPL-3.0-only (addons/LICENSE).
HERE := $(dir $(lastword $(MAKEFILE_LIST)))
SRC ?= doomgeneric
OUT ?= obj
CC ?= cc
OPT ?= -O2
CFLAGS ?=
LDFLAGS ?=
VERSION ?= dev

# The engine's files: doomgeneric's Makefile's, without the X11 frontend and the ENDOOM screen of
# Chocolate Doom's text mode library (doomterm.c shows ENDOOM)
DOOM := dummy am_map doomdef doomstat dstrings d_event d_items d_iwad d_loop d_main d_mode d_net \
  f_finale f_wipe g_game hu_lib hu_stuff info i_cdmus i_joystick i_scale i_sound i_system i_timer \
  memio m_argv m_bbox m_cheat m_config m_controls m_fixed m_menu m_misc m_random p_ceilng p_doors \
  p_enemy p_floor p_inter p_lights p_map p_maputl p_mobj p_plats p_pspr p_saveg p_setup p_sight \
  p_spec p_switch p_telept p_tick p_user r_bsp r_data r_draw r_main r_plane r_segs r_sky r_things \
  sha1 sounds statdump st_lib st_stuff s_sound tables v_video wi_stuff w_checksum w_file w_main \
  w_wad z_zone w_file_stdc i_input i_video doomgeneric
FRONT := doomterm render input deh_lite

DEFS := -DDOOMGENERIC_RESX=320 -DDOOMGENERIC_RESY=200 -DNORMALUNIX -DLINUX -D_DEFAULT_SOURCE \
  -DDOOMTERM_VERSION='"$(VERSION)"'
ALL_CFLAGS := -std=gnu11 $(OPT) -fno-strict-aliasing -fwrapv $(DEFS) $(CFLAGS)

all: $(OUT)/doom $(OUT)/test_render $(OUT)/test_input

$(OUT)/doom: $(addprefix $(OUT)/,$(addsuffix .o,$(DOOM) $(FRONT)))
	$(CC) $(LDFLAGS) -o $@ $^ -lm

$(OUT)/test_render: $(OUT)/test_render.o $(OUT)/render.o
	$(CC) $(LDFLAGS) -o $@ $^ -lm

$(OUT)/test_input: $(OUT)/test_input.o $(OUT)/input.o
	$(CC) $(LDFLAGS) -o $@ $^ -lm

# DOOM's own code as it is (its warnings are not ours)
$(OUT)/%.o: $(SRC)/%.c | $(OUT)
	$(CC) $(ALL_CFLAGS) -w -MMD -MP -c -o $@ $<

$(OUT)/%.o: $(HERE)%.c | $(OUT)
	$(CC) $(ALL_CFLAGS) -I$(SRC) -I$(OUT) -Wall -Wextra -Wno-unused-parameter -Wno-sign-compare -MMD -MP -c -o $@ $<

$(OUT)/%.o: $(HERE)../tests/unit/%.c | $(OUT)
	$(CC) $(ALL_CFLAGS) -I$(HERE) -Wall -Wextra -MMD -MP -c -o $@ $<

# BEX names of DOOM's texts: every string macro of d_englsh.h
$(OUT)/deh_names.h: $(SRC)/d_englsh.h | $(OUT)
	sed -n 's/^#define[ \t]\{1,\}\([A-Z0-9_]\{1,\}\)[ \t]\{1,\}".*/    {"\1", \1},/p' $< > $@.tmp && mv $@.tmp $@

$(OUT)/deh_lite.o: $(OUT)/deh_names.h

$(OUT):
	mkdir -p $@

-include $(wildcard $(OUT)/*.d)

.PHONY: all
