/*
 * Human input for the browser's actions (browser.c): where a person's pointer goes on its way to a
 * point, and when their presses, keys and wheel notches happen. The browser sends each planned
 * event at its time, as the display would send a real device's (one event per report), so a page
 * that watches how input arrives — the path before a click, the time a button is held, the gaps
 * between keys — sees a person's rhythm rather than everything in the same millisecond.
 *
 * The figures are the ones Camoufox's input is fitted to (its ci/tribal-rules.yml, human-* rules,
 * from recordings of people on macOS and Windows): a click held 78/96/125 ms (p10/p50/p90), a
 * quick later press 63/83/113; the last move to the press 48-63 ms (p50) with a mouse; presses of
 * a double click 141-157 ms apart; keys held 95-125 ms (p50) with 129-235 ms between keydowns, a
 * third of them pressed while the one before is still down; a mouse reporting every 8-10 ms; a
 * wheel turned in strokes of 1-5 notches about 16 ms apart, 47-78 ms between strokes. A path is a
 * minimum-jerk movement (the bell-shaped speed of a person's reach) along a gently bent line,
 * taking as long as Fitts's law says for its distance and the target's size, sometimes past the
 * target and back on a long reach.
 */
#include <glib.h>
#include <math.h>

#include "human.h"

/* a standard normal draw (Box-Muller; 1 - u keeps the logarithm's argument above 0) */
static double gauss(GRand *rand)
{
    double u = g_rand_double(rand), v = g_rand_double(rand);
    return sqrt(-2 * log(1 - u)) * cos(2 * G_PI * v);
}

/* a log-normal draw: `median` and the factor `spread` that p90 is above it, within [min, max] */
double human_delay(GRand *rand, double median, double spread, double min, double max)
{
    double value = median * exp(gauss(rand) * log(spread) / 1.2816);
    return value < min ? min : value > max ? max : value;
}

/* minimum jerk: the share of the way done at time share s */
static double min_jerk(double s)
{
    return s * s * s * (10 - 15 * s + 6 * s * s);
}

/* One movement from (x0, y0) to (x1, y1) lasting `ms`, along a quadratic curve bent by `bend` (a
 * share of the distance, sideways), sampled every report interval from `t0`: the points after the
 * start, appended. */
static void reach(GRand *rand, GArray *points, double x0, double y0, double x1, double y1, double t0, double ms, double bend,
                  double interval)
{
    double dx = x1 - x0, dy = y1 - y0, distance = hypot(dx, dy);
    if (distance < 0.5)
        return;
    /* the control point: halfway, pushed sideways */
    double cx = x0 + dx / 2 - dy * bend, cy = y0 + dy / 2 + dx * bend;
    double tremor = distance > 40 ? 0.35 : 0.15;
    /* a report every interval (give or take a fraction of a ms), the last one at the target */
    for (double t = interval; ; t += interval + (g_rand_double(rand) - 0.5) * 0.6) {
        gboolean last = t >= ms;
        double s = last ? 1 : min_jerk(t / ms);
        double a = 1 - s;
        HumanPoint p;
        p.x = a * a * x0 + 2 * a * s * cx + s * s * x1;
        p.y = a * a * y0 + 2 * a * s * cy + s * s * y1;
        if (!last) {
            p.x += gauss(rand) * tremor;
            p.y += gauss(rand) * tremor;
        }
        p.t = t0 + t;
        g_array_append_val(points, p);
        if (last)
            break;
    }
}

GArray *human_path(GRand *rand, double x0, double y0, double x1, double y1, double target_size, double report_ms)
{
    GArray *points = g_array_new(FALSE, FALSE, sizeof(HumanPoint));
    double distance = hypot(x1 - x0, y1 - y0);
    if (distance < 0.5)
        return points;
    if (target_size < 4)
        target_size = 4;
    /* Fitts: a + b log2(D/W + 1), the constants of pointing studies with a mouse */
    double ms = (90 + 140 * log2(distance / target_size + 1)) * human_delay(rand, 1, 1.25, 0.75, 1.4);
    if (ms > 1400)
        ms = 1400;
    double bend = (g_rand_double(rand) - 0.5) * (distance > 200 ? 0.18 : 0.3);
    /* a long reach goes a little past the target now and then, and comes back */
    if (distance > 300 && g_rand_double(rand) < 0.3) {
        double over = 1 + 0.03 + g_rand_double(rand) * 0.05;
        double ox = x0 + (x1 - x0) * over + gauss(rand) * 3, oy = y0 + (y1 - y0) * over + gauss(rand) * 3;
        reach(rand, points, x0, y0, ox, oy, 0, ms * 0.85, bend, report_ms);
        double back = human_delay(rand, 140, 1.4, 90, 260);
        reach(rand, points, ox, oy, x1, y1, ms * 0.85 + human_delay(rand, 40, 1.6, 15, 120), back, 0, report_ms);
    } else
        reach(rand, points, x0, y0, x1, y1, 0, ms, bend, report_ms);
    /* a mouse at a scale of 1 reports whole pixels, never the same one twice in a row */
    GArray *whole = g_array_new(FALSE, FALSE, sizeof(HumanPoint));
    double lx = round(x0), ly = round(y0);
    for (guint i = 0; i < points->len; i++) {
        HumanPoint p = g_array_index(points, HumanPoint, i);
        gboolean last = i == points->len - 1;
        p.x = last ? x1 : round(p.x);
        p.y = last ? y1 : round(p.y);
        if (!last && ((p.x == lx && p.y == ly) || (p.x == x1 && p.y == y1)))
            continue;
        lx = p.x;
        ly = p.y;
        g_array_append_val(whole, p);
    }
    g_array_free(points, TRUE);
    return whole;
}
