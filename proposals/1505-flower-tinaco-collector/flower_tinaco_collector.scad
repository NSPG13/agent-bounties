// Flower-shaped rainwater collector for a rooftop tinaco (Mexico City)
// NSPG13/agent-bounties #1505 (continuation of #1340)
//
// Units: mm. z = 0 is the top rim of the tinaco lid neck.
// Drainage route: petals -> central funnel -> 110 mm outlet tube -> through
// the adapter socket -> into the tank. The adapter replaces the tank lid,
// clamps around the measured lid neck and carries the collector on ribs.
//
//   openscad -o exports/flower_tinaco_collector.stl flower_tinaco_collector.scad
//   openscad -o exports/petal_flat.dxf -D 'part="petal_flat"' flower_tinaco_collector.scad
//   openscad -o exports/section_a.svg  -D 'part="section_a"'  flower_tinaco_collector.scad
//   openscad -o exports/adapter.stl    -D 'part="adapter"'    flower_tinaco_collector.scad
//   python3 export.py   # all of the above + STEP + reopen checks

/* [Flower catchment] */
petal_count = 8;               // [6:1:12]
collector_diameter_mm = 1500;  // petal tip to petal tip, plan view [1000:10:2000]
petal_notch_mm = 110;          // plan depth of the notch between two petals [0:5:250]
petal_slope_deg = 15;          // petal fall toward the centre [10:1:30]
petal_overlap_mm = 30;         // seam lap between neighbouring petals [15:1:60]
sheet_mm = 3;                  // petal and funnel wall (HDPE / PP sheet) [2:0.5:6]

/* [Central funnel and outlet] */
funnel_top_d_mm = 360;         // diameter where the petals meet the funnel [240:10:600]
funnel_slope_deg = 40;         // steeper than the petals so grit keeps moving [30:1:60]
petal_lap_mm = 40;             // petal inner edge overhangs into the funnel [20:5:80]
flange_w_mm = 60;              // funnel top flange the petals bolt onto [40:5:120]
outlet_od_mm = 110;            // PVC sanitario 110 mm (4 in) [75:1:160]
outlet_wall_mm = 3.2;
drop_tube_mm = 150;            // outlet tube length below the neck rim, inside the tank
screen_d_mm = 220;             // leaf screen seated in the funnel cone
screen_slot_mm = 6;            // radial drainage slots, cut fully through
screen_slots = 24;

/* [Tinaco adapter: MEASURE ON SITE] */
neck_od_mm = 485;              // measured outside diameter of the lid neck [380:1:700]
neck_clearance_mm = 3;         // radial clearance, taken up by the clamp band [1:0.5:8]
skirt_h_mm = 60;               // skirt length down around the neck [40:5:120]
skirt_wall_mm = 6;
skirt_slots = 8;               // relief slots so the band can close the skirt
plate_t_mm = 8;
gasket_t_mm = 4;               // EPDM ring between neck rim and plate
gasket_w_mm = 18;
socket_h_mm = 60;              // socket that receives the outlet tube
socket_wall_mm = 6;
socket_clearance_mm = 0.75;    // radial; sealed with an EPDM lip ring
vent_count = 4;                // screened tank vents in the plate
vent_d_mm = 30;

/* [Supports] */
rib_count = 4;
rib_t_mm = 8;
rib_reach_mm = 520;            // radius where each rib meets the petal underside

/* [Output] */
// assembly | petals | funnel | adapter | screen | gasket | petal_flat | section_a | section_b | plan
part = "assembly";
$fn = 64;                      // kept moderate so STL/STEP stay small
flower_steps = 16;             // outline samples per petal

// ---------------------------------------------------------------- derived
R        = collector_diameter_mm / 2;
r_oo     = outlet_od_mm / 2;                    // outlet tube outside
r_oi     = r_oo - outlet_wall_mm;               // outlet tube inside
r_ft     = funnel_top_d_mm / 2;                 // funnel lip (inner surface)
r_pi     = r_ft - petal_lap_mm;                 // petal inner edge
r_fl     = r_ft + flange_w_mm;                  // funnel flange outer edge
sector   = 360 / petal_count;
tp       = tan(petal_slope_deg);
tf       = tan(funnel_slope_deg);
t_pv     = sheet_mm / cos(petal_slope_deg);     // vertical thickness of petal sheet
t_fv     = sheet_mm / cos(funnel_slope_deg);

r_neck_in  = neck_od_mm / 2 + neck_clearance_mm; // skirt inner radius
r_plate    = r_neck_in + skirt_wall_mm;          // plate / skirt outer radius
r_sock_in  = r_oo + socket_clearance_mm;
r_sock_out = r_sock_in + socket_wall_mm;

z_plate_b = gasket_t_mm;
z_plate_t = z_plate_b + plate_t_mm;
z_sock_t  = z_plate_t + socket_h_mm;
z_fb      = z_sock_t + 20;                      // funnel cone meets outlet tube
z_ft      = z_fb + (r_ft - r_oi) * tf;          // funnel lip (inner surface)

function z_fin(r)  = z_fb + (r - r_oi) * tf;              // funnel inner surface
function z_pbot(r) = z_ft + (r - r_ft) * tp;              // petal underside
function r_edge(a) = R - petal_notch_mm * pow(sin(petal_count * a / 2), 2);
function plan_pt(r, a) = [r * cos(a), r * sin(a)];
// Development of the petal cone (apex on the axis): rho = r / cos(slope),
// phi = theta * cos(slope). Arc lengths and radial lengths are preserved.
function flat_pt(r, a) = let (rho = r / cos(petal_slope_deg), phi = a * cos(petal_slope_deg))
                         [rho * cos(phi), rho * sin(phi)];
function seam_half(r) = sector / 2 + (petal_overlap_mm / 2) / r * 180 / PI;

z_tip_top = z_pbot(R) + t_pv;
r_valley  = R - petal_notch_mm;
n_out     = petal_count * flower_steps;
plan_area_m2 = let (da = 360 / n_out)
    0.5 * [for (i = [0 : n_out - 1]) pow(r_edge(i * da), 2)] * [for (i = [0 : n_out - 1]) 1]
    * (da * PI / 180) / 1e6;
funnel_area_m2 = PI * pow(r_ft / 1000, 2);
rib_h_at_reach = z_pbot(rib_reach_mm) - z_plate_t;
seam_hole_r    = [0.30, 0.55, 0.80];            // fractions of the seam length
flange_hole_r  = r_ft + flange_w_mm / 2;

// Every input, so export.py can rebuild the same solids as exact B-rep for STEP.
echo(str("DIM petal_count=", petal_count, " collector_diameter_mm=", collector_diameter_mm,
         " petal_notch_mm=", petal_notch_mm, " petal_slope_deg=", petal_slope_deg,
         " petal_overlap_mm=", petal_overlap_mm, " sheet_mm=", sheet_mm));
echo(str("DIM funnel_top_d_mm=", funnel_top_d_mm, " funnel_slope_deg=", funnel_slope_deg,
         " petal_lap_mm=", petal_lap_mm, " flange_w_mm=", flange_w_mm, " outlet_od_mm=", outlet_od_mm,
         " outlet_wall_mm=", outlet_wall_mm, " drop_tube_mm=", drop_tube_mm,
         " screen_d_mm=", screen_d_mm, " screen_slot_mm=", screen_slot_mm, " screen_slots=", screen_slots));
echo(str("DIM neck_od_mm=", neck_od_mm, " neck_clearance_mm=", neck_clearance_mm, " skirt_h_mm=", skirt_h_mm,
         " skirt_wall_mm=", skirt_wall_mm, " skirt_slots=", skirt_slots, " plate_t_mm=", plate_t_mm,
         " gasket_t_mm=", gasket_t_mm, " gasket_w_mm=", gasket_w_mm, " socket_h_mm=", socket_h_mm,
         " socket_wall_mm=", socket_wall_mm, " socket_clearance_mm=", socket_clearance_mm,
         " vent_count=", vent_count, " vent_d_mm=", vent_d_mm, " rib_count=", rib_count,
         " rib_t_mm=", rib_t_mm, " rib_reach_mm=", rib_reach_mm, " flower_steps=", flower_steps));
echo(str("DIM plan_catchment_area_m2=", plan_area_m2));
echo(str("DIM funnel_opening_area_m2=", funnel_area_m2));
echo(str("DIM tip_radius_mm=", R, " valley_radius_mm=", r_valley));
echo(str("DIM petal_inner_radius_mm=", r_pi, " funnel_lip_radius_mm=", r_ft, " flange_outer_radius_mm=", r_fl));
echo(str("DIM z_plate_bottom=", z_plate_b, " z_plate_top=", z_plate_t, " z_socket_top=", z_sock_t));
echo(str("DIM z_funnel_bottom=", z_fb, " z_funnel_lip=", z_ft, " z_tip_top=", z_tip_top));
echo(str("DIM outlet_bottom_z=", -drop_tube_mm, " overall_height_above_rim=", z_tip_top));
echo(str("DIM adapter_outer_d=", 2 * r_plate, " skirt_inner_d=", 2 * r_neck_in, " socket_inner_d=", 2 * r_sock_in));
echo(str("DIM rib_height_at_reach=", rib_h_at_reach));
echo(str("DIM petal_flat_inner_rho=", r_pi / cos(petal_slope_deg), " petal_flat_outer_rho=", R / cos(petal_slope_deg)));

assert(petal_count >= 6 && petal_count <= 12, "petal_count must be 6..12");
assert(r_valley > rib_reach_mm + 40, "ribs must end inside the petal notch radius");
assert(r_pi > r_oo + 20, "petal inner edge must stay outside the outlet tube");
assert(z_fin(r_pi) < z_pbot(r_pi) - 5, "petal lap must stay above the funnel inner surface");
assert(r_sock_out < r_neck_in - vent_d_mm - 20, "no room for vents between socket and neck");
assert(rib_reach_mm > r_plate, "ribs must reach beyond the adapter plate");
assert(outlet_od_mm < neck_od_mm - 100, "outlet must pass through the lid neck");
assert(screen_d_mm / 2 > r_oi + 10 && screen_d_mm / 2 < r_ft - 10, "screen must seat inside the funnel");

// ---------------------------------------------------------------- 2D helpers
module flower_2d() {
    polygon([for (i = [0 : n_out - 1]) plan_pt(r_edge(i * 360 / n_out), i * 360 / n_out)]);
}

// Hole centres shared by the 3D petals and the flat pattern (petal frame:
// petal centre on +x, seams at +/- sector/2).
function seam_holes() = [for (s = [-1, 1]) for (f = seam_hole_r)
    let (r = r_fl + (r_valley - 30 - r_fl) * f) [r, s * sector / 2]];
function flange_holes() = [for (a = [-sector / 4, 0, sector / 4]) [flange_hole_r, a]];
tie_hole = [R - 30, 0];

// ---------------------------------------------------------------- parts
module petals() {
    difference() {
        intersection() {
            rotate_extrude($fn = 128)
                polygon([[r_pi, z_pbot(r_pi)], [R + 20, z_pbot(R + 20)],
                         [R + 20, z_pbot(R + 20) + t_pv], [r_pi, z_pbot(r_pi) + t_pv]]);
            translate([0, 0, z_ft - 200]) linear_extrude(height = 800) flower_2d();
        }
        for (k = [0 : petal_count - 1]) rotate(k * sector) {
            for (h = concat(seam_holes(), flange_holes(), [tie_hole]))
                translate([h[0] * cos(h[1]), h[0] * sin(h[1]), 0])
                    cylinder(d = h == tie_hole ? 10 : 6.5, h = 2000, center = true, $fn = 8);
        }
    }
}

module funnel() {
    difference() {
        union() {
            // cone wall: inner surface from outlet ID up to the lip
            rotate_extrude()
                polygon([[r_oi, z_fb], [r_ft, z_ft], [r_ft + sheet_mm / sin(funnel_slope_deg), z_ft],
                         [r_oo, z_fb - (r_oo - r_oi) / tf], [r_oo, z_fb]]);
            // sloped flange under the petals (same slope as the petals)
            rotate_extrude()
                polygon([[r_ft, z_ft - t_pv], [r_fl, z_pbot(r_fl) - t_pv],
                         [r_fl, z_pbot(r_fl) + t_pv / 2], [r_ft, z_ft + t_pv / 2]]);
            // outlet tube
            translate([0, 0, -drop_tube_mm])
                difference() {
                    cylinder(r = r_oo, h = z_fb + drop_tube_mm);
                    translate([0, 0, -1]) cylinder(r = r_oi, h = z_fb + drop_tube_mm + 2);
                }
        }
        for (k = [0 : petal_count - 1]) rotate(k * sector)
            for (h = flange_holes())
                translate([h[0] * cos(h[1]), h[0] * sin(h[1]), 0])
                    cylinder(d = 6.5, h = 2000, center = true, $fn = 8);
    }
}

module screen() {
    rs = screen_d_mm / 2;
    translate([0, 0, z_fin(rs) - 1])
        difference() {
            cylinder(r = rs, h = sheet_mm);
            for (k = [0 : screen_slots - 1]) rotate(k * 360 / screen_slots)
                translate([25, -screen_slot_mm / 2, -1])
                    cube([rs - 25 - 12, screen_slot_mm, sheet_mm + 2]);
            translate([0, 0, -1]) cylinder(r = 15, h = sheet_mm + 2);
        }
}

module rib_2d() {
    // (r, z) profile; top edge follows the funnel inner surface and petal
    // underside so no rib enters the wetted volume.
    n = 12;
    polygon(concat(
        [[r_sock_out - 1, z_plate_t - 1], [r_plate, z_plate_t - 1],
         [rib_reach_mm, z_pbot(rib_reach_mm) + t_pv * 0.4]],
        [for (i = [1 : n]) let (r = rib_reach_mm - (rib_reach_mm - r_ft) * i / n) [r, z_pbot(r) + t_pv * 0.4]],
        [for (i = [1 : n]) let (r = r_ft - (r_ft - (r_oo + 4)) * i / n) [r, z_fin(r) - t_fv / 2]],
        [[r_oo + 4, z_sock_t + 4], [r_sock_out - 1, z_sock_t]]
    ));
}

module adapter() {
    difference() {
        union() {
            // lid plate
            translate([0, 0, z_plate_b]) cylinder(r = r_plate, h = plate_t_mm);
            // skirt around the measured neck
            translate([0, 0, z_plate_b - skirt_h_mm])
                difference() {
                    cylinder(r = r_plate, h = skirt_h_mm);
                    translate([0, 0, -1]) cylinder(r = r_neck_in, h = skirt_h_mm + 2);
                    // clamp band groove
                    translate([0, 0, skirt_h_mm * 0.35])
                        difference() {
                            cylinder(r = r_plate + 1, h = 16);
                            cylinder(r = r_plate - 1.5, h = 16);
                        }
                }
            // outlet socket
            translate([0, 0, z_plate_t]) cylinder(r = r_sock_out, h = socket_h_mm);
            // support ribs
            for (k = [0 : rib_count - 1]) rotate(k * 360 / rib_count)
                rotate([90, 0, 0]) linear_extrude(height = rib_t_mm, center = true) rib_2d();
        }
        // bore for the outlet tube
        translate([0, 0, z_plate_b - 1]) cylinder(r = r_sock_in, h = socket_h_mm + plate_t_mm + 2);
        // skirt relief slots
        for (k = [0 : skirt_slots - 1]) rotate(k * 360 / skirt_slots + 180 / skirt_slots)
            translate([r_neck_in - 1, -1.5, z_plate_b - skirt_h_mm - 1])
                cube([skirt_wall_mm + 2, 3, skirt_h_mm * 0.7 + 1]);
        // screened tank vents, between the ribs
        for (k = [0 : vent_count - 1]) rotate(k * 360 / vent_count + 180 / vent_count)
            translate([(r_sock_out + r_neck_in - 10) / 2, 0, 0])
                cylinder(d = vent_d_mm, h = 100, center = true, $fn = 32);
    }
}

module gasket() {
    difference() {
        cylinder(r = neck_od_mm / 2, h = gasket_t_mm);
        translate([0, 0, -1]) cylinder(r = neck_od_mm / 2 - gasket_w_mm, h = gasket_t_mm + 2);
    }
}

module assembly() {
    color("SkyBlue") petals();
    color("SteelBlue") funnel();
    color("Silver") screen();
    color("DarkOrange") adapter();
    color("Black") gasket();
}

module petal_flat() {
    // One developed petal including the seam lap on both sides.
    ns = 48;
    a_in = seam_half(r_pi);
    difference() {
        polygon(concat(
            [for (i = [0 : ns]) let (a = -a_in + 2 * a_in * i / ns) flat_pt(r_pi, a)],
            [for (i = [0 : ns]) let (r = r_pi + (r_edge(seam_half(R)) - r_pi) * i / ns) flat_pt(r, seam_half(r))],
            [for (i = [0 : 4 * ns]) let (a = seam_half(R) - 2 * seam_half(R) * i / (4 * ns)) flat_pt(r_edge(a), a)],
            [for (i = [0 : ns]) let (r = r_edge(seam_half(R)) - (r_edge(seam_half(R)) - r_pi) * i / ns) flat_pt(r, -seam_half(r))]
        ));
        for (h = concat(seam_holes(), flange_holes()))
            translate(flat_pt(h[0], h[1])) circle(d = 6.5, $fn = 16);
        translate(flat_pt(tie_hole[0], tie_hole[1])) circle(d = 10, $fn = 16);
    }
}

// Vertical sections. A-A: through a petal tip and a vent, between ribs
// (drainage route). B-B: through a petal tip and two support ribs.
module section(angle) {
    projection(cut = true) rotate([-90, 0, 0]) rotate(-angle) assembly();
}

if (part == "petals") petals();
else if (part == "funnel") funnel();
else if (part == "adapter") adapter();
else if (part == "screen") screen();
else if (part == "gasket") gasket();
else if (part == "petal_flat") petal_flat();
else if (part == "section" || part == "section_a") section(180 / vent_count);
else if (part == "section_b") section(0);
else if (part == "plan") projection() assembly();
else assembly();
