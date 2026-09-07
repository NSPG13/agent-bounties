// Flower-shaped rainwater collector for a tinaco (Mexico City).
// Parametric CAD concept.
//
// A shallow, flower-like funnel retrofits onto the threaded fill port of a
// rooftop tinaco. Rain falling on the concave petals drains inward through
// radial gaps to a screened central hub and down into the tank. See README.md
// for the full concept, standard tinaco sizing, materials, and verification.

/* [Petals] */
n_petals        = 8;      // number of petals (4..16)
collector_dia   = 1200;   // overall collector diameter (mm)
petal_rise      = 100;    // rim-to-hub concave depth (mm)
petal_gap       = 6;      // radial drainage channel width between petals (mm)
petal_gap_flare = 2.0;    // rim valley radius as a multiple of petal_gap

/* [Hub and funnel] */
hub_dia       = 180;      // mouth diameter of the central funnel (mm)
funnel_height = 90;       // funnel drop from hub mouth to the port (mm)

/* [Tinaco interface] */
port_thread_dia = 50.8;   // tinaco fill-port thread diameter (mm); 2" nominal
port_length     = 40;     // engagement length of the threaded sleeve (mm)

/* [Debris screen] */
screen_thick = 3;         // screen disc thickness (mm)
screen_hole  = 2.5;       // square hole side (mm), debris/mosquito filter
screen_pitch = 6;         // hole grid pitch (mm)

/* [Wall] */
wall = 2.5;               // material wall thickness (mm)

$fn = 120;

collector_radius = collector_dia / 2;
hub_radius       = hub_dia / 2;
port_radius      = port_thread_dia / 2;

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

// Central funnel from the hub mouth down to the tinaco port.
module funnel() {
  difference() {
    cylinder(r1 = hub_radius + wall, r2 = port_radius + wall, h = funnel_height);
    translate([0, 0, -1])
      cylinder(r1 = hub_radius, r2 = port_radius, h = funnel_height + 2);
  }
}

// Threaded sleeve that engages the tinaco fill port.
module port_sleeve() {
  difference() {
    cylinder(r = port_radius + wall, h = port_length);
    translate([0, 0, -1])
      cylinder(r = port_radius, h = port_length + 2);
  }
}

// Debris screen across the hub mouth.
module screen() {
  difference() {
    translate([0, 0, -1])
      cylinder(r = hub_radius - 1, h = screen_thick);
    for (x = [-hub_radius : screen_pitch : hub_radius])
      for (y = [-hub_radius : screen_pitch : hub_radius])
        if (x * x + y * y <= (hub_radius - screen_pitch) * (hub_radius - screen_pitch))
          translate([x, y, -1 - screen_thick / 2])
            cube([screen_hole, screen_hole, screen_thick + 4], center = true);
  }
}

// Full collector assembly.
module collector() {
  flower();
  funnel();
  translate([0, 0, -funnel_height])
    port_sleeve();
  screen();
}

collector();
