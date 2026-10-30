/*
 * Flower-shaped rainwater collector and adjustable tinaco adapter.
 * Parametric CAD concept retrofitting onto standard rooftop water tanks in Mexico City.
 */

n_petals = 8;
collector_dia = 1200.0;
petal_rise = 100.0;
petal_gap = 6.0;
petal_gap_flare = 2.0;

hub_dia = 180.0;
funnel_height = 90.0;
outlet_dia = 60.0;

adapter_collar_min_dia = 400.0;
adapter_collar_max_dia = 650.0;
adapter_height = 120.0;

screen_thick = 3.0;
screen_hole = 2.5;
screen_pitch = 6.0;

wall = 3.0;

$fn = 120;

collector_radius = collector_dia / 2;
hub_radius = hub_dia / 2;
outlet_radius = outlet_dia / 2;

module concave_dish() {
  rotate_extrude(angle = 360)
    polygon(points = [
      [hub_radius, 0],
      [collector_radius, petal_rise],
      [collector_radius, petal_rise + wall],
      [hub_radius, wall]
    ]);
}

module petal_drainage_channels() {
  for (i = [0 : n_petals - 1])
    rotate([0, 0, i * 360 / n_petals])
      translate([hub_radius, -petal_gap / 2, -1])
        cube([collector_radius - hub_radius + 2, petal_gap, petal_rise + wall + 2]);
}

module outer_rim_scallops() {
  for (i = [0 : n_petals - 1])
    rotate([0, 0, i * 360 / n_petals])
      translate([collector_radius, 0, petal_rise / 2])
        cylinder(r = petal_gap * petal_gap_flare, h = petal_rise + wall + 4, center = true);
}

module flower_collector() {
  difference() {
    concave_dish();
    petal_drainage_channels();
    outer_rim_scallops();
  }
}

module central_funnel() {
  difference() {
    cylinder(r1 = hub_radius + wall, r2 = outlet_radius + wall, h = funnel_height);
    translate([0, 0, -1])
      cylinder(r1 = hub_radius, r2 = outlet_radius, h = funnel_height + 2);
  }
}

module debris_screen() {
  difference() {
    translate([0, 0, -1])
      cylinder(r = hub_radius - 0.5, h = screen_thick);
    for (x = [-hub_radius : screen_pitch : hub_radius])
      for (y = [-hub_radius : screen_pitch : hub_radius])
        if (x * x + y * y <= (hub_radius - screen_pitch) * (hub_radius - screen_pitch))
          translate([x, y, -1 - screen_thick / 2])
            cube([screen_hole, screen_hole, screen_thick + 4], center = true);
  }
}

module adjustable_tinaco_adapter() {
  difference() {
    union() {
      cylinder(r = outlet_radius + wall + 4, h = 40);
      translate([0, 0, -60])
        cylinder(r1 = 300, r2 = outlet_radius + wall + 4, h = 60);
      translate([0, 0, -80])
        cylinder(r = 310, h = 20);
    }
    translate([0, 0, -82])
      cylinder(r = outlet_radius, h = 130);
    translate([0, 0, -82])
      cylinder(r = 225, h = 15);
    translate([0, 0, -82])
      cylinder(r = 250, h = 10);
    translate([0, 0, -82])
      cylinder(r = 275, h = 5);
    for (i = [0 : 5])
      rotate([0, 0, i * 60])
        translate([200, -5, -82])
          cube([110, 10, 25]);
  }
}

module complete_assembly() {
  flower_collector();
  debris_screen();
  translate([0, 0, -funnel_height]) {
    central_funnel();
    translate([0, 0, -40])
      adjustable_tinaco_adapter();
  }
}

complete_assembly();
