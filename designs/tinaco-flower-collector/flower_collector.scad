// flower_collector.scad
// Parametric CAD concept: flower-shaped rainwater collector that retrofits
// onto the lid opening of a Mexican "tinaco" (Rotoplas-style water tank).
//
// Render to STL with OpenSCAD CLI, e.g.:
//   openscad -o flower_collector.stl flower_collector.scad
// Example for a 2500 L tinaco (approx. 1.5 m diameter, 590 mm lid):
//   openscad -D tinaco_lid_diameter=590 -D tinaco_radius=0.75 -D petal_count=8 \
//            -o collect-2500L.stl flower_collector.scad
//
// All dimensions in millimetres, angles in degrees. `$fn` is set high so the
// bowl renders as smooth curves; pass -D '$fn=64' on the CLI to lower it.

// ---------------------------------------------------------------------------
// Parameters (edit here or override with -D)
// ---------------------------------------------------------------------------

// Diameter of the existing tinaco lid opening the bowl retrofits onto.
tinaco_lid_diameter = 470; // 1100 L Rotoplas tinaco lid (mm)

// Outer radius of the collector bowl from centre to petal tip.
flower_radius = 300;

// Outer radius of the collector bowl at the petal notches (mm).
flower_niche_radius = 210;

// Number of petals ("flower" lobes) around the rim.
petal_count = 8;

// Depth of the bowl at the centre below the rim lip (mm). Creates the slope
// that drains captured water towards the central outlet.
bowl_depth = 60;

// Wall thickness of the bowl and flange (mm).
wall_thickness = 3;

// Height of the outer rim lip above the inner bowl floor near the edge (mm).
rim_lip = 12;

// Height of the skirt that sits inside the tinaco lid opening (mm).
flange_depth = 45;

// Diameter of the central outlet stub that pokes through the lid (mm).
outlet_diameter = 30;

// Wall thickness of the outlet tube (mm).
outlet_wall = 2.5;

// Height of the outlet stub below the bowl above the flange (mm).
outlet_height = 25;

// ---------------------------------------------------------------------------
// Geometry helpers
// ---------------------------------------------------------------------------

fn = 96;

// Point of a petal outline at angle `a` (radians). Radius interpolates
// between the niche and the tip using a raised-cosine so every petal blends
// smoothly into the next.
function petal_radius(a, n, tip, niche) =
    (tip + niche) / 2 + (tip - niche) / 2 * cos(n * a);

module petal_rim(tip, niche, n, thickness) {
    // 2D petaled polygon, one petal per `n` lobes, spanning 0..360 deg.
    steps = max(64, n * 24);
    pts = [ for (i = [0:steps]) let (a = 360 * i / steps)
        petal_radius(a, n, tip, niche) * [cos(a), sin(a)] ];
    polygon(points = concat([ [0, 0] ], pts));
}

// ---------------------------------------------------------------------------
// Collector body
// ---------------------------------------------------------------------------

module flower_collector() {
    difference() {
        union() {
            // Outer rim lip: a thin annulus at the petal outline.
            linear_extrude(height = rim_lip, convexity = 10)
                petal_rim(flower_radius, flower_niche_radius, petal_count, wall_thickness);

            // Bowl shell: petaled rim profile extruded by wall_thickness,
            // with the inner floor dropped by bowl_depth to form the drain
            // slope toward the centre.
            translate([0, 0, -bowl_depth])
                linear_extrude(height = bowl_depth, scale = [1, 1] * 0.45, convexity = 10)
                    petal_rim(flower_radius, flower_niche_radius, petal_count, wall_thickness);

            // Centre outlet tube running down through the lid.
            translate([0, 0, -flange_depth])
                cylinder(h = flange_depth + bowl_depth + 2, r = outlet_diameter / 2, $fn = fn);
        }

        // Hollow out the bowl, wall_thickness thick, and drill the outlet bore.
        translate([0, 0, -bowl_depth - 1])
            linear_extrude(height = bowl_depth + rim_lip + 2, scale = [1, 1] * 0.45, convexity = 10)
                petal_rim(flower_radius - wall_thickness, flower_niche_radius - wall_thickness,
                          petal_count, wall_thickness);

        // Inner bore of the outlet.
        translate([0, 0, -flange_depth - 1])
            cylinder(h = flange_depth + bowl_depth + 4, r = (outlet_diameter - 2 * outlet_wall) / 2, $fn = fn);
    }

    // Rigid base collars at top and bottom of the outlet to grab the lid.
    translate([0, 0, -flange_depth + 15])
        cylinder(h = 8, r = outlet_diameter / 2 + wall_thickness + 2, $fn = fn);
}

flower_collector();
