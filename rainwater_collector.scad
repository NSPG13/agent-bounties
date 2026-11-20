// Parametric Flower-Shaped Rainwater Collector
// Designed for retrofitting onto a Tinaco (water tank).

/* [Tank Interface] */
// Diameter of the tank opening (mm)
tank_opening_diameter = 400; 
// Thickness of the funnel wall (mm)
wall_thickness = 4;

/* [Flower Geometry] */
// Number of petals
petal_count = 6;
// Length of each petal from the funnel (mm)
petal_length = 200;
// Width of each petal at its widest point (mm)
petal_width = 120;
// Thickness of the petal (mm)
petal_thickness = 3;
// Angle of the petals relative to the horizontal plane (degrees)
petal_angle = 30;

/* [Central Funnel] */
// Diameter of the funnel top (mm)
funnel_top_diameter = 250;
// Height of the funnel above the tank rim (mm)
funnel_height = 100;

// --- Internal Settings ---
$fn = 64;

// --- Modules ---

module petal() {
    // Creates a single petal using a scaled and deformed sphere-like shape
    // to create a concave surface for water collection.
    difference() {
        // Outer shell
        scale([petal_length / 2, petal_width / 2, petal_thickness / 2])
        sphere(r = 1);
        
        // Inner hollow
        scale([(petal_length / 2) - petal_thickness, 
               (petal_width / 2) - petal_thickness, 
               0.1])
        sphere(r = 1);
        
        // Cut the petal in half to make it a shell
        translate([-petal_length/2, -petal_width/2, -petal_thickness/2])
        cube([petal_length, petal_width, petal_thickness]);
    }
}

module collector() {
    // 1. Central Funnel
    // The funnel directs water from the petals and the top area into the tank.
    difference() {
        union() {
            // Main funnel body
            cylinder(h = funnel_height, d1 = tank_opening_diameter - 20, d2 = funnel_top_diameter);
            // Flange to sit on the tank rim
            translate([0, 0, 0])
            cylinder(h = wall_thickness, d = tank_opening_diameter + 10);
        }
        
        // Hollow out the funnel
        translate([0, 0, -1])
        cylinder(h = funnel_height + 2, d1 = tank_opening_diameter - 20 - 2*wall_thickness, d2 = funnel_top_diameter - 2*wall_thickness);
        
        // Hollow out the flange
        translate([0, 0, -1])
        cylinder(h = wall_thickness + 2, d = tank_opening_diameter + 10 - 2*wall_thickness);
    }

    // 2. Petals
    // Arranged around the funnel to catch rain.
    for (i = [0 : petal_count - 1]) {
        rotate([0, 0, i * (360 / petal_count)])
        translate([funnel_top_diameter / 2, 0, funnel_height / 2])
        rotate([0, -petal_angle, 0])
        petal();
    }
}

// --- Execution ---
collector();