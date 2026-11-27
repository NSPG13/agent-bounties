"""
Parametric CAD generator for the flower-shaped rainwater collector and adjustable tinaco adapter.
Exports valid ISO 10303-21 STEP files, binary STL meshes, and dimensioned SVG assembly drawings.
"""

from pathlib import Path
import math
import build123d as bd


def create_flower_collector(
    collector_dia: float = 1200.0,
    hub_dia: float = 180.0,
    outlet_dia: float = 60.0,
    petal_rise: float = 100.0,
    funnel_height: float = 90.0,
    n_petals: int = 8,
    petal_gap: float = 6.0,
    wall: float = 3.0,
) -> bd.Part:
    """
    Construct the 3D solid geometry of the flower rainwater collector.

    Parameters:
        collector_dia: Outer diameter of the flower dish in millimeters.
        hub_dia: Diameter of the central catchment hub in millimeters.
        outlet_dia: Outer diameter of the bottom drain outlet in millimeters.
        petal_rise: Height difference from hub to outer petal rim in millimeters.
        funnel_height: Vertical drop from hub to outlet in millimeters.
        n_petals: Number of flower petals.
        petal_gap: Width of radial drainage channel between petals in millimeters.
        wall: Shell wall thickness in millimeters.

    Returns:
        Solid Part representing the complete flower collector.
    """
    collector_r = collector_dia / 2.0
    hub_r = hub_dia / 2.0
    outlet_r = outlet_dia / 2.0

    with bd.BuildPart() as part:
        with bd.BuildSketch(bd.Plane.XZ):
            with bd.BuildLine():
                bd.Line((hub_r, 0), (collector_r, petal_rise))
                bd.Line((collector_r, petal_rise), (collector_r, petal_rise + wall))
                bd.Line((collector_r, petal_rise + wall), (hub_r, wall))
                bd.Line((hub_r, wall), (hub_r, 0))
            bd.make_face()
        bd.revolve(axis=bd.Axis.Z)

        with bd.BuildSketch(bd.Plane.XZ):
            with bd.BuildLine():
                bd.Line((hub_r + wall, 0), (outlet_r + wall, -funnel_height))
                bd.Line((outlet_r + wall, -funnel_height), (outlet_r, -funnel_height))
                bd.Line((outlet_r, -funnel_height), (hub_r, 0))
                bd.Line((hub_r, 0), (hub_r + wall, 0))
            bd.make_face()
        bd.revolve(axis=bd.Axis.Z)

        channel_length = collector_r - hub_r + 20.0
        center_channel_x = hub_r + channel_length / 2.0

        for index in range(n_petals):
            angle = index * 360.0 / float(n_petals)
            with bd.BuildPart(mode=bd.Mode.SUBTRACT):
                with bd.Locations(bd.Rotation(0, 0, angle)):
                    with bd.Locations(bd.Location((center_channel_x, 0, petal_rise / 2.0))):
                        bd.Box(channel_length, petal_gap, petal_rise + wall + 20.0)
                    with bd.Locations(bd.Location((collector_r, 0, petal_rise / 2.0))):
                        bd.Cylinder(radius=petal_gap * 2.0, height=petal_rise + wall + 20.0)

    return part.part


def create_tinaco_adapter(
    outlet_dia: float = 60.0,
    adapter_collar_min_dia: float = 400.0,
    adapter_collar_max_dia: float = 650.0,
    adapter_height: float = 120.0,
    wall: float = 4.0,
) -> bd.Part:
    """
    Construct the 3D solid geometry of the dimensioned adjustable tinaco adapter.

    Parameters:
        outlet_dia: Inner receiver diameter matching the collector funnel.
        adapter_collar_min_dia: Minimum tinaco neck diameter accommodated.
        adapter_collar_max_dia: Maximum tinaco rim flange diameter accommodated.
        adapter_height: Total vertical profile height of the adapter.
        wall: Structural wall thickness in millimeters.

    Returns:
        Solid Part representing the adjustable tinaco adapter collar.
    """
    receiver_r = outlet_dia / 2.0
    rim_min_r = adapter_collar_min_dia / 2.0
    rim_max_r = adapter_collar_max_dia / 2.0

    with bd.BuildPart() as part:
        with bd.BuildSketch(bd.Plane.XZ):
            with bd.BuildLine():
                bd.Line((receiver_r, 0), (receiver_r + wall + 2.0, 0))
                bd.Line((receiver_r + wall + 2.0, 0), (receiver_r + wall + 2.0, -30))
                bd.Line((receiver_r + wall + 2.0, -30), (rim_min_r + wall, -70))
                bd.Line((rim_min_r + wall, -70), (rim_min_r + wall, -85))
                bd.Line((rim_min_r + wall, -85), (rim_max_r, -85))
                bd.Line((rim_max_r, -85), (rim_max_r, -adapter_height))
                bd.Line((rim_max_r, -adapter_height), (rim_max_r - wall, -adapter_height))
                bd.Line((rim_max_r - wall, -adapter_height), (rim_max_r - wall, -95))
                bd.Line((rim_max_r - wall, -95), (rim_min_r, -95))
                bd.Line((rim_min_r, -95), (receiver_r + wall, -35))
                bd.Line((receiver_r + wall, -35), (receiver_r, -35))
                bd.Line((receiver_r, -35), (receiver_r, 0))
            bd.make_face()
        bd.revolve(axis=bd.Axis.Z)

        slot_count = 6
        slot_length = (rim_max_r - rim_min_r) + 20.0
        slot_center_x = (rim_min_r + rim_max_r) / 2.0

        for index in range(slot_count):
            angle = index * 360.0 / float(slot_count)
            with bd.BuildPart(mode=bd.Mode.SUBTRACT):
                with bd.Locations(bd.Rotation(0, 0, angle)):
                    with bd.Locations(bd.Location((slot_center_x, 0, -90))):
                        bd.Box(slot_length, 10.0, 30.0)

    return part.part


def create_svg_assembly_drawing(output_path: Path) -> None:
    """
    Generate a dimensioned vector SVG assembly drawing of the complete system.

    Parameters:
        output_path: Target filesystem path to write the SVG drawing.
    """
    svg_content = """<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1200 850" width="100%" height="100%">
  <defs>
    <marker id="arrow" viewBox="0 0 10 10" refX="5" refY="5" markerWidth="6" markerHeight="6" orient="auto-start-reverse">
      <path d="M 0 1.5 L 10 5 L 0 8.5 z" fill="#1e293b" />
    </marker>
    <style>
      .title { font-family: system-ui, -apple-system, sans-serif; font-weight: 700; fill: #0f172a; }
      .label { font-family: system-ui, -apple-system, sans-serif; font-size: 12px; fill: #334155; }
      .dim-text { font-family: monospace; font-size: 12px; fill: #0f172a; font-weight: 600; text-anchor: middle; }
      .dim-line { stroke: #0f172a; stroke-width: 1.2; marker-start: url(#arrow); marker-end: url(#arrow); }
      .ext-line { stroke: #94a3b8; stroke-width: 0.8; stroke-dasharray: 3,3; }
      .outline { stroke: #0f172a; stroke-width: 2.2; fill: none; }
      .hatch { stroke: #cbd5e1; stroke-width: 1.0; }
      .callout { font-family: system-ui, sans-serif; font-size: 11px; font-weight: bold; fill: #ffffff; }
    </style>
  </defs>

  <rect width="1200" height="850" fill="#ffffff" stroke="#0f172a" stroke-width="2"/>
  <rect x="20" y="20" width="1160" height="810" fill="none" stroke="#64748b" stroke-width="1"/>

  <g transform="translate(740, 710)">
    <rect width="440" height="120" fill="#f8fafc" stroke="#0f172a" stroke-width="1.5"/>
    <line x1="0" y1="30" x2="440" y2="30" stroke="#0f172a" stroke-width="1"/>
    <line x1="0" y1="60" x2="440" y2="60" stroke="#0f172a" stroke-width="1"/>
    <line x1="0" y1="90" x2="440" y2="90" stroke="#0f172a" stroke-width="1"/>
    <line x1="220" y1="30" x2="220" y2="120" stroke="#0f172a" stroke-width="1"/>
    <text x="15" y="21" class="title" font-size="14">FLOWER RAINWATER COLLECTOR &amp; TINACO ADAPTER</text>
    <text x="15" y="48" class="label">Organization: s6pa1rta3n-lab</text>
    <text x="235" y="48" class="label">Bounty Target: NSPG13 #1365</text>
    <text x="15" y="78" class="label">Standard: ISO 128 / ASME Y14.5</text>
    <text x="235" y="78" class="label">System: Rotoplas / CDMX Retrofit</text>
    <text x="15" y="108" class="label">Scale: 1:10 (Dimensions in mm)</text>
    <text x="235" y="108" class="label">Drawing: SHT 1 OF 1 - REV A</text>
  </g>

  <g transform="translate(0, 50)">
    <text x="60" y="30" class="title" font-size="16">SECTION A-A: ASSEMBLY ELEVATION</text>
    
    <path d="M 100,100 L 500,190 L 500,280 L 560,280 L 560,190 L 960,100 L 960,108 L 568,198 L 568,280 L 492,280 L 492,198 L 100,108 Z" fill="#e0f2fe" class="outline"/>
    
    <rect x="500" y="188" width="60" height="6" fill="#3b82f6" stroke="#1d4ed8" stroke-width="1"/>
    <line x1="500" y1="191" x2="560" y2="191" stroke="#ffffff" stroke-width="2" stroke-dasharray="2,3"/>

    <path d="M 485,280 L 575,280 L 575,310 L 730,360 L 730,410 L 710,410 L 710,370 L 550,320 L 550,420 L 510,420 L 510,320 L 350,370 L 350,410 L 330,410 L 330,360 L 485,310 Z" fill="#f1f5f9" class="outline"/>

    <rect x="315" y="380" width="30" height="40" fill="#cbd5e1" stroke="#0f172a" stroke-width="1.2"/>
    <circle cx="330" cy="400" r="4" fill="#0f172a"/>
    <rect x="715" y="380" width="30" height="40" fill="#cbd5e1" stroke="#0f172a" stroke-width="1.2"/>
    <circle cx="730" cy="400" r="4" fill="#0f172a"/>

    <path d="M 280,480 L 330,480 L 330,410 L 320,410 L 320,470 L 280,470" fill="#fed7aa" stroke="#c2410c" stroke-width="1.5" stroke-dasharray="4,2"/>
    <path d="M 780,480 L 730,480 L 730,410 L 740,410 L 740,470 L 780,470" fill="#fed7aa" stroke="#c2410c" stroke-width="1.5" stroke-dasharray="4,2"/>

    <line x1="100" y1="70" x2="960" y2="70" class="dim-line"/>
    <line x1="100" y1="60" x2="100" y2="100" class="ext-line"/>
    <line x1="960" y1="60" x2="960" y2="100" class="ext-line"/>
    <text x="530" y="65" class="dim-text">Ø 1200 (OVERALL CATCHMENT)</text>

    <line x1="500" y1="170" x2="560" y2="170" class="dim-line"/>
    <line x1="500" y1="160" x2="500" y2="190" class="ext-line"/>
    <line x1="560" y1="160" x2="560" y2="190" class="ext-line"/>
    <text x="530" y="165" class="dim-text">Ø 180</text>

    <line x1="980" y1="100" x2="980" y2="190" class="dim-line"/>
    <line x1="960" y1="100" x2="990" y2="100" class="ext-line"/>
    <line x1="560" y1="190" x2="990" y2="190" class="ext-line"/>
    <text x="1015" y="150" class="dim-text">100 RISE</text>

    <line x1="470" y1="190" x2="470" y2="280" class="dim-line"/>
    <text x="440" y="240" class="dim-text">90 DROP</text>

    <line x1="755" y1="280" x2="755" y2="410" class="dim-line"/>
    <line x1="575" y1="280" x2="765" y2="280" class="ext-line"/>
    <line x1="730" y1="410" x2="765" y2="410" class="ext-line"/>
    <text x="795" y="350" class="dim-text">120 ADAPTER</text>

    <line x1="330" y1="440" x2="730" y2="440" class="dim-line"/>
    <line x1="330" y1="410" x2="330" y2="450" class="ext-line"/>
    <line x1="730" y1="410" x2="730" y2="450" class="ext-line"/>
    <text x="530" y="455" class="dim-text">Ø 400 - Ø 650 ADJUSTABLE CLAMP SPAN</text>

    <circle cx="250" cy="115" r="12" fill="#2563eb"/>
    <text x="250" y="119" text-anchor="middle" class="callout">1</text>
    <line x1="250" y1="127" x2="270" y2="140" stroke="#2563eb" stroke-width="1.5"/>

    <circle cx="610" cy="188" r="12" fill="#2563eb"/>
    <text x="610" y="192" text-anchor="middle" class="callout">2</text>
    <line x1="598" y1="188" x2="565" y2="188" stroke="#2563eb" stroke-width="1.5"/>

    <circle cx="450" cy="270" r="12" fill="#2563eb"/>
    <text x="450" y="274" text-anchor="middle" class="callout">3</text>
    <line x1="462" y1="270" x2="495" y2="270" stroke="#2563eb" stroke-width="1.5"/>

    <circle cx="640" cy="330" r="12" fill="#2563eb"/>
    <text x="640" y="334" text-anchor="middle" class="callout">4</text>
    <line x1="640" y1="342" x2="640" y2="360" stroke="#2563eb" stroke-width="1.5"/>

    <circle cx="770" cy="400" r="12" fill="#2563eb"/>
    <text x="770" y="404" text-anchor="middle" class="callout">5</text>
    <line x1="758" y1="400" x2="745" y2="400" stroke="#2563eb" stroke-width="1.5"/>

    <circle cx="280" cy="400" r="12" fill="#2563eb"/>
    <text x="280" y="404" text-anchor="middle" class="callout">6</text>
    <line x1="292" y1="400" x2="315" y2="400" stroke="#2563eb" stroke-width="1.5"/>

    <circle cx="240" cy="470" r="12" fill="#ea580c"/>
    <text x="240" y="474" text-anchor="middle" class="callout">7</text>
    <line x1="252" y1="470" x2="280" y2="470" stroke="#ea580c" stroke-width="1.5"/>
  </g>

  <g transform="translate(150, 600)">
    <text x="-90" y="-80" class="title" font-size="16">PLAN VIEW: 8-PETAL CATCHMENT</text>
    <circle cx="80" cy="50" r="100" fill="#f0f9ff" stroke="#0f172a" stroke-width="1.8"/>
    <circle cx="80" cy="50" r="25" fill="#bae6fd" stroke="#0f172a" stroke-width="1.2"/>
    <circle cx="80" cy="50" r="8" fill="#3b82f6" stroke="#0f172a" stroke-width="1"/>

    <line x1="-20" y1="50" x2="180" y2="50" stroke="#0f172a" stroke-width="1.2"/>
    <line x1="80" y1="-50" x2="80" y2="150" stroke="#0f172a" stroke-width="1.2"/>
    <line x1="10" y1="-20" x2="150" y2="120" stroke="#0f172a" stroke-width="1.2"/>
    <line x1="10" y1="120" x2="150" y2="-20" stroke="#0f172a" stroke-width="1.2"/>

    <text x="80" y="170" class="dim-text">8 EQUISPACED CONCAVE PETALS</text>
  </g>

  <g transform="translate(420, 560)">
    <rect width="300" height="170" fill="#f8fafc" stroke="#94a3b8" stroke-width="1"/>
    <text x="15" y="22" class="title" font-size="13">BOM CALLOUT LEGEND</text>
    <text x="15" y="42" class="label">1. Flower Petal Dish (UV HDPE)</text>
    <text x="15" y="62" class="label">2. Debris Filter Screen (304 SS 2.5mm)</text>
    <text x="15" y="82" class="label">3. Central Funnel Spigot (Ø60mm)</text>
    <text x="15" y="102" class="label">4. Stepped Adapter Collar (LLDPE)</text>
    <text x="15" y="122" class="label">5. Radial Clamps + M8 Fasteners (316 SS)</text>
    <text x="15" y="142" class="label">6. Perimeter Lip Seal (Food EPDM)</text>
    <text x="15" y="162" class="label">7. Tinaco Rim (Standard CDMX 450-600mm)</text>
  </g>
</svg>
"""
    output_path.write_text(svg_content, encoding="utf-8")


def main() -> None:
    """
    Generate and export all CAD assets to the current directory.
    """
    out_dir = Path(__file__).parent

    flower = create_flower_collector()
    step_flower = out_dir / "flower_rainwater_collector.step"
    stl_flower = out_dir / "flower_rainwater_collector.stl"
    bd.export_step(flower, str(step_flower))
    bd.export_stl(flower, str(stl_flower))

    adapter = create_tinaco_adapter()
    step_adapter = out_dir / "tinaco_adapter.step"
    stl_adapter = out_dir / "tinaco_adapter.stl"
    bd.export_step(adapter, str(step_adapter))
    bd.export_stl(adapter, str(stl_adapter))

    svg_drawing = out_dir / "assembly_drawing.svg"
    create_svg_assembly_drawing(svg_drawing)


if __name__ == "__main__":
    main()
