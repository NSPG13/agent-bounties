// Flower-shaped rainwater collector for Mexican tinaco retrofit
// Parametric design with continuous open drainage bore and adjustable adapter

// ==========================================
// PARAMETRIC CONFIGURATION (CLI OVERRIDABLE)
// ==========================================
petals = 6;                // Number of petals [3:12]
diameter = 500;            // Outer diameter in mm [300:600]
funnel_height = 120;       // Height of collection funnel in mm
tube_height = 200;         // Central downspout tube height in mm
wall_thickness = 3;        // Wall thickness in mm
tube_outer_d = 80;         // Outer diameter of collection downspout in mm
adapter_type = 1;          // Tinaco inlet adapter preset: 1 = 90mm inlet, 2 = 110mm inlet
show_cross_section = false;// Cutaway view to inspect continuous internal bore

// Calculate internal bore diameters
tube_inner_d = tube_outer_d - (2 * wall_thickness);
adapter_outer_d = (adapter_type == 2) ? 110 : 90;
adapter_inner_d = adapter_outer_d - (2 * wall_thickness);

$fn = 60;

// Individual Petal: angled concave catchment leaf guiding water inward
module petal(r, width, wall) {
    rotate([20, 0, 0])
    difference() {
        // Outer curved petal shell
        scale([width / r, 1.0, 0.4])
        cylinder(r = r, h = wall * 3, center = false);
        
        // Scoop hollow to form water drainage trough
        translate([0, 0, wall])
        scale([(width - 2 * wall) / r, 0.96, 0.4])
        cylinder(r = r, h = wall * 4, center = false);
        
        // Trim outer boundary
        translate([0, -r, -1])
        cube([r * 2, r * 2, r], center = true);
    }
}

// Complete Flower Rainwater Collector Assembly
module flower_collector(
    p_count = petals,
    p_diam = diameter,
    f_h = funnel_height,
    t_h = tube_height,
    wall = wall_thickness,
    t_out_d = tube_outer_d,
    adapt_out_d = adapter_outer_d
) {
    p_radius = p_diam / 2;
    p_width = (3.14159 * p_diam) / p_count * 0.7;
    t_in_d = t_out_d - 2 * wall;
    adapt_in_d = adapt_out_d - 2 * wall;
    funnel_top_d = p_radius * 0.9;

    difference() {
        union() {
            // 1. Central Catchment Funnel (Outer)
            translate([0, 0, t_h])
            cylinder(h = f_h, d1 = t_out_d, d2 = funnel_top_d, center = false);

            // 2. Petals radiating outward from top rim of funnel
            for (i = [0 : p_count - 1]) {
                rotate([0, 0, i * (360 / p_count)])
                translate([0, funnel_top_d / 2 - 5, t_h + f_h - 10])
                petal(r = p_radius * 0.65, width = p_width, wall = wall);
            }

            // 3. Central Downspout Tube (Outer)
            translate([0, 0, 50])
            cylinder(h = t_h - 50, d = t_out_d, center = false);

            // 4. Adjustable Tinaco Mounting Adapter Collar (Outer)
            translate([0, 0, 0])
            cylinder(h = 60, d1 = adapt_out_d, d2 = t_out_d, center = false);

            // 5. Four Tinaco Rim Mounting Brackets
            for (j = [0 : 3]) {
                rotate([0, 0, j * 90])
                translate([adapt_out_d / 2, -10, 10])
                cube([25, 20, 8]);
            }
        }

        // ========================================================
        // CONTINUOUS OPEN INTERNAL BORE (Water flow path from top to outlet)
        // ========================================================
        
        // Funnel hollow bore
        translate([0, 0, t_h - 1])
        cylinder(h = f_h + 2, d1 = t_in_d, d2 = funnel_top_d - (2 * wall), center = false);

        // Downspout tube hollow bore
        translate([0, 0, 49])
        cylinder(h = t_h - 48, d = t_in_d, center = false);

        // Tinaco adapter hollow bore and outlet
        translate([0, 0, -2])
        cylinder(h = 64, d1 = adapt_in_d, d2 = t_in_d, center = false);

        // Mounting bolt holes (8mm) in the 4 mounting brackets
        for (j = [0 : 3]) {
            rotate([0, 0, j * 90])
            translate([adapt_out_d / 2 + 15, 0, 5])
            cylinder(h = 20, d = 8, center = true);
        }

        // Optional cutaway cross-section for inspection
        if (show_cross_section) {
            translate([0, -p_radius, -10])
            cube([p_radius * 2, p_radius * 2, t_h + f_h + 50]);
        }
    }
}

// Instantiate collector with CLI-overridable top-level variables
flower_collector(
    p_count = petals,
    p_diam = diameter,
    f_h = funnel_height,
    t_h = tube_height,
    wall = wall_thickness,
    t_out_d = tube_outer_d,
    adapt_out_d = adapter_outer_d
);