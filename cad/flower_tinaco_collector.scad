// Flower-shaped rainwater collector for tinaco retrofit
\n// Parametric design for 3-12 petals, 300-600mm diameter
\n// Material: 1.5mm galvanized steel or recycled HDPE
\n
\nmodule flower_petal(radius = 300, petal_width = 50, curve_factor = 1.2) {
\n  difference() {
\n    // Petal shape with optimized water flow
\n    union() {
\n      translate([0, 0, 0])
\n      rotate_extrude(angle = 180, convexity = 10)
\n      scale([1, 1, 1.2])
\n      circle(d = radius, $fn = 100);
\n      
      // Water channel
\n      translate([0, -radius/2 + petal_width/2, 0])
\n      rotate_extrude(angle = 180, convexity = 5)
\n      circle(d = radius * 0.6, $fn = 100);
\n    }
\n    
    // Cutout for mounting
\n    translate([0, 0, 1])
\n    circle(d = radius * 0.8, $fn = 100);
\n  }
\n}
\n
\nmodule flower_collector(petals = 6, diameter = 500, height = 400) {
\n  // Base parameters
\n  petal_radius = diameter / 2;
\n  petal_width = diameter / petals / 1.5;
\n  
  // Generate petals
\n  for (i = [0:petals-1]) {
\n    rotate([0, 0, i * (360/petals)])
\n    flower_petal(radius = petal_radius, petal_width = petal_width);
\n  }
\n  
  // Central collection tube
\n  translate([0, 0, height/2])
\n  cylinder(h = height, d1 = 80, d2 = 80, $fn = 50);
\n  
  // Mounting brackets (4 points)
\n  for (i = [0:3]) {
\n    rotate([0, 0, i * 90])
\n    translate([petal_radius * 0.9, 0, -height/2 + 20])
\n    linear_extrude(height = 10)
\n    square([40, 15], center = true);
\n  }
\n}
\n
\n// Default instance
\nflower_collector(petals = 6, diameter = 500, height = 400);