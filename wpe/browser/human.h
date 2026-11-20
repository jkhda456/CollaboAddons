/* Human input's timing and paths (human.c), for the browser's actions (browser.c). */
#pragma once
#include <glib.h>

typedef struct {
    double x, y;
    double t; /* ms from the movement's start */
} HumanPoint;

/* a log-normal delay in ms: its median, the factor its p90 is above the median, within [min, max] */
double human_delay(GRand *rand, double median, double spread, double min, double max);

/* the pointer's way from (x0, y0) to (x1, y1), a target `target_size` px across: HumanPoints, one
 * per report of a mouse reporting every `report_ms`, the last exactly (x1, y1); empty when the
 * pointer is there already */
GArray *human_path(GRand *rand, double x0, double y0, double x1, double y1, double target_size, double report_ms);
