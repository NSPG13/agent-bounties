+// Flower-shaped rainwater collector for a rooftop tinaco lid.
+// NSPG13/agent-bounties#1505 (continuation of #1340)
+//
+//   openscad -o flower_tinaco_collector.stl flower_tinaco_collector.scad
+//   openscad -o petal_pattern.dxf -D export_mode='"petal_dxf"' flower_tinaco_collector.scad
+//   openscad -o adapter_ring.stl -D export_mode='"adapter"' -D lid_neck_mm=450 flower_tinaco_collector.scad
+
+/* [Catchment] */
+petal_count = 8;                 // [6:1:12]
+collector_diameter_mm = 1500;    // [1000:10:2000]
+throat_diameter_mm = 120;        // [100:1:200]
+petal_slope_deg = 18;            // [12:1:30]
+petal_overlap_mm = 15;           // [0:1:30]
+material_thickness_mm = 2.0;     // [1.2:0.1:4]
+
+/* [Adapter / lid] */
+lid_neck_mm = 600;               // 450 or 600
+adapter_outer_mm = 620;
+adapter_height_mm = 40;
+gasket_thickness_mm = 4;
+
+/* [Ports] */
+overflow_port_mm = 25;
+overflow_height_frac = 0.80;
+funnel_slope_deg = 30;
+
+/* [Export] */
+export_mode = "assembly";        // "assembly" | "petal_dxf" | "adapter"
+cutaway = false;
+
+$fn = 48;
+
+outer_r = collector_diameter_mm / 2;
+throat_r = throat_diameter_mm / 2;
+funnel_top_r = throat_diameter_mm;           // cone 2*throat_d -> throat_d
+radial_span = outer_r - funnel_top_r;
+dish_h = radial_span * tan(petal_slope_deg);
+funnel_h = (funnel_top_r - throat_r) * tan(funnel_slope_deg);
+sector_deg = 360 / petal_count;
+overlap_deg = petal_overlap_mm / outer_r * 180 / PI;
+
+capture_area_m2 = PI * pow(outer_r / 1000, 2);
+peak_L_per_h_50mm = capture_area_m2 * 50;
+storm20_L = capture_area_m2 * 20;
+first2mm_L = capture_area_m2 * 2;
+
+echo("capture_area_m2", capture_area_m2);
+echo("peak_L_per_h_50mm", peak_L_per_h_50mm);
+echo("storm20_L", storm20_L);
+echo("first2mm_L", first2mm_L);
+echo("dish_h_mm", dish_h);
+echo("funnel_h_mm", funnel_h);
+
+assert(petal_count >= 6 && petal_count <= 12, "petal_count 6–12");
+assert(lid_neck_mm == 450 || lid_neck_mm == 600, "lid_neck_mm must be 450 or 600");
+assert(adapter_outer_mm > lid_neck_mm, "adapter flange must clear the lid neck");
+assert(throat_diameter_mm < lid_neck_mm, "throat must pass through the lid opening");
+
+module petal_lobe_2d() {
+    half = sector_deg / 2 + overlap_deg / 2;
+    hull() {
+        translate([funnel_top_r * 0.25, 0])
+            circle(r = funnel_top_r * 0.4);
+        rotate(half * 0.82)
+            translate([outer_r * 0.76, 0])
+                circle(r = outer_r * 0.20);
+        rotate(-half * 0.82)
+            translate([outer_r * 0.76, 0])
+                circle(r = outer_r * 0.20);
+        translate([outer_r * 0.88, 0])
+            circle(r = outer_r * 0.16);
+    }
+}
+
+module flower_outline() {
+    for (i = [0 : petal_count - 1])
+        rotate(i * sector_deg)
+            petal_lobe_2d();
+}
+
+module cone_shell(h, r_bottom, r_top, wall) {
+    difference() {
+        cylinder(h = h, r1 = r_bottom + wall, r2 = r_top + wall);
+        translate([0, 0, -0.05])
+            cylinder(h = h + 0.1, r1 = r_bottom, r2 = r_top);
+    }
+}
+
+module dish() {
+    difference() {
+        intersection() {
+            cone_shell(dish_h, funnel_top_r, outer_r, material_thickness_mm);
+            translate([0, 0, -1])
+                linear_extrude(height = dish_h + 2)
+                    flower_outline();
+        }
+        // overflow ports, 80% up the dish, opposite petals
+        for (a = [0, 180])
+            rotate([0, 0, a])
+                translate([funnel_top_r + (outer_r - funnel_top_r) * 0.35,
+                           0,
+                           dish_h * overflow_height_frac])
+                    rotate([0, 90, 0])
+                        cylinder(h = outer_r, d = overflow_port_mm);
+    }
+}
+
+module funnel() {
+    translate([0, 0, -funnel_h])
+        cone_shell(funnel_h, throat_r, funnel_top_r, material_thickness_mm);
+}
+
+module mesh_dome() {
+    translate([0, 0, -funnel_h])
+        difference() {
+            sphere(r = throat_r + material_thickness_mm);
+            sphere(r = throat_r);
+            translate([0, 0, -throat_r - 1])
+                cube(throat_r * 4, center = true);
+        }
+}
+
+module adapter_ring() {
+    difference() {
+        union() {
+            cylinder(h = adapter_height_mm, d = adapter_outer_mm);
+            // gasket land under the flange
+            translate([0, 0, -gasket_thickness_mm])
+                cylinder(h = gasket_thickness_mm,
+                         d = (adapter_outer_mm + lid_neck_mm) / 2);
+        }
+        translate([0, 0, -gasket_thickness_mm - 1])
+            cylinder(h = adapter_height_mm + gasket_thickness_mm + 2,
+                     d = lid_neck_mm);
+        // four clamp-band windows on the collar
+        for (a = [0 : 90 : 270])
+            rotate([0, 0, a])
+                translate([lid_neck_mm / 2 - 1, 0, adapter_height_mm * 0.45])
+                    cube([8, 18, 10], center = true);
+    }
+}
+
+module strap_lug() {
+    translate([outer_r - 18, 0, dish_h - 6])
+        difference() {
+            hull() {
+                cube([24, 18, 8], center = true);
+                translate([10, 0, 0])
+                    cylinder(h = 8, d = 18, center = true);
+            }
+            translate([10, 0, 0])
+                cylinder(h = 12, d = 8, center = true);
+        }
+}
+
+module seam_bolts_3d() {
+    // visual only: M6 holes along one radial lap, repeated per petal
+    for (i = [0 : petal_count - 1])
+        rotate([0, 0, i * sector_deg + sector_deg / 2])
+            for (f = [0.25, 0.45, 0.65, 0.85]) {
+                z = dish_h * f;
+                r = funnel_top_r + (outer_r - funnel_top_r) * f;
+                translate([r, 0, z])
+                    rotate([90, 0, 0])
+                        cylinder(h = petal_overlap_mm + 8, d = 6.5, center = true);
+            }
+}
+
+module developed_petal_2d() {
+    r_in = funnel_top_r;
+    r_out = r_in + radial_span / cos(petal_slope_deg);
+    half = sector_deg / 2 + overlap_deg / 2;
+    difference() {
+        hull() {
+            intersection() {
+                difference() {
+                    circle(r = r_out);
+                    circle(r = max(r_in - 1, 1));
+                }
+                polygon([
+                    [0, 0],
+                    [r_out * 2 * cos(-half), r_out * 2 * sin(-half)],
+                    [r_out * 2 * cos( half), r_out * 2 * sin( half)]
+                ]);
+            }
+            translate([r_out * 0.97, 0])
+                circle(r = (r_out - r_in) * 0.10);
+        }
+        circle(r = r_in);
+        // seam bolt holes, 4 per long edge
+        for (side = [-1, 1])
+            for (f = [0.25, 0.45, 0.65, 0.85]) {
+                rr = r_in + (r_out - r_in) * f;
+                ang = side * (half - 8 / rr * 180 / PI);
+                translate([rr * cos(ang), rr * sin(ang)])
+                    circle(d = 6.5);
+            }
+        // inner bolt circle to the funnel cone
+        for (a = [-half * 0.4, 0, half * 0.4])
+            translate([(r_in + 18) * cos(a), (r_in + 18) * sin(a)])
+                circle(d = 6.5);
+    }
+}
+
+module assembly() {
+    difference() {
+        union() {
+            dish();
+            funnel();
+            mesh_dome();
+            translate([0, 0, -funnel_h - adapter_height_mm])
+                adapter_ring();
+            for (i = [0 : 3])
+                rotate([0, 0, i * 90 + 45])
+                    strap_lug();
+        }
+        seam_bolts_3d();
+        if (cutaway)
+            translate([0, -outer_r, -funnel_h - 80])
+                cube([outer_r + 40, outer_r * 2, dish_h + funnel_h + 160]);
+    }
+}
+
+if (export_mode == "petal_dxf") {
+    developed_petal_2d();
+} else if (export_mode == "adapter") {
+    adapter_ring();
+} else {
+    assembly();
+}
