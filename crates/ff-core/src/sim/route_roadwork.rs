//! Live construction along a candidate route, read the way a dispatcher
//! reads 511 before choosing a lane: which of the sensible routes carries
//! reported construction, how many minutes it costs, and whether the road is
//! closed outright.
//!
//! Dispatch has no map of roadwork of its own. The zones a drive meets are
//! drawn when the trip starts, and the only construction known before the
//! wheels turn is what the state 511 feeds report. So this module reads the
//! same provider cache the trip will read, on the two or three route options
//! `World::supported_route_options` already ranks by distance, and re-ranks
//! them by drive time plus the construction delay. A route through a
//! reported full closure sorts last whatever its time: a dispatcher does not
//! send a truck at a closed road while another way exists.
//!
//! Every number here is derived from the feed and the deadline model, and
//! says so: the delay through a zone is its length at the zone's limit minus
//! the same length at the route's own planned pace. No queue penalty is
//! guessed for a lane closure, because the feeds carry no demand to derive
//! one from; a closure costs only its slower stretch until they do.

use std::collections::HashSet;

use crate::data::world::World;
use crate::data::world_models::{Leg, Route};
use crate::models::jobs::route_drive_hours;
use crate::pyfmt::fmt_f;
use crate::settings::Settings;
use crate::sim::trip::Trip;
use crate::sim::trip_models::ZONE_MIN_GAP_MI;
use crate::sim::trip_route_helpers::nearest_mile_on_leg;
use crate::sim::trip_traffic::TrafficProvider;

/// How far from a route point a 511 event still counts as being on the
/// road, the same radius the trip uses when it places its zones.
const NEAR_ROUTE_MI: f64 = 3.0;

/// A second route that is quicker by less than this is a coin toss, and the
/// shorter one keeps its place.
const TIE_MINUTES: f64 = 1.0;

/// One reported construction event snapped onto a route.
#[derive(Debug, Clone, PartialEq)]
pub struct ConstructionSpot {
    /// The road as the world names it (`I-70`).
    pub road: String,
    /// The route city nearest the spot.
    pub near_city: String,
    /// The feed's closure kind: "full closure", "single lane", "alternating",
    /// "shoulder", or empty when it did not say.
    pub closure: String,
    pub length_mi: f64,
    pub limit_mph: f64,
    pub route_mile: f64,
    /// Hours lost against the route's planned pace; derived, never guessed.
    pub delay_h: f64,
}

impl ConstructionSpot {
    /// The closure in a driver's words.
    pub fn closure_words(&self) -> &'static str {
        match self.closure.as_str() {
            "full closure" => "the road closed",
            "single lane" => "one lane closed",
            "alternating" => "alternating one-way traffic",
            "shoulder" => "shoulder work",
            _ => "a construction zone",
        }
    }
}

/// Everything 511 reports along one route option.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RouteConstruction {
    pub spots: Vec<ConstructionSpot>,
    /// The sum of the spots' delays, in hours.
    pub delay_h: f64,
    /// A reported full closure somewhere on the route.
    pub blocked: bool,
    /// The deadline model's drive hours for the route with no construction.
    pub drive_h: f64,
}

impl RouteConstruction {
    pub fn total_h(&self) -> f64 {
        self.drive_h + self.delay_h
    }
}

/// Dispatch's ranking of the route options, quickest first.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DispatchRouting {
    /// Indices into the routes given, dispatch's pick first.
    pub order: Vec<usize>,
    /// One report per route given, in the routes' original order.
    pub reports: Vec<RouteConstruction>,
}

impl DispatchRouting {
    /// The route dispatch picked, or 0 when nothing was given.
    pub fn pick(&self) -> usize {
        self.order.first().copied().unwrap_or(0)
    }

    /// True when live construction moved dispatch off the shortest route.
    pub fn detoured(&self) -> bool {
        self.pick() != 0
    }

    pub fn report(&self, index: usize) -> Option<&RouteConstruction> {
        self.reports.get(index)
    }
}

/// The 511 state key a leg is looked up under: the state it runs in, as the
/// bake recorded it, lower-cased the way the provider keys its cache. Empty
/// where the bake is silent.
fn leg_state(leg: &Leg, forward: bool) -> String {
    let mut state = String::new();
    for sc in leg.state_crossings() {
        state = if forward {
            sc.from_state.clone()
        } else {
            sc.state.clone()
        };
    }
    let state_miles = leg.state_miles();
    if state.is_empty() && !state_miles.is_empty() {
        let first = if forward {
            &state_miles[0]
        } else {
            &state_miles[state_miles.len() - 1]
        };
        state = first.state.clone();
    }
    state.trim().to_lowercase()
}

/// The 511 state keys a route touches, for warming the provider's cache
/// before dispatch needs an answer.
pub fn route_state_keys(route: &Route) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut keys = Vec::new();
    for (i, leg) in route.legs.iter().enumerate() {
        let forward = route.cities[i] == leg.a;
        let state = leg_state(leg, forward);
        if !state.is_empty() && seen.insert(state.clone()) {
            keys.push(state);
        }
    }
    keys
}

/// Read the provider's construction events onto a route.
pub fn scan_route_construction(
    route: &Route,
    provider: &dyn TrafficProvider,
    world: &World,
) -> RouteConstruction {
    let drive_h = route_drive_hours(Some(route), 0.0, Some(world));
    let mut report = RouteConstruction {
        drive_h,
        ..RouteConstruction::default()
    };
    let miles = route.miles();
    if miles <= 0.0 || drive_h <= 0.0 {
        return report;
    }
    let pace_mph = miles / drive_h;
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut seen_spans: Vec<(f64, f64)> = Vec::new();
    let mut leg_start = 0.0;
    for (i, leg) in route.legs.iter().enumerate() {
        let forward = route.cities[i] == leg.a;
        let state = leg_state(leg, forward);
        let points: Vec<(f64, f64)> = leg
            .route_points()
            .iter()
            .map(|rp| (rp.lat, rp.lon))
            .collect();
        if state.is_empty() || points.is_empty() {
            leg_start += leg.miles;
            continue;
        }
        let events = provider.get_construction_near_route(
            &state,
            &points,
            Some(&leg.highway),
            NEAR_ROUTE_MI,
        );
        for event in events {
            if !seen_ids.insert(event.id.clone()) {
                continue;
            }
            let (Some(lat), Some(lon)) = (event.latitude, event.longitude) else {
                continue;
            };
            let Some(route_mile) = nearest_mile_on_leg(lat, lon, leg, forward, leg_start) else {
                continue;
            };
            let length_mi = Trip::construction_zone_length(&event);
            let start_mi = (route_mile - length_mi / 2.0).max(0.0);
            let end_mi = miles.min(start_mi + length_mi);
            if seen_spans
                .iter()
                .any(|(s, e)| start_mi < e + ZONE_MIN_GAP_MI && end_mi > s - ZONE_MIN_GAP_MI)
            {
                continue;
            }
            seen_spans.push((start_mi, end_mi));
            let limit_mph = Trip::construction_zone_speed(&event);
            // Derived: the zone's length at its limit, less the same length at
            // the pace the deadline model already plans this route at.
            let delay_h = (length_mi / limit_mph - length_mi / pace_mph).max(0.0);
            let offset = route_mile - leg_start;
            let near_key = if (offset < leg.miles / 2.0) == forward {
                &leg.a
            } else {
                &leg.b
            };
            let near_city = world
                .cities
                .get(near_key)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| near_key.clone());
            let blocked = event.closure == "full closure";
            report.blocked |= blocked;
            report.delay_h += delay_h;
            report.spots.push(ConstructionSpot {
                road: leg.highway.clone(),
                near_city,
                closure: event.closure.clone(),
                length_mi,
                limit_mph,
                route_mile,
                delay_h,
            });
        }
        leg_start += leg.miles;
    }
    report
        .spots
        .sort_by(|a, b| a.route_mile.total_cmp(&b.route_mile));
    report
}

/// Rank the route options the way dispatch does: quickest first counting
/// reported construction, a full closure last, and the shorter route keeping
/// its place when the difference is under a minute. With no provider the
/// order is the one given.
pub fn choose_dispatch_route(
    routes: &[Route],
    provider: Option<&dyn TrafficProvider>,
    world: &World,
) -> DispatchRouting {
    let reports: Vec<RouteConstruction> = routes
        .iter()
        .map(|route| match provider {
            Some(provider) => scan_route_construction(route, provider, world),
            None => RouteConstruction {
                drive_h: route_drive_hours(Some(route), 0.0, Some(world)),
                ..RouteConstruction::default()
            },
        })
        .collect();
    let mut order: Vec<usize> = (0..routes.len()).collect();
    if provider.is_some() && reports.iter().any(|r| !r.spots.is_empty()) {
        order.sort_by(|&a, &b| {
            let (ra, rb) = (&reports[a], &reports[b]);
            // An open road before a closed one, whatever the clock says.
            ra.blocked.cmp(&rb.blocked).then_with(|| {
                let gap_min = (ra.total_h() - rb.total_h()) * 60.0;
                if gap_min.abs() < TIE_MINUTES {
                    a.cmp(&b)
                } else {
                    ra.total_h().total_cmp(&rb.total_h())
                }
            })
        });
    }
    DispatchRouting { order, reports }
}

fn minutes_words(hours: f64) -> String {
    let minutes = (hours * 60.0).round();
    if minutes < 1.0 {
        "under a minute".to_string()
    } else if minutes == 1.0 {
        "about a minute".to_string()
    } else {
        format!("about {} minutes", fmt_f(minutes, 0))
    }
}

/// One spot in dispatch's words: "one lane closed on I-70 near Topeka, 5
/// miles at 45 miles per hour, about 3 minutes".
pub fn spot_text(spot: &ConstructionSpot, settings: &Settings) -> String {
    format!(
        "{} on {} near {}, {} at {}, {}",
        spot.closure_words(),
        spot.road,
        spot.near_city,
        settings.distance_text(spot.length_mi, false),
        settings.speed_text(spot.limit_mph),
        minutes_words(spot.delay_h)
    )
}

/// What dispatch says about the route it picked, before naming the
/// destination. Empty when 511 reported nothing on any option.
pub fn dispatch_route_line(
    routes: &[Route],
    routing: &DispatchRouting,
    world: &World,
    settings: &Settings,
) -> String {
    let pick = routing.pick();
    let (Some(picked), Some(report)) = (routes.get(pick), routing.report(pick)) else {
        return String::new();
    };
    let mut parts: Vec<String> = Vec::new();
    if routing.detoured() {
        if let (Some(shortest), Some(avoided)) = (routes.first(), routing.report(0)) {
            let extra_mi = picked.miles() - shortest.miles();
            let adds = if extra_mi > 0.5 {
                format!("adds {} and ", settings.distance_text(extra_mi, false))
            } else {
                String::new()
            };
            let spot = avoided.spots.first();
            let place = spot
                .map(|spot| format!("on {} near {}", spot.road, spot.near_city))
                .unwrap_or_default();
            if avoided.blocked {
                // A closed road is not a time saving; the detour costs what it
                // costs against the road as it would have run open.
                let extra_h = report.total_h() - avoided.drive_h;
                let via = picked
                    .cities
                    .get(1)
                    .filter(|_| picked.cities.len() > 2)
                    .map(|key| {
                        world
                            .cities
                            .get(key)
                            .map(|c| c.name.clone())
                            .unwrap_or_else(|| key.clone())
                    })
                    .map(|name| format!(" by way of {name}"))
                    .unwrap_or_default();
                let cost = if extra_h * 60.0 >= 0.5 {
                    format!("This way {adds}takes {}.", minutes_words(extra_h))
                } else if extra_mi > 0.5 {
                    format!("This way adds {}.", settings.distance_text(extra_mi, false))
                } else {
                    "This way costs no time.".to_string()
                };
                parts.push(format!(
                    "The road is closed {place}. Dispatch is routing you around it{via}. {cost}"
                ));
            } else {
                let what = spot
                    .map(|spot| format!("{} {place}", spot.closure_words()))
                    .unwrap_or_else(|| "construction".to_string());
                let saved_h = avoided.total_h() - report.total_h();
                parts.push(format!(
                    "Dispatch is routing you around {what}. This way {adds}saves {}.",
                    minutes_words(saved_h)
                ));
            }
        }
    }
    if !report.spots.is_empty() {
        let listed: Vec<String> = report
            .spots
            .iter()
            .map(|spot| spot_text(spot, settings))
            .collect();
        let tail = if routing.detoured() {
            ""
        } else {
            " No quicker way around."
        };
        parts.push(format!(
            "Construction reported on the way: {}.{tail}",
            listed.join("; ")
        ));
    }
    parts.join(" ")
}

/// What the route planning screen says about dispatch's ranking, for a
/// driver who chooses their own route. Empty unless construction moved the
/// recommendation off the shortest way; the per-option notes carry the rest.
pub fn route_planning_note(routes: &[Route], routing: &DispatchRouting, world: &World) -> String {
    if !routing.detoured() {
        return String::new();
    }
    let pick = routing.pick();
    let (Some(picked), Some(report), Some(avoided)) =
        (routes.get(pick), routing.report(pick), routing.report(0))
    else {
        return String::new();
    };
    let spot = avoided.spots.first();
    let place = spot
        .map(|spot| format!("on {} near {}", spot.road, spot.near_city))
        .unwrap_or_default();
    if avoided.blocked {
        let via = picked
            .cities
            .get(1)
            .filter(|_| picked.cities.len() > 2)
            .map(|key| {
                world
                    .cities
                    .get(key)
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| key.clone())
            })
            .map(|name| format!(" by way of {name}"))
            .unwrap_or_default();
        return format!("The road is closed {place}. Route 1 goes around it{via}.");
    }
    let what = spot
        .map(|spot| format!("{} {place}", spot.closure_words()))
        .unwrap_or_else(|| "construction".to_string());
    let saved_h = avoided.total_h() - report.total_h();
    format!(
        "Route 1 avoids {what} and saves {}.",
        minutes_words(saved_h)
    )
}

/// The note appended to one route option on the route planning screen.
/// Empty when 511 reported nothing on it.
pub fn route_option_note(report: &RouteConstruction, settings: &Settings) -> String {
    if report.spots.is_empty() {
        return String::new();
    }
    let listed: Vec<String> = report
        .spots
        .iter()
        .map(|spot| spot_text(spot, settings))
        .collect();
    format!("Construction: {}.", listed.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::world::get_world;
    use crate::sim::real_traffic::{wall_time, RealTrafficProvider, TrafficData};
    use crate::sim::real_traffic_parsers::TrafficEvent;

    /// The fixture pair: I-71 straight down from Columbus, or I-70 and I-75
    /// by way of Dayton, 18 miles and 25 minutes longer. Ohio has no live
    /// 511 API, so the provider serves exactly what the test seeds.
    fn ohio_routes() -> Vec<Route> {
        get_world()
            .supported_route_options("Columbus", "Cincinnati", 3)
            .expect("Columbus to Cincinnati routes")
    }

    fn construction(id: &str, closure: &str, lat: f64, lon: f64, location: &str) -> TrafficEvent {
        let mut event = TrafficEvent::new(id, "construction", "medium", "Roadwork", "Test");
        event.latitude = Some(lat);
        event.longitude = Some(lon);
        event.closure = closure.to_string();
        event.location_text = location.to_string();
        event.work_type = "construction".to_string();
        event
    }

    fn seeded(state: &str, events: Vec<TrafficEvent>) -> RealTrafficProvider {
        let provider = RealTrafficProvider::offline();
        let now = wall_time();
        provider.seed_cache(
            &format!("{state}:construction"),
            TrafficData::new(state, events, now, now, "test"),
        );
        provider
    }

    /// A point the I-71 leg's own geometry passes through, between the two
    /// cities, so the event snaps onto the shortest route and no other.
    const ON_I71: (f64, f64) = (39.53665, -83.78303);

    #[test]
    fn test_no_provider_keeps_the_distance_order_and_says_nothing() {
        let world = get_world();
        let routes = ohio_routes();
        let routing = choose_dispatch_route(&routes, None, world);
        assert_eq!(routing.order, vec![0, 1, 2]);
        assert!(!routing.detoured());
        assert!(routing
            .reports
            .iter()
            .all(|r| r.spots.is_empty() && r.drive_h > 0.0));
        assert_eq!(
            dispatch_route_line(&routes, &routing, world, &Settings::default()),
            ""
        );
    }

    #[test]
    fn test_a_reported_closure_sends_dispatch_around_by_the_next_route() {
        let world = get_world();
        let routes = ohio_routes();
        let provider = seeded(
            "ohio",
            vec![construction(
                "closed",
                "full closure",
                ON_I71.0,
                ON_I71.1,
                "",
            )],
        );
        let routing = choose_dispatch_route(&routes, Some(&provider), world);
        assert!(routing.reports[0].blocked);
        assert!(!routing.reports[1].blocked);
        assert_eq!(routing.pick(), 1, "{:?}", routing.order);
        let line = dispatch_route_line(&routes, &routing, world, &Settings::default());
        assert!(
            line.starts_with("The road is closed on I-71 near "),
            "{line}"
        );
        assert!(
            line.contains("Dispatch is routing you around it by way of Dayton."),
            "{line}"
        );
        assert!(
            line.contains("This way adds 18 miles and takes about "),
            "{line}"
        );
        assert!(!line.contains("Construction reported on the way"), "{line}");
        let note = route_planning_note(&routes, &routing, world);
        assert!(
            note.starts_with("The road is closed on I-71 near "),
            "{note}"
        );
        assert!(
            note.ends_with("Route 1 goes around it by way of Dayton."),
            "{note}"
        );
    }

    #[test]
    fn test_a_short_lane_closure_stays_on_the_shortest_route_and_is_announced() {
        let world = get_world();
        let routes = ohio_routes();
        let provider = seeded(
            "ohio",
            vec![construction("lane", "single lane", ON_I71.0, ON_I71.1, "")],
        );
        let routing = choose_dispatch_route(&routes, Some(&provider), world);
        assert_eq!(routing.pick(), 0, "{:?}", routing.order);
        let report = &routing.reports[0];
        assert_eq!(report.spots.len(), 1);
        assert!(!report.blocked);
        // 5 miles at 45 against the route's own pace: a couple of minutes,
        // never the 25 the Dayton way costs.
        let delay_min = report.delay_h * 60.0;
        assert!(delay_min > 1.0 && delay_min < 4.0, "{delay_min}");
        let line = dispatch_route_line(&routes, &routing, world, &Settings::default());
        assert!(
            line.starts_with("Construction reported on the way: one lane closed on I-71 near "),
            "{line}"
        );
        assert!(
            line.contains("5 miles at 45 miles per hour, about "),
            "{line}"
        );
        assert!(line.ends_with("No quicker way around."), "{line}");
        let note = route_option_note(report, &Settings::default());
        assert!(
            note.starts_with("Construction: one lane closed on I-71 near "),
            "{note}"
        );
        assert!(route_option_note(&routing.reports[1], &Settings::default()).is_empty());
    }

    #[test]
    fn test_a_lane_closure_costing_more_than_the_next_route_moves_dispatch_over() {
        // Chicago to Indianapolis: I-65 straight through, or I-90 then I-65
        // through Gary and Lafayette, two miles and two minutes longer. A
        // twenty-mile single-lane stretch costs more than that by more than
        // the one-minute margin a coin toss keeps the shorter route on.
        let world = get_world();
        let routes = world
            .supported_route_options("Chicago", "Indianapolis", 3)
            .expect("Chicago to Indianapolis routes");
        assert!(routes.len() >= 2);
        let gap_min = (route_drive_hours(Some(&routes[1]), 0.0, Some(world))
            - route_drive_hours(Some(&routes[0]), 0.0, Some(world)))
            * 60.0;
        assert!(gap_min > TIE_MINUTES && gap_min < 5.0, "{gap_min}");
        let provider = seeded(
            "illinois",
            vec![construction(
                "long-lane",
                "single lane",
                40.62512,
                -86.99675,
                "Between milepost 152 and 172",
            )],
        );
        let routing = choose_dispatch_route(&routes, Some(&provider), world);
        assert_eq!(routing.reports[0].spots.len(), 1);
        assert!((routing.reports[0].spots[0].length_mi - 20.0).abs() < 1e-9);
        assert_eq!(routing.pick(), 1, "{:?}", routing.order);
        let line = dispatch_route_line(&routes, &routing, world, &Settings::default());
        assert!(
            line.starts_with("Dispatch is routing you around one lane closed on I-65 near "),
            "{line}"
        );
        assert!(line.contains("saves "), "{line}");
        let note = route_planning_note(&routes, &routing, world);
        assert!(
            note.starts_with("Route 1 avoids one lane closed on I-65 near "),
            "{note}"
        );
        assert!(
            route_planning_note(&routes, &choose_dispatch_route(&routes, None, world), world)
                .is_empty()
        );
    }

    #[test]
    fn test_route_state_keys_name_each_state_once_in_road_order() {
        let world = get_world();
        let routes = world
            .supported_route_options("Chicago", "Indianapolis", 3)
            .expect("routes");
        assert_eq!(route_state_keys(&routes[1]), vec!["illinois", "indiana"]);
        assert_eq!(route_state_keys(&ohio_routes()[1]), vec!["ohio"]);
    }
}
