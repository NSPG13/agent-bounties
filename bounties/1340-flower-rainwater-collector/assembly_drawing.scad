// Assembly drawing for the flower-shaped rainwater collector.
//
// Generated from the SAME source as the part: this file includes
// flower_rainwater_collector.scad and projects it, so every view and every
// dimension label is derived from the parametric model rather than redrawn.
//
// Render (OpenSCAD):
//   openscad -D render_assembly=false -o exports/assembly_drawing.svg assembly_drawing.scad
//
// Views: SECTION A-A through a petal centre (1:4) and PLAN (1:8).
// Dimension values are read from the model's own variables; the parameter
// table in the title block is generated from the same values.
//
// String handling: OpenSCAD 2021 has no `+` operator for strings, so every
// label is built with str(...) from the rounded numeric helper r2().

include <flower_rainwater_collector.scad>

S = 0.25;      // section scale 1:4
S2 = 0.125;    // plan scale 1:8
LW = 0.5;      // line width on the sheet (mm)
TS = 5;        // dimension text size (mm)

// ------------------------------------------------------------------ helpers

function r2(v) = round(v * 100) / 100;
function mm(v, unit = " mm") = str(r2(v), unit);

module hline(x1, x2, y, w = LW) {
  translate([min(x1, x2), y - w / 2]) square([abs(x2 - x1), w]);
}

module vline(x, y1, y2, w = LW) {
  translate([x - w / 2, min(y1, y2)]) square([w, abs(y2 - y1)]);
}

module arrowhead(x, y, angle) {
  rotate([0, 0, angle]) polygon(points = [[0, 0], [4, 1.4], [4, -1.4]]);
}

// Horizontal dimension chain: extension lines, dim line, arrows, label above.
module hdim(x1, x2, y, label, ext = 0) {
  if (ext != 0) {
    vline(x1, y, y + ext, LW / 2);
    vline(x2, y, y + ext, LW / 2);
  }
  hline(x1, x2, y);
  arrowhead(min(x1, x2), y, 180);
  arrowhead(max(x1, x2), y, 0);
  translate([(x1 + x2) / 2, y + 3]) text(label, size = TS, halign = "center", valign = "bottom");
}

// Vertical dimension chain: extension lines, dim line, arrows, label left.
module vdim(y1, y2, x, label, ext = 0) {
  if (ext != 0) {
    hline(x, x + ext, y1, LW / 2);
    hline(x, x + ext, y2, LW / 2);
  }
  vline(x, y1, y2);
  arrowhead(x, min(y1, y2), 90);
  arrowhead(x, max(y1, y2), 270);
  translate([x - 3, (y1 + y2) / 2]) text(label, size = TS, halign = "right", valign = "center");
}

module leader(x1, y1, x2, y2, label) {
  vline(x1, y1, y2, LW / 2);
  hline(x1, x2, y2, LW / 2);
  translate([x2 + 2, y2]) text(label, size = TS, halign = "left", valign = "center");
}

// --------------------------------------------------------------- view layout

// SECTION A-A: cut through a petal centre, so the dish thickness, the hub
// joint, the funnel bore and the screen all appear in section.
translate([-140, 40])
  scale([S, S, S])
    projection(cut = true)
      rotate([90, 0, 0])
        rotate([0, 0, -180 / n_petals])
          collector();

// PLAN: silhouette from above (petal scallops and rim).
translate([160, 60])
  scale([S2, S2, S2])
    projection(cut = false)
      collector();

// ------------------------------------------------------------- dimensions

// Overall height (rim -> outlet), drawn to the left of the section.
translate([-140, 40]) {
  x_h = -S * (collector_radius + 25);
  y_top = S * (petal_rise + wall);
  y_out = S * -(funnel_height + port_length);
  vdim(y_out, y_top, x_h, mm(petal_rise + wall + funnel_height + port_length), ext = S * 25);
  // funnel drop and sleeve engagement, chained inside the section's left gap
  vdim(S * -funnel_height, 0, x_h + 22, mm(funnel_height), ext = S * 20);
  vdim(S * -(funnel_height + port_length), S * -funnel_height, x_h + 44, mm(port_length), ext = S * 20);
  // hub mouth diameter across the funnel throat
  hdim(S * -hub_radius, S * hub_radius, S * 45, mm(hub_dia, " mm dia"));
  // port thread diameter at the outlet
  hdim(S * -port_radius, S * port_radius, S * -(funnel_height + port_length) - 12,
       mm(port_thread_dia, " mm dia"), ext = S * 8);
  // rim-to-hub concave depth
  vdim(0, S * petal_rise, S * 95, mm(petal_rise), ext = S * 10);
  leader(S * hub_radius + S * 20, S * 1, S * 150, S * 22, str("wall ", r2(wall), " mm"));
  leader(S * 30, S * -1.2, S * 120, S * -20,
         str("screen ", r2(screen_thick), " mm, holes ", r2(screen_hole), " mm @ ",
             r2(screen_pitch), " mm"));
  translate([0, S * 30]) text("SECTION A-A  (1:4)", size = 8, halign = "center");
  translate([0, S * 30 - 10]) text("cut through a petal centre", size = 5, halign = "center");
}

// Plan dimensions.
translate([160, 60]) {
  hdim(-S2 * collector_radius, S2 * collector_radius, S2 * collector_radius + 22,
       mm(collector_dia, " mm dia"), ext = S2 * 12);
  hdim(-S2 * hub_radius, S2 * hub_radius, S2 * (hub_radius + 40),
       str("hub ", mm(hub_dia)), ext = S2 * 40);
  vdim(-S2 * collector_radius, S2 * collector_radius, -S2 * collector_radius - 22,
       mm(collector_dia), ext = S2 * 12);
  translate([0, -S2 * collector_radius - 40]) text("PLAN  (1:8)", size = 8, halign = "center");
  translate([0, -S2 * collector_radius - 50]) text(str(n_petals, " petals"), size = 5, halign = "center");
}

// ------------------------------------------------------------------ title block

module table_row(x, y, key, value) {
  translate([x, y]) text(key, size = 5, halign = "left", valign = "center");
  translate([x + 95, y]) text(value, size = 5, halign = "right", valign = "center");
}

translate([-380, -95]) {
  square([300, 185]);
  translate([5, 175]) text("FLOWER RAINWATER COLLECTOR - tinaco retrofit", size = 8, halign = "left");
  translate([5, 165]) text("concept design - dimensions nominal, not for fabrication", size = 5, halign = "left");
  translate([5, 158]) hline(0, 290, 158, 0.4);

  rows_label = ["n_petals", "collector_dia", "petal_rise", "petal_gap", "hub_dia",
                "funnel_height", "port_thread_dia", "port_length", "screen_thick",
                "screen_hole", "screen_pitch", "wall"];
  rows_value = [str(n_petals), mm(collector_dia), mm(petal_rise),
                mm(petal_gap), mm(hub_dia), mm(funnel_height),
                mm(port_thread_dia), mm(port_length),
                mm(screen_thick), mm(screen_hole), mm(screen_pitch),
                mm(wall)];
  for (i = [0 : len(rows_label) - 1])
    table_row(5, 148 - i * 8, rows_label[i], rows_value[i]);

  x2 = 155;
  translate([x2, 158]) vline(x2, 158, -1, 0.4);
  derived_label = ["overall height", "outlet below datum", "hub mouth", "rim crest",
                   "capture area", "material volume", "BOM"];
  cap_area = 3.14159265 * pow(collector_dia / 2000, 2);
  derived_value = [mm(petal_rise + wall + funnel_height + port_length),
                   mm(-(funnel_height + port_length)),
                   "z = 0", str("z = ", mm(petal_rise + wall)),
                   str(r2(cap_area), " m^2"), "see README", "1 part, see README"];
  for (i = [0 : len(derived_label) - 1])
    table_row(x2 + 5, 148 - i * 8, derived_label[i], derived_value[i]);

  translate([x2 + 5, 148 - len(derived_label) * 8 - 6])
    text("scale 1:4 section / 1:8 plan - units mm", size = 5, halign = "left");
  translate([x2 + 5, 148 - len(derived_label) * 8 - 14])
    text("source: flower_rainwater_collector.scad", size = 5, halign = "left");
}
