# Tunnels and bridges with truck rules

Real tunnels on the map's legs whose published rules turn trucks away. The
rules live on each leg as `crossings` in `data/world_source/legs/*.json`
(written through `tools/world_source.py`), and every entry carries its
official source, the date it was read, and a `basis` note saying which
values were read, derived or assumed. The Anton Anderson Memorial Tunnel's
schedule, toll and gate are documented in [whittier-tunnel.md](whittier-tunnel.md).

## How a crossing plays

- **Dispatch plans around it.** The job board books a regulated load only
  on a legal road. If the legal road is much longer than the open one
  (beyond the usual alternate-route allowance) or there is none, the load
  is not offered.
- **Departure re-checks.** The route menu offers only legal roads and says
  "Routed around the" crossing when the shortest road was dropped. With no
  legal road the load stays at the dock and the rule is read out.
- **No truck is stranded.** A crossing is judged where its leg starts. A
  truck that reaches one it may not use (a save from before these rules,
  for example) is re-planned from that city, and if no legal road continues
  from there it runs back toward the origin and takes the origin's legal
  road. Only the Whittier tunnel's schedule ever holds a truck, and only
  until its next published opening.

## Game loads and how rules read them

The catalog does not name hazard classes, so these mappings are modelling
choices, not measurements:

- **Placarded hazardous materials** (`hazardous`): placarded packaged
  freight of unnamed classes. A crossing that bans some classes outright
  refuses it.
- **Bulk fuel** (`fuel_bulk`): placarded Class 3 flammable liquid in a cargo
  tank.
- **Packaged chemicals** (`chemicals`): regulated but below placard
  thresholds; counted by weight against a pound limit.
- **Other tank freight** (milk, food-grade liquids): a cargo tank with no
  hazardous material, refused only where every cargo tanker is banned.

Size limits use a stock rig: 73.5 feet with a trailer, 25 feet bobtail,
8.5 feet wide, 13.5 feet high.

## Tunnels added

| Tunnel | Legs | Rules modelled | Source (read 2026-10-11) |
| --- | --- | --- | --- |
| Anton Anderson Memorial Tunnel | Bear Valley to Whittier | Placarded loads banned; 400 lb unplacarded limit; 75 ft long, 10 ft wide, 14 ft high without permit | https://dot.alaska.gov/creg/whittiertunnel/vehiclesize.shtml |
| Eisenhower-Johnson Memorial Tunnel | Denver to Salt Lake City; Denver to Silverthorne | Placarded loads banned; they take US 6 over Loveland Pass (the map's Silverthorne to Denver leg) | https://www.codot.gov/travel/ejmt/metering |
| Fort McHenry Tunnel | Wilmington to Baltimore; Philadelphia to Baltimore; Washington to Philadelphia | Placarded loads banned, including bulk fuel | https://www.mdta.maryland.gov/TunnelRestrictionsAndVehiclePermits |
| Boston tunnels (Central Artery, Prudential, Tip O'Neill) | Boston to Hartford, Manchester, Providence, Worcester; New York, Portland and Portsmouth to Boston | All hazardous cargo (any amount) and all cargo tankers banned | https://www.mass.gov/info-details/hazardous-material-route-designation |
| Hampton Roads Bridge-Tunnel | Norfolk to Richmond; Richmond to Virginia Beach | Banned classes refuse placarded packaged loads; flammables only non-bulk, so bulk fuel refused | https://law.lis.virginia.gov/admincode/title24/agency30/chapter61/section40/ |
| Chesapeake Bay Bridge-Tunnel (Thimble Shoal and Chesapeake Channel tunnels) | Norfolk to Cape Charles | Banned classes refuse placarded packaged loads; Class 3 tank vehicles banned even empty | https://www.cbbt.com/regulations/ |
| Pennsylvania Turnpike tunnels (Allegheny, Tuscarora, Kittatinny, Blue Mountain, Lehigh) | Baltimore, Harrisburg, Philadelphia, Carlisle and Hagerstown to Pittsburgh; Scranton to Allentown | Table 1 materials and explosives banned; flammable liquid banned in bulk | https://www.paturnpike.com/commercial/permits-restrictions/hazardous-materials-(placarded-loads) |

Supporting sources: Colorado rule 2 CCR 601-8, 3.02 (EJMT placarded loads
use Loveland Pass); COMAR 11.07.01.04 (Maryland tunnels); 700 CMR 7.11
(Massachusetts); 24VAC30-61-40 and Code of Virginia 46.2-1110 (Virginia
tunnels); 67 Pa. Code 601.5 (Turnpike).

**How legs were matched.** Each leg's archived road geometry was compared
with the FHWA National Tunnel Inventory (2026 release) and with portal
coordinates; only legs whose geometry runs through the bore were tagged.
The map has one record per leg, crossed in either direction, so each rule
applies both ways. Legs without archived geometry were checked against
their corridor route points.

**Assumptions to know.** EJMT: Loveland Pass closures are not modelled,
so the pass is always open to placarded loads. Fort McHenry: unplacarded
packaged chemicals are not refused, since the propane container limits are
not modelled. Boston: the legal roads below reach the same downtown city
point as the tunnel legs, on surface streets for the last miles.

## Legal detours

A load a tunnel refuses is planned onto the real road a hazardous-materials
truck takes instead. Five legs carry those roads. Each runs between the same
two cities as the tunnel leg it replaces, so it serves every trip that
tunnel leg did, and the planner only picks it for a refused load or when it
is shorter. Each road was pinned from a Valhalla truck route with
`hazmat=true` (OpenStreetMap data, read 2026-10-13) and then given the full
corridor enrichment; lane counts came from the same OpenStreetMap data in a
local extract. Each leg's `detour_for` names the crossing it bypasses, so the
planner never mistakes it for the other direction of the tunnel leg, and a
saved trip resumes on whichever of the two its load was planned on.

| Detour leg | Road | Miles (tunnel leg) | Avoids |
| --- | --- | --- | --- |
| Worcester to Boston | I-90, I-95 and Route 128, then Route 9 into the city | 47.5 (47.2) | Boston tunnels |
| Providence to Boston | I-95 and Route 128, then Route 9 | 53.1 (50) | Boston tunnels |
| Boston to Portsmouth | Route 99, US 1 and I-95 | 58 (58) | Boston tunnels |
| Manchester to Boston | I-93 to Sullivan Square, then Rutherford Avenue into Charlestown | 54 (52) | Boston tunnels |
| Baltimore to Wilmington | I-83, the northern I-695 beltway and I-95 | 82.9 (75) | Fort McHenry Tunnel |
| Silverthorne to Denver | US 6 over Loveland Pass and I-70 | 76.9 (67) | Eisenhower-Johnson Memorial Tunnel |

Hartford, Portland and New York loads into or out of Boston join one of
these roads at Worcester, Providence or Portsmouth, and Manchester loads
take the I-93 leg in both directions. Philadelphia
and Washington loads through Baltimore use the I-695 leg by way of
Wilmington.

**Which legs pass under Fort McHenry.** Checked against archived geometry:
Wilmington to Baltimore passes 0.13 miles from the tunnel's inventory point
at mile 70 and then takes I-395 downtown; Philadelphia to Baltimore and
Washington to Philadelphia run the same I-95 harbor crossing. No other
Baltimore leg does.

## Bridges added

| Bridge | Legs | Rules modelled | Source (read 2026-10-11) |
| --- | --- | --- | --- |
| San Francisco-Oakland Bay Bridge | Sacramento, Stockton and Los Angeles to San Francisco; San Francisco to Portland and Salt Lake City | Explosives and flammable-placarded tank loads banned; placarded packaged freight refused (classes unnamed) | https://www.chp.ca.gov/siteassets/files/ib_hmroutes.pdf (21 CCR 1402.1) |
| Mackinac Bridge | Sault Ste. Marie to Traverse City | Toll: 5 dollars per axle, 25 dollars for a five-axle rig, both directions | https://www.mackinacbridge.org/fares-traffic/fare-schedule/ |
| Chesapeake Bay Bridge-Tunnel | Norfolk to Cape Charles | Toll corrected from an estimated 45 to the published 48 dollars (Class 12, initial crossing) | https://www.cbbt.com/tolls/ |

Bay Bridge legs were matched where archived geometry runs within 60 metres
of the Yerba Buena Island tunnel. The Mackinac toll sits at the St. Ignace
plaza, 52.1 miles along its leg, from the same geometry.

**Posted bridge weight limits and low clearances** were already on the map
as corridor restrictions taken from OpenStreetMap; truck routing already
keeps a legal rig under them, so they stay as spoken signage and are not
duplicated here.

## Researched and left out

- **Interstate 90 tunnels across Lake Washington (Mount Baker Ridge,
  Mercer Island lid).** The only official text found is a WSDOT notice of
  an overnight maintenance closure
  (https://content.govdelivery.com/accounts/WADOT/bulletins/3df8359, read
  2026-10-13). No standing WSDOT rule, WAC section or posted restriction
  was found, and RCW 47.48.050 only allows temporary restrictions, so no
  rule is modelled and Seattle fuel keeps I-90.
- **Holland and Lincoln tunnels.** No leg uses them. The New York city point
  is in the Bronx, and every New York leg, including Allentown and Newark,
  crosses the Hudson on the George Washington Bridge.

- **Baltimore Harbor Tunnel (I-895).** Same MDTA ban, but no leg runs
  through it.
- **Wallace Tunnel, Wheeling Tunnel, I-10 Deck Park Tunnel, Harvey Tunnel,
  Vista Ridge Tunnel, Pigeon River tunnels.** On a leg and flagged in the
  FHWA inventory, but no official rule text was found to model.
- **George Washington Bridge lower level.** Trucks are kept off the lower
  level, but placarded loads may use the upper level, so no load is turned
  away; a bridge in any case.
- **Golden Gate Bridge (San Francisco to Santa Rosa).** It bans explosive
  loads only at rush hours and escorts them otherwise; the game has no
  explosives cargo or escort, so nothing is refused.
- **Mackinac Bridge escorts.** Placarded loads, rigs over 40 tons gross and
  oversize loads cross under bridge escort, not a ban; escorts are not
  modelled. Its 72-ton maximum is above any legal game rig.
- **Wind closures.** The Chesapeake Bay Bridge-Tunnel restricts empty
  tractor-trailers (under 15,000 lb of cargo) from 40 mph winds, and the
  Mackinac Bridge closes in high wind. The game's weather has no wind
  speed yet, so these are not modelled.
- **Chesapeake Bay Bridge (US 50, Maryland) toll.** Collected eastbound
  only; the official truck rate table could not be read, so no toll was
  added.

## Whittier

Whittier still has no freight. No real freight receiver with a verifiable
address and coordinates was found, so no facility was added and no career
job drives the tunnel; see [whittier-tunnel.md](whittier-tunnel.md).
