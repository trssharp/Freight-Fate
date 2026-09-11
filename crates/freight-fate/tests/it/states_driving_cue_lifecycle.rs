use super::*;

#[test]
fn steering_blinker_releases_when_wheel_centers_or_exit_position_is_set() {
    for exit_ready in [false, true] {
        let mut app = TestApp::new();
        let mut d = a_steering_drive(&mut app);
        let _tape = CueAudio::install(&mut app);
        if exit_ready {
            signal_for_the_exit(&mut d);
        }
        d.lane.offset = 0.2;
        arm(&mut d, &mut app, 1.0);
        assert!(app.ctx.audio.cue_held("vehicle/turn_signal"));
        if exit_ready {
            d.exit_lane_alignment = EXIT_LANE_READY;
        } else {
            d.lane.steering = 0.0;
        }
        d.update_steering_lane_cue(&mut app.ctx, 1.0 / 60.0);
        assert!(!app.ctx.audio.cue_held("vehicle/turn_signal"));
    }
}

// -- the steering lane cue (test_lane_position_cue.py) --------------------------------

fn a_steering_drive(app: &mut TestApp) -> DrivingState {
    let mut d = a_drive(app);
    app.ctx.settings.lane_keeping = "off".to_string(); // the lane work is the driver's
    d.trip.truck.velocity_mps = 25.0; // rolling, well over the cue's floor
    d
}

/// `_arm(driving, direction)`: hold the wheel long enough that this is a
/// move, not a correction.
fn arm(d: &mut DrivingState, app: &mut TestApp, direction: f64) {
    d.lane.steering = direction;
    d.update_steering_lane_cue(&mut app.ctx, STEER_CUE_ARM_S);
}

/// `_signal_for_the_exit(driving)`: an armed route exit, without needing a
/// real stop on this leg.
fn signal_for_the_exit(d: &mut DrivingState) {
    d.exit_stop = Some(RoadStop::new("Test Exit", 30.0, "travel_center"));
    d.exit_signal_on = true;
    d.lane.lane = 0; // ramps peel off the right lane
}

#[test]
fn test_holding_the_arrow_plays_the_blinker_panned_to_the_lane() {
    let mut app = TestApp::new();
    let mut d = a_steering_drive(&mut app);
    let tape = CueAudio::install(&mut app);
    d.lane.offset = 0.6; // already right of centre and still going right
    d.lane.steering = 1.0;

    d.update_steering_lane_cue(&mut app.ctx, STEER_CUE_ARM_S - 0.1);
    assert!(tape.calls().is_empty()); // a nudge of the wheel is not a move

    d.update_steering_lane_cue(&mut app.ctx, 0.2);
    let (key, _, pan) = tape.last();
    assert_eq!(key, "vehicle/turn_signal");
    assert!((pan - 0.6).abs() < 1e-9);

    // It keeps time for as long as the wheel is held, and follows the truck.
    d.lane.offset = 0.95;
    d.update_steering_lane_cue(&mut app.ctx, STEER_CUE_TOCK_S);
    let (key, _, pan) = tape.last();
    assert_eq!(key, "vehicle/turn_signal");
    assert!((pan - 0.95).abs() < 1e-9);
}

#[test]
fn test_the_tock_scales_with_the_lane_cue_loudness_setting() {
    let mut app = TestApp::new();
    let mut d = a_steering_drive(&mut app);
    let tape = CueAudio::install(&mut app);
    app.ctx.settings.lane_cue_loudness = "subtle".to_string();
    arm(&mut d, &mut app, 1.0);
    let (key, volume, _) = tape.last();
    assert_eq!(key, "vehicle/turn_signal");
    assert!((volume - 0.5 * 0.6).abs() < 1e-9);

    d.lane.steering = 0.0;
    tape.clear();
    d.update_steering_lane_cue(&mut app.ctx, 1.0 / 60.0);
    let (key, volume, _) = tape.last();
    assert_eq!(key, SIGNAL);
    assert!((volume - 0.45 * 0.6).abs() < 1e-9);
}

#[test]
fn test_letting_go_of_the_wheel_cancels_the_signal() {
    let mut app = TestApp::new();
    let mut d = a_steering_drive(&mut app);
    let tape = CueAudio::install(&mut app);
    arm(&mut d, &mut app, 1.0);
    assert_eq!(
        tape.keys().first().map(String::as_str),
        Some("vehicle/turn_signal")
    );

    tape.clear();
    d.lane.steering = 0.0; // straightened out: the move is over
    d.update_steering_lane_cue(&mut app.ctx, 1.0 / 60.0);
    // centred, quieter
    assert_eq!(tape.calls(), vec![(SIGNAL.to_string(), 0.45, 0.0)]);
    assert!(!d.steer_cue_active);

    // And it stays over: no second click, no stray tocks.
    tape.clear();
    for _ in 0..120 {
        d.update_steering_lane_cue(&mut app.ctx, 1.0 / 60.0);
    }
    assert!(tape.calls().is_empty());
}

#[test]
fn test_a_nudge_of_the_wheel_never_clicks() {
    // A drift correction is not a manoeuvre, so it gets no cue and no click.
    let mut app = TestApp::new();
    let mut d = a_steering_drive(&mut app);
    let tape = CueAudio::install(&mut app);
    d.lane.steering = -1.0;
    d.update_steering_lane_cue(&mut app.ctx, STEER_CUE_ARM_S - 0.2);
    d.lane.steering = 0.0;
    d.update_steering_lane_cue(&mut app.ctx, 1.0 / 60.0);
    assert!(tape.calls().is_empty());
}

#[test]
fn test_the_lane_change_ends_with_the_click_after_the_line_is_crossed() {
    let mut app = TestApp::new();
    let mut d = a_steering_drive(&mut app);
    let tape = CueAudio::install(&mut app);
    d.lane.lane = 0;
    arm(&mut d, &mut app, -1.0); // holding Left, moving toward the left lane
    d.lane.offset = -0.9;
    d.update_steering_lane_cue(&mut app.ctx, STEER_CUE_TOCK_S);
    assert!((tape.last().2 + 0.9).abs() < 1e-9); // heard sliding left

    // The tires roll the line, the lane model re-centres in the new lane,
    // and the cue follows the truck through the settle.
    d.lane.lane = 1;
    d.lane.offset = -0.9 + LANE_WIDTH;
    d.update_steering_lane_cue(&mut app.ctx, STEER_CUE_TOCK_S);
    let (key, volume, pan) = tape.last();
    assert_eq!(key, "vehicle/turn_signal");
    assert!((volume - 0.5).abs() < 1e-9);
    assert!((pan - 1.0).abs() < 1e-9);

    tape.clear();
    d.lane.steering = 0.0; // straightened up in the new lane
    d.update_steering_lane_cue(&mut app.ctx, 1.0 / 60.0);
    assert_eq!(tape.keys(), vec![SIGNAL.to_string()]);
}

#[test]
fn test_the_beat_quickens_as_the_exit_lane_position_fills() {
    let mut app = TestApp::new();
    let mut d = a_steering_drive(&mut app);
    let _tape = CueAudio::install(&mut app);
    signal_for_the_exit(&mut d);
    d.exit_lane_alignment = 0.0;
    d.lane.offset = 0.0;
    arm(&mut d, &mut app, 1.0);
    let wide = d.steer_cue_timer;
    assert!((wide - STEER_CUE_TOCK_S).abs() < 1e-9);

    d.exit_lane_alignment = EXIT_LANE_READY - 0.05; // nearly there
    d.update_steering_lane_cue(&mut app.ctx, wide);
    assert!(d.steer_cue_timer < wide / 2.0);
}

#[test]
fn test_reaching_the_exit_position_clicks_off_with_the_wheel_still_held() {
    // "Far enough right now" arrives as the signal cancelling, not a sentence.
    let mut app = TestApp::new();
    let mut d = a_steering_drive(&mut app);
    let tape = CueAudio::install(&mut app);
    signal_for_the_exit(&mut d);
    d.exit_lane_alignment = 0.5;
    arm(&mut d, &mut app, 1.0);
    assert_eq!(tape.last().0, "vehicle/turn_signal");

    tape.clear();
    d.exit_lane_alignment = EXIT_LANE_READY; // the exit has the lane it needs
    d.update_steering_lane_cue(&mut app.ctx, 1.0 / 60.0);
    assert_eq!(tape.calls(), vec![(SIGNAL.to_string(), 0.45, 0.0)]);
    // the wheel is still over; the position is what ended it
    assert_eq!(d.lane.steering, 1.0);
    assert!(!d.steer_cue_active);

    // Holding Right past the mark does not start it up again.
    tape.clear();
    for _ in 0..120 {
        d.update_steering_lane_cue(&mut app.ctx, 1.0 / 60.0);
    }
    assert!(tape.calls().is_empty());
}

#[test]
fn test_abandoning_the_exit_line_up_clicks_off_too() {
    let mut app = TestApp::new();
    let mut d = a_steering_drive(&mut app);
    let tape = CueAudio::install(&mut app);
    signal_for_the_exit(&mut d);
    d.exit_lane_alignment = 0.4;
    arm(&mut d, &mut app, 1.0);
    assert_eq!(tape.last().0, "vehicle/turn_signal");

    tape.clear();
    d.lane.steering = 0.0;
    d.exit_lane_alignment = 0.0; // steered back and let the commitment bleed away
    d.update_steering_lane_cue(&mut app.ctx, 1.0 / 60.0);
    assert_eq!(tape.keys(), vec![SIGNAL.to_string()]);
}

#[test]
fn test_the_cue_stays_silent_under_lane_keeping_and_below_the_speed_floor() {
    let mut app = TestApp::new();
    let mut d = a_steering_drive(&mut app);
    let tape = CueAudio::install(&mut app);
    // the truck holds the lane and takes the exit
    app.ctx.settings.lane_keeping = "full".to_string();
    signal_for_the_exit(&mut d);
    d.exit_lane_alignment = 0.5;
    for _ in 0..120 {
        d.lane.steering = 1.0;
        d.update_steering_lane_cue(&mut app.ctx, 1.0 / 60.0);
    }
    assert!(tape.calls().is_empty());

    app.ctx.settings.lane_keeping = "off".to_string();
    d.trip.truck.velocity_mps = 0.5; // about a walking pace: nothing to steer yet
    for _ in 0..120 {
        d.lane.steering = 1.0;
        d.update_steering_lane_cue(&mut app.ctx, 1.0 / 60.0);
    }
    assert!(tape.calls().is_empty());
}

#[test]
fn test_it_does_not_double_the_locator_the_driver_already_turned_on() {
    let mut app = TestApp::new();
    let mut d = a_steering_drive(&mut app);
    let tape = CueAudio::install(&mut app);
    d.lane_locator_on = true; // I is already ticking the same tock
    signal_for_the_exit(&mut d);
    d.exit_lane_alignment = 0.5;
    for _ in 0..120 {
        d.lane.steering = 1.0;
        d.update_steering_lane_cue(&mut app.ctx, 1.0 / 60.0);
    }
    assert!(tape.calls().is_empty());
    d.update_lane_locator_audio(&mut app.ctx, 0.9);
    assert_eq!(tape.last().0, LOCATOR);
}

#[test]
fn test_the_cue_cannot_survive_the_drive_losing_the_frame() {
    // A menu over the drive lets the latch lapse, and the move ends in
    // silence -- never a signal cancelling over the pause screen.
    let mut app = TestApp::new();
    let mut d = a_steering_drive(&mut app);
    let tape = CueAudio::install(&mut app);
    arm(&mut d, &mut app, 1.0);
    assert!(app.ctx.audio.cue_held(STEER_CUE_HOLD));

    // A menu owns the frames now: the driving state stops updating while
    // the audio clock keeps running.
    app.ctx.audio.update(0.5);
    assert!(!app.ctx.audio.cue_held(STEER_CUE_HOLD));

    tape.clear();
    d.lane.steering = 0.0;
    d.update_steering_lane_cue(&mut app.ctx, 1.0 / 60.0);
    assert!(tape.calls().is_empty());
    assert!(!d.steer_cue_active);
}

#[test]
fn test_the_whole_manoeuvre_adds_no_speech() {
    let mut app = TestApp::new();
    let mut d = a_steering_drive(&mut app);
    let _tape = CueAudio::install(&mut app);
    signal_for_the_exit(&mut d);
    d.exit_lane_alignment = 0.3;
    app.clear_speech();
    for _ in 0..240 {
        d.lane.steering = 1.0;
        d.exit_lane_alignment = 1.0f64.min(d.exit_lane_alignment + 1.0 / 60.0);
        d.update_steering_lane_cue(&mut app.ctx, 1.0 / 60.0);
    }
    d.lane.steering = 0.0;
    d.update_steering_lane_cue(&mut app.ctx, 1.0 / 60.0);
    assert!(app.main_lines().is_empty());
    assert!(app.event_lines().is_empty());
}
