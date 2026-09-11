// Flower-shaped rainwater collector for a tinaco (Mexico City).
// Parametric CAD concept.
//
// A shallow, flower-like funnel retrofits onto the threaded fill port of a
// rooftop tinaco. Rain falling on the concave petals drains inward through
// radial gaps to a screened central hub and down into the tank. See README.md
// for the full concept, standard tinaco sizing, materials, and verification.
//
// DATUM: z = 0 is the hub mouth plane, i.e. the top of the mounting datum on
// the tank's fill port. The concave dish lives ABOVE z = 0 (hub mouth at z = 0,
// rim at z = petal_rise); the funnel drops BELOW z = 0 to the port (outlet at
// z = -funnel_height) and the threaded sleeve continues below that to
// z = -(funnel_height + port_length). Every part therefore overlaps its
// neighbour in a real volume, so the assembly is ONE closed solid.

/* [Petals] */
n_petals        = 8;      // number of petals (4..16)
collector_dia   = 1200;   // overall collector diameter (mm)
petal_rise      = 100;    // rim-to-hub concave depth (mm)
petal_gap       = 6;      // radial drainage channel width between petals (mm)
petal_gap_flare = 2.0;    // rim valley radius as a multiple of petal_gap

/* [Hub and funnel] */
hub_dia       = 180;      // mouth diameter of the central funnel (mm)
funnel_height = 90;       // funnel drop from the hub mouth down to the port (mm)

/* [Tinaco interface] */
port_thread_dia = 50.8;   // tinaco fill-port thread diameter (mm); 2" nominal
port_length     = 40;     // engagement length of the threaded sleeve (mm)

/* [Debris screen] */
screen_thick = 3;         // screen disc thickness (mm)
screen_hole  = 2.5;       // square hole side (mm), debris/mosquito filter
screen_pitch = 6;         // hole grid pitch (mm)

/* [Wall] */
wall = 2.5;               // material wall thickness (mm)

/* [Render] */
render_assembly = true;   // render the assembly when this file is run directly

$fn = 120;

collector_radius = collector_dia / 2;
hub_radius       = hub_dia / 2;
port_radius      = port_thread_dia / 2;

// Every joint is fused by a real volumetric overlap instead of a coincident
// face, so booleans cannot open a seam: the funnel lip rises `fuse` above
// z = 0 into the dish shell, and the screen rim sinks `fuse` into the funnel
// wall. mesh_checks.py measures the result (one watertight body).
fuse = wall;

// Concave dish: rim high, hub low, so water always runs inward.
module dish() {
  rotate_extrude(angle = 360)
    polygon(points = [
      [hub_radius, 0],
      [collector_radius, petal_rise],
      [collector_radius, petal_rise + wall],
      [hub_radius, wall],
    ]);
}

// Radial channels split the dish into discrete petals.
module petal_channels() {
  for (i = [0 : n_petals - 1])
    rotate([0, 0, i * 360 / n_petals])
      translate([hub_radius, -petal_gap / 2, -1])
        cube([collector_radius - hub_radius + 1, petal_gap, petal_rise + wall + 2]);
}

// Round the rim valleys for a scalloped, flower-like outer edge.
module rim_scallops() {
  for (i = [0 : n_petals - 1])
    rotate([0, 0, i * 360 / n_petals])
      translate([collector_radius, 0, petal_rise / 2])
        cylinder(r = petal_gap * petal_gap_flare, h = petal_rise + wall + 2, center = true);
}

// The petal flower: concave dish segmented into petals.
module flower() {
  difference() {
    dish();
    petal_channels();
    rim_scallops();
  }
}

// Central funnel: wide rim at the hub mouth (z = 0, radius hub_radius + wall)
// narrowing DOWNWARD to the port (z = -funnel_height, radius port_radius +
// wall). The cone rises `fuse` above z = 0 and its bore opens to hub_radius at
// that height, so the mouth is buried inside the dish shell (welding the
// petals to the funnel) and the screen nests in the surrounding material.
module funnel() {
  difference() {
    translate([0, 0, -funnel_height])
      cylinder(r1 = port_radius + wall, r2 = hub_radius + wall, h = funnel_height + fuse);
    translate([0, 0, -funnel_height])
      cylinder(r1 = port_radius, r2 = hub_radius, h = funnel_height + fuse);
  }
}

// Threaded sleeve that engages the tinaco fill port. It continues the funnel
// bore downward from the funnel outlet, so the water path is continuous.
module port_sleeve() {
  difference() {
    translate([0, 0, -funnel_height - port_length])
      cylinder(r = port_radius + wall, h = port_length + fuse);
    translate([0, 0, -funnel_height - port_length - 1])
      cylinder(r = port_radius, h = port_length + fuse + 2);
  }
}

// Debris screen across the hub mouth. It seats in the parallel mouth wall, so
// it is part of the same solid, and the holes are cut clear through the disc
// with margin on both faces (a hole that only pierces the underside would
// leave the collector watertight but blocked - the geometry test checks this).
module screen() {
  difference() {
    cylinder(r = hub_radius + fuse / 2, h = screen_thick);
    for (x = [-hub_radius : screen_pitch : hub_radius])
      for (y = [-hub_radius : screen_pitch : hub_radius])
        if (x * x + y * y <= (hub_radius - screen_pitch) * (hub_radius - screen_pitch))
          translate([x - screen_hole / 2, y - screen_hole / 2, -1])
            cube([screen_hole, screen_hole, screen_thick + 2], center = false);
  }
}

// Full collector assembly: one closed solid (dish + funnel + sleeve + screen).
module collector() {
  flower();
  funnel();
  port_sleeve();
  screen();
}

if (render_assembly) collector();
