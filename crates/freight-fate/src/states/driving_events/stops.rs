//! The rest key (`T`): planning a break or sleep stop, the selected-stop intent, and
//! opening a route point's own menu.

use ff_core::sim::hos;
use ff_core::sim::trip_models::RoadStop;
use ff_core::speech_pacing::SpeechCategory;

use crate::app::{GameContext, Say};
use crate::states::base::TimedMessageState;
use crate::states::driving::{DrivingState, StopVisit};
use crate::states::driving_core::*;
use crate::states::driving_rest_states::RestFocus;

use super::with_drive;

#[derive(Clone, Copy, PartialEq, Eq)]
enum HosRestPurpose {
    Break,
    SleepForBreak,
    Sleep,
}

enum HosRestChoice {
    NotNeeded,
    NoReachable,
    Recommended {
        stop: Box<RoadStop>,
        purpose: HosRestPurpose,
        fallback: bool,
    },
}

impl DrivingState {
    /// `_try_rest_stop()`: the rest key, wherever the truck happens to be.
    pub fn try_rest_stop(&mut self, ctx: &mut GameContext) {
        let rest_hint = ctx.control_hint("rest");
        let exit_hint = ctx.control_hint("take_exit");
        if self.pull_over.is_some() {
            self.set_status("Rest-stop planning unavailable during a police stop.");
            self.say_plain(
                ctx,
                format!(
                    "Resolve the police stop first. Press {exit_hint} to signal the trooper stop."
                ),
            );
            return;
        }
        // An open scale ahead is not optional, so the rest key must not plan
        // a rest stop past it -- the scale comes first, then the plan.
        if self.trip.truck.speed_mph() > DOCKING_MAX_MPH && self.scale_outranks_rest_planning(ctx) {
            return;
        }
        // ...and the scale's own RAMP claims the key too, at any speed. The
        // guard above only fires while the scale is still AHEAD, and a truck
        // on the ramp is already past its mile: the rest key fell straight
        // through to sleep planning and answered "the scale is behind you,
        // plan the next sleep-capable stop" to a driver doing exactly what
        // the scale had just told them to do -- press this key at the scale
        // (owner playtest, 2026-08-21). Jerry's 2026-08-12 report was the
        // same confusion one step earlier, before the ramp; that fix guarded
        // the approach and left the ramp itself open.
        // Stopped ON the scale falls through on purpose: the check-in below
        // is what the key means there.
        let on_scale_ramp = self
            .ramp_stop
            .as_ref()
            .is_some_and(|ramp| ramp.stop_type == "weigh_station");
        if on_scale_ramp && self.trip.truck.speed_mph() > DOCKING_MAX_MPH {
            let name = self
                .ramp_stop
                .as_ref()
                .expect("checked above")
                .spoken_name();
            self.say_plain(
                ctx,
                format!(
                    "On the ramp for {name}. Stop at the scale, then press {rest_hint} to check \
                     in."
                ),
            );
            return;
        }
        let stop = self.trip.nearest_stop_within(1.5).cloned();
        if self.trip.truck.speed_mph() <= DOCKING_MAX_MPH {
            let Some(stop) = stop else {
                if let Some(hint) = self.gate_short_hint(ctx) {
                    self.say_plain(ctx, hint);
                    return;
                }
                if !secure_truck_for_stopped_menu(self, ctx) {
                    self.say_plain(ctx, "Come to a complete stop first.");
                    return;
                }
                let Some(reason) = self.emergency_shoulder_sleep_reason(ctx) else {
                    self.say_plain(
                        ctx,
                        "Emergency shoulder sleep is not available here. Use a route stop.",
                    );
                    return;
                };
                let anchor_mi = self.trip.position_mi;
                self.push_shoulder_sleep_confirmation(ctx, &reason, anchor_mi);
                return;
            };
            self.open_poi_stop(ctx, &stop, false, None);
            return;
        }

        if let Some(active) = self.ramp_stop.clone() {
            let active_is_selected = self.is_selected_stop(Some(&active));
            // What the assist does is the manual's to explain (owner,
            // 2026-10-14); the line only owes the driver's own job.
            let assist = if ctx.settings.destination_approach_assist {
                ""
            } else {
                " Stop at the entrance."
            };
            let message = if active_is_selected {
                format!("On the selected ramp for {}.{assist}", active.spoken_name())
            } else {
                format!("On the ramp for {}.{assist}", active.spoken_name())
            };
            self.set_status(message.clone());
            // The cab confirming a control the player just worked. At the
            // quiet rung this becomes its earcon: you know you pressed K.
            ctx.say_with(message, Say::new().category(SpeechCategory::Confirmation));
            return;
        }

        // A selected plan only owns a repeated T while rolling freely. X can
        // clear the selected-stop intent while deliberately leaving the route
        // plan intact; the next T must then reselect that plan. Police stops,
        // scales, stopped route menus, and an active ramp all returned above.
        // Once this plan is canceled, no selected-stop or assist state may
        // survive to steer a later optional exit.
        if let Some(planned_key) = self
            .trip
            .planned_stop_key
            .clone()
            .filter(|_| self.selected_stop_key.is_some())
        {
            let active_exit = self.exit_stop.clone();
            let exit_matches_plan = active_exit
                .as_ref()
                .is_some_and(|stop| stop.key() == planned_key);
            let signal_was_on = exit_matches_plan && self.exit_signal_on;
            self.trip.planned_stop_key = None;
            self.clear_selected_stop_intent();
            if exit_matches_plan {
                self.exit_stop = None;
                self.exit_signal_on = false;
                self.exit_signal_canceled = false;
                self.cruise_exit_mph = None;
                self.reset_exit_lane_state();
            }
            let message = if signal_was_on {
                "Planned stop canceled. Exit signal canceled.".to_string()
            } else if self.exit_signal_on {
                active_exit.as_ref().map_or_else(
                    || "Planned stop canceled.".to_string(),
                    |stop| {
                        format!(
                            "Planned stop canceled. Exit signal remains active for {}.",
                            stop.spoken_name()
                        )
                    },
                )
            } else {
                "Planned stop canceled.".to_string()
            };
            self.set_status(message.clone());
            self.say_plain(ctx, message);
            return;
        }

        if let Some(selected) = self.selected_rest_stop() {
            let ahead = selected.at_mi - self.trip.position_mi;
            if ahead <= 0.0 {
                self.say_plain(ctx, format!("{} is behind you.", selected.spoken_name()));
                return;
            }
            self.speak_selected_rest_stop(ctx, &selected, true, None, false);
            return;
        }

        if let Some(stop) = stop.as_ref() {
            if stop.at_mi <= self.trip.position_mi {
                self.say_plain(ctx, format!("{} is behind you.", stop.spoken_name()));
                return;
            }
        }

        if let Some(active) = self.exit_stop.clone() {
            if self.exit_signal_on {
                self.set_status(format!("Exit signal active for {}.", active.spoken_name()));
                self.say_plain(
                    ctx,
                    format!("The exit for {} is already selected.", active.spoken_name()),
                );
                return;
            }
        }

        // The departure streets are a trip of their own, swapped out at the
        // on-ramp merge: a plan made here landed on the wrong trip and was
        // cancelled a frame later with a false "past your planned stop".
        // Planning waits for the highway, as the HOS hints already do.
        if self.departure_chain {
            return;
        }

        // HOS advice can choose a compatible break-only stop or a comfortable
        // sleep stop before the legal fallback. Outside that opt-in case, T
        // keeps choosing the NEXT sleep-capable stop ahead, however far.
        let hos_choice = self.hos_rest_choice(ctx);
        // This used to look
        // only as far as the exit window -- the five-odd miles inside which an
        // exit can be SIGNALLED -- so T seven miles short of a rest area
        // answered "no sleep-capable route stop is close enough ahead to
        // plan", and worked a minute later with nothing changed but the
        // odometer (owner, Hanging Lake, 2026-08-22). Planning a stop is a
        // decision about hours, not about the turn signal; where the exit
        // window matters is what the confirmation tells the driver to do
        // next, below.
        let mut candidates: Vec<RoadStop> = self
            .trip
            .stops
            .iter()
            .filter(|candidate| {
                candidate.at_mi > self.trip.position_mi
                    && candidate.actions.iter().any(|action| action == "sleep")
                    && candidate.parking != "none"
                    && candidate.accessible_to(self.trip.bobtail)
            })
            .cloned()
            .collect();
        candidates.sort_by(|a, b| a.at_mi.total_cmp(&b.at_mi));
        let Some(candidate) = (match &hos_choice {
            HosRestChoice::Recommended { stop, .. } => Some((**stop).clone()),
            HosRestChoice::NotNeeded | HosRestChoice::NoReachable => candidates.first().cloned(),
        }) else {
            self.set_status("No sleep-capable route stop ahead on this route.");
            self.say_plain(ctx, "No sleep stop ahead. Shoulder sleep available.");
            return;
        };
        if let Some(current) = self.trip.planned_stop().cloned() {
            if current.key() != candidate.key() {
                self.set_status(format!("Planned stop remains {}.", current.spoken_name()));
                self.say_plain(
                    ctx,
                    format!(
                        "Planned stop stays {}. {} is also ahead.",
                        current.spoken_name(),
                        candidate.spoken_name()
                    ),
                );
                return;
            }
        }
        self.trip.planned_stop_key = Some(candidate.key());
        self.selected_stop_key = Some(candidate.key());
        self.selected_stop_break = matches!(
            &hos_choice,
            HosRestChoice::Recommended {
                purpose: HosRestPurpose::Break,
                ..
            }
        );
        self.selected_stop_assist_armed = false;
        self.selected_stop_assist_said = false;
        self.speak_selected_rest_stop(
            ctx,
            &candidate,
            false,
            match &hos_choice {
                HosRestChoice::Recommended {
                    purpose, fallback, ..
                } => Some((*purpose, *fallback)),
                HosRestChoice::NotNeeded | HosRestChoice::NoReachable => None,
            },
            matches!(&hos_choice, HosRestChoice::NoReachable),
        );
    }

    /// The HOS recommendation only owns T when an intermediate stop is needed.
    /// The stop itself comes from the planner, which checks action, parking,
    /// vehicle access, route position, and legal reach.
    fn hos_rest_choice(&self, ctx: &GameContext) -> HosRestChoice {
        if !ctx.settings.hos_planning_hints {
            return HosRestChoice::NotNeeded;
        }
        let Some(advice) = self.hos_stop_advice(ctx) else {
            return HosRestChoice::NotNeeded;
        };
        if advice.destination_reachable {
            return HosRestChoice::NotNeeded;
        }
        let Some(fallback) = advice.stop.as_ref() else {
            return HosRestChoice::NoReachable;
        };
        let Some(selected) = advice
            .suggested
            .as_ref()
            .map(|option| &option.stop)
            .filter(|stop| stop.at_mi > self.trip.position_mi)
            .or_else(|| (fallback.at_mi > self.trip.position_mi).then_some(fallback))
        else {
            return HosRestChoice::NotNeeded;
        };
        let purpose = if advice.action == "break" {
            if selected.actions.iter().any(|action| action == "break") {
                HosRestPurpose::Break
            } else {
                HosRestPurpose::SleepForBreak
            }
        } else {
            HosRestPurpose::Sleep
        };
        HosRestChoice::Recommended {
            stop: Box::new(selected.clone()),
            purpose,
            fallback: selected.key() == fallback.key(),
        }
    }

    /// The route stop explicitly selected by T, for break or sleep.
    pub fn selected_rest_stop(&self) -> Option<RoadStop> {
        let key = self.selected_stop_key.as_ref()?;
        self.trip
            .stops
            .iter()
            .find(|stop| &stop.key() == key)
            .cloned()
    }

    /// `_is_selected_stop(stop)`.
    pub fn is_selected_stop(&self, stop: Option<&RoadStop>) -> bool {
        match (&self.selected_stop_key, stop) {
            (Some(key), Some(stop)) => *key == stop.key(),
            _ => false,
        }
    }

    /// This visit's record at `stop`, fresh when it is a different stop.
    pub fn stop_visit(&mut self, stop: &RoadStop) -> &mut StopVisit {
        let key = stop.key();
        if self.stop_visit.key != key {
            self.stop_visit = StopVisit {
                key,
                ..StopVisit::default()
            };
        }
        &mut self.stop_visit
    }

    /// Speak the selected stop and the HOS purpose when this press chose it.
    fn speak_selected_rest_stop(
        &mut self,
        ctx: &mut GameContext,
        stop: &RoadStop,
        repeated: bool,
        hos_purpose: Option<(HosRestPurpose, bool)>,
        no_reachable_hos_stop: bool,
    ) {
        let ahead = 0.0f64.max(stop.at_mi - self.trip.position_mi);
        let distance = ctx.settings.distance_text(ahead, true);
        let exit_text = if stop.exit_label.is_empty() {
            String::new()
        } else {
            format!(" at {}", stop.exit_label)
        };
        // What the stopping assistance does, and when the exit will be
        // called, is the manual's to explain (owner, 2026-10-14). The line
        // owes only the stop and, when the driver stops, that job.
        let assist = if ctx.settings.destination_approach_assist {
            ""
        } else {
            " Stop at the entrance."
        };
        let prefix = if repeated {
            if self.selected_stop_break {
                "Still planned for a 30-minute break"
            } else {
                "Still planned"
            }
        } else {
            match hos_purpose {
                Some((HosRestPurpose::Break, true)) => {
                    "Break stop planned, the last one before your break limit"
                }
                Some((HosRestPurpose::Break, false)) => "Break stop planned",
                Some((HosRestPurpose::SleepForBreak, true)) => {
                    "Sleep stop planned for your break, the last one before your break limit"
                }
                Some((HosRestPurpose::Sleep, true)) => {
                    "Sleep stop planned, the last one before your sleep limit"
                }
                Some((HosRestPurpose::SleepForBreak, false)) => "Sleep stop planned for your break",
                Some((HosRestPurpose::Sleep, false)) => "Sleep stop planned",
                None if no_reachable_hos_stop => {
                    "Nearest sleep stop planned, but none is reachable before your hours limit. \
                     Stop somewhere safe sooner"
                }
                None => "Sleep stop planned",
            }
        };
        // Inside the exit window the signal is the next thing to do; beyond
        // it the exit is called when it comes.
        let next_step = if ahead <= self.exit_window_mi() {
            format!(" {}", self.exit_signal_instruction())
        } else {
            String::new()
        };
        let message = format!(
            "{prefix}, {}, {distance} ahead{exit_text}.{next_step}{assist}",
            stop.spoken_name()
        );
        self.set_status(message.clone());
        self.say_plain(ctx, message);
    }

    /// `_clear_selected_stop_intent()`.
    pub fn clear_selected_stop_intent(&mut self) {
        if self.selected_stop_assist_brake > 0.0 {
            if self.trip.truck.brake <= self.selected_stop_assist_brake + 1e-6 {
                self.trip.truck.brake = 0.0;
            }
            self.selected_stop_assist_brake = 0.0;
        }
        self.selected_stop_key = None;
        self.selected_stop_break = false;
        self.selected_stop_assist_armed = false;
        self.selected_stop_assist_said = false;
    }

    /// `_open_poi_stop(stop, *, settle=False, preferred_rest=None)`.
    pub fn open_poi_stop(
        &mut self,
        ctx: &mut GameContext,
        stop: &RoadStop,
        settle: bool,
        preferred_rest: Option<RestFocus>,
    ) {
        // Secure the truck before handing off to the stop menu: zero the
        // throttle, apply the service brake, and set the parking brake. A truck
        // that rolled in just under the docking threshold (or idled in gear)
        // would otherwise keep creeping while the driver rests -- napping while
        // the rig drifts down the freeway. Mirrors the pickup/delivery arrivals.
        if !secure_truck_for_stopped_menu(self, ctx) {
            self.say_plain(ctx, "Come to a complete stop first.");
            return;
        }
        let selected_rest_intent = self.is_selected_stop(Some(stop));
        let preferred_rest = preferred_rest.unwrap_or_else(|| {
            if !selected_rest_intent {
                RestFocus::Default
            } else if self.selected_stop_break
                && stop.actions.iter().any(|action| action == "break")
            {
                RestFocus::Break
            } else {
                RestFocus::Sleep
            }
        });
        if selected_rest_intent {
            self.clear_selected_stop_intent();
        }
        if self.trip.is_planned(stop) {
            // Plan fulfilled; the stop menu announces itself.
            self.trip.planned_stop_key = None;
        }

        if settle {
            // A POI stop that pulls in through this wait is a menu-driven
            // stop like a roadside inspection: the frame loop that eases
            // revs down between frames stops the instant the wait state
            // takes over, so without this the engine audio froze at
            // whatever rev the approach left it at for the whole stop.
            self.settle_engine_to_idle(ctx);
            advance_rest_clock(self, ctx, STOP_PULL_IN_MIN, None, "");
            hos_mut_of(ctx).on_duty(STOP_PULL_IN_MIN);
            let snapshot = self.snapshot(ctx);
            profile_mut_of(ctx).active_trip = Some(snapshot);
            ctx.save_profile();

            let name = stop.spoken_name();
            let stop = stop.clone();
            ctx.push_state(
                TimedMessageState::new(
                    "Pulling into stop",
                    &format!("Stopped at {name}. Menu opening."),
                    &format!("Stopped at {name}. Menu opening."),
                    STOP_PULL_IN_WAIT_S,
                    move |ctx: &mut GameContext| {
                        ctx.pop_state();
                        with_drive(ctx, |drive, ctx| {
                            drive.open_poi_stop(ctx, &stop, false, Some(preferred_rest));
                        });
                    },
                )
                .sound_key(Some("ui/notify")),
            );
            return;
        }

        let can_sleep = stop.actions.iter().any(|action| action == "sleep");
        if can_sleep
            && hos::parking_is_full(
                self.trip_seed,
                stop.at_mi,
                self.trip.local_hour(),
                stop.parking_spaces,
            )
        {
            self.push_parking_full_state(ctx, stop);
            return;
        }
        self.push_rest_stop_state(ctx, stop, preferred_rest);
        if matches!(
            stop.stop_type.as_str(),
            "truck_stop"
                | "travel_center"
                | "fuel_station"
                | "service_plaza"
                | "public_rest_area"
                | "truck_parking"
        ) {
            ctx.award_achievement("first_rest_stop");
        }
    }
}
