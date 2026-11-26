// SPDX-License-Identifier: AGPL-3.0-only

//! A stroke kept in a file and played back, for a table of what it cost.
//!
//! The build machine this was written on has no graphics card: it draws on a
//! CPU. So nothing about a frame on real hardware can be measured there, and
//! "it feels slow on my model" cannot be handed over as anything but words.
//! This is how it is handed over. Record the pointer on the machine and the
//! model where it is slow, and the same events can be played back there, or
//! after a change, with no hand on the mouse, and the time read off by
//! phase. See `phases.rs` for the phases.
//!
//! ```text
//! HANI_RECORD=stroke.txt hanisculpt          # sculpt, then quit
//! HANI_REPLAY=stroke.txt hanisculpt          # plays, prints, quits
//! HANI_REPLAY=stroke.txt HANI_REPLAY_PROJECT=big.hani hanisculpt
//! ```
//!
//! **What a recording holds**: the pointer events of the viewport, where the
//! frames fell between them, and at every press of the left button the
//! camera and the brush in hand by its name, its size and its strength.
//! Events a frame are kept as they were, because how many events arrive
//! between two frames is part of what is being measured.
//!
//! **What it does not hold**: the keyboard, the panel, the pen's pressure,
//! or the document. A replay is against the project named beside it, or
//! against the ball the application opens with. A brush with settings of its
//! own that is not on the palette comes back as the brush it goes by.
//!
//! The file is lines of text so that one can be written or trimmed by hand.

use std::io::Write;
use std::path::{Path, PathBuf};
use web_time::Instant;

use glam::Vec3;
use hani_core::BrushKind;
use iced::{Task, Vector};

use super::phases::{self, Frame};
use super::{DragKind, Hani};
use crate::message::{Message, PointerButton, PointerEvent, StudioMessage};

/// The first line of a recording.
const HEADING: &str = "hani-stroke 1";

/// Frames drawn before a replay begins, so that the model it plays on has
/// been meshed, taken in by the renderer and drawn.
const SETTLE_FRAMES: u32 = 90;

/// One line of a recording.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Step {
    /// A frame was drawn here.
    Frame,
    Pointer(Pointed),
    /// The camera at a press.
    Camera {
        target: Vec3,
        distance: f32,
        yaw: f32,
        pitch: f32,
        roll: f32,
    },
    /// The brush in hand at a press.
    Brush {
        radius: f32,
        strength: f32,
        name: String,
    },
}

/// A pointer event, as [`PointerEvent`] with something to compare.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Pointed {
    Moved { at: [f32; 2], size: [f32; 2] },
    Pressed { button: PointerButton, at: [f32; 2], size: [f32; 2] },
    Released { button: PointerButton },
    Scrolled { amount: f32, at: [f32; 2], size: [f32; 2] },
    Modifiers { shift: bool, control: bool, alt: bool },
}

impl From<PointerEvent> for Pointed {
    fn from(event: PointerEvent) -> Self {
        let pair = |vector: Vector| [vector.x, vector.y];
        match event {
            PointerEvent::Moved { position, size } => {
                Pointed::Moved { at: pair(position), size: pair(size) }
            }
            PointerEvent::Pressed { button, position, size } => {
                Pointed::Pressed { button, at: pair(position), size: pair(size) }
            }
            PointerEvent::Released { button } => Pointed::Released { button },
            PointerEvent::Scrolled { amount, position, size } => {
                Pointed::Scrolled { amount, at: pair(position), size: pair(size) }
            }
            PointerEvent::Modifiers { shift, control, alt } => {
                Pointed::Modifiers { shift, control, alt }
            }
        }
    }
}

impl From<Pointed> for PointerEvent {
    fn from(pointed: Pointed) -> Self {
        let vector = |pair: [f32; 2]| Vector::new(pair[0], pair[1]);
        match pointed {
            Pointed::Moved { at, size } => {
                PointerEvent::Moved { position: vector(at), size: vector(size) }
            }
            Pointed::Pressed { button, at, size } => {
                PointerEvent::Pressed { button, position: vector(at), size: vector(size) }
            }
            Pointed::Released { button } => PointerEvent::Released { button },
            Pointed::Scrolled { amount, at, size } => {
                PointerEvent::Scrolled { amount, position: vector(at), size: vector(size) }
            }
            Pointed::Modifiers { shift, control, alt } => {
                PointerEvent::Modifiers { shift, control, alt }
            }
        }
    }
}

fn button_name(button: PointerButton) -> &'static str {
    match button {
        PointerButton::Left => "left",
        PointerButton::Right => "right",
        PointerButton::Middle => "middle",
    }
}

fn button_named(name: &str) -> Option<PointerButton> {
    match name {
        "left" => Some(PointerButton::Left),
        "right" => Some(PointerButton::Right),
        "middle" => Some(PointerButton::Middle),
        _ => None,
    }
}

impl Step {
    /// The step as one line of a recording.
    fn line(&self) -> String {
        let flag = |on: bool| u8::from(on);
        match self {
            Step::Frame => "frame".to_string(),
            Step::Pointer(Pointed::Moved { at, size }) => {
                format!("move {} {} {} {}", at[0], at[1], size[0], size[1])
            }
            Step::Pointer(Pointed::Pressed { button, at, size }) => format!(
                "press {} {} {} {} {}",
                button_name(*button),
                at[0],
                at[1],
                size[0],
                size[1]
            ),
            Step::Pointer(Pointed::Released { button }) => {
                format!("release {}", button_name(*button))
            }
            Step::Pointer(Pointed::Scrolled { amount, at, size }) => {
                format!("scroll {amount} {} {} {} {}", at[0], at[1], size[0], size[1])
            }
            Step::Pointer(Pointed::Modifiers { shift, control, alt }) => {
                format!("keys {} {} {}", flag(*shift), flag(*control), flag(*alt))
            }
            Step::Camera { target, distance, yaw, pitch, roll } => format!(
                "camera {} {} {} {distance} {yaw} {pitch} {roll}",
                target.x, target.y, target.z
            ),
            Step::Brush { radius, strength, name } => format!("brush {radius} {strength} {name}"),
        }
    }

    /// Read one line. `None` for a line that is not a step.
    fn parse(line: &str) -> Option<Step> {
        let mut words = line.split_whitespace();
        let word = words.next()?;
        // The numbers that follow, as many as are asked for.
        fn numbers<'a, const N: usize>(
            words: &mut impl Iterator<Item = &'a str>,
        ) -> Option<[f32; N]> {
            let mut read = [0.0f32; N];
            for slot in &mut read {
                *slot = words.next()?.parse().ok().filter(|value: &f32| value.is_finite())?;
            }
            Some(read)
        }
        Some(match word {
            "frame" => Step::Frame,
            "move" => {
                let [x, y, w, h] = numbers(&mut words)?;
                Step::Pointer(Pointed::Moved { at: [x, y], size: [w, h] })
            }
            "press" => {
                let button = button_named(words.next()?)?;
                let [x, y, w, h] = numbers(&mut words)?;
                Step::Pointer(Pointed::Pressed { button, at: [x, y], size: [w, h] })
            }
            "release" => Step::Pointer(Pointed::Released { button: button_named(words.next()?)? }),
            "scroll" => {
                let [amount, x, y, w, h] = numbers(&mut words)?;
                Step::Pointer(Pointed::Scrolled { amount, at: [x, y], size: [w, h] })
            }
            "keys" => {
                let [shift, control, alt] = numbers(&mut words)?;
                Step::Pointer(Pointed::Modifiers {
                    shift: shift != 0.0,
                    control: control != 0.0,
                    alt: alt != 0.0,
                })
            }
            "camera" => {
                let [x, y, z, distance, yaw, pitch, roll] = numbers(&mut words)?;
                Step::Camera { target: Vec3::new(x, y, z), distance, yaw, pitch, roll }
            }
            "brush" => {
                let [radius, strength] = numbers(&mut words)?;
                Step::Brush { radius, strength, name: words.collect::<Vec<_>>().join(" ") }
            }
            _ => return None,
        })
    }
}

/// A recording read whole.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Recording {
    pub steps: Vec<Step>,
}

impl Recording {
    /// Read a recording's text. Lines that begin with `#` and empty lines
    /// are passed over; any other line that cannot be read is an error that
    /// says which.
    pub(super) fn parse(text: &str) -> Result<Recording, String> {
        let mut lines = text.lines().enumerate();
        match lines.next() {
            Some((_, first)) if first.trim() == HEADING => {}
            _ => return Err(format!("not a recording: the first line is not \"{HEADING}\"")),
        }
        let mut steps = Vec::new();
        for (index, line) in lines {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let step = Step::parse(line)
                .ok_or_else(|| format!("line {} cannot be read: {line}", index + 1))?;
            steps.push(step);
        }
        Ok(Recording { steps })
    }

    #[cfg(test)]
    pub(super) fn text(&self) -> String {
        let mut text = String::from(HEADING);
        text.push('\n');
        for step in &self.steps {
            text.push_str(&step.line());
            text.push('\n');
        }
        text
    }

    /// One drag of the left button through `points`, `per_frame` moves to a
    /// frame, in a viewport of `size`. For a measurement that needs no hand.
    #[cfg(test)]
    pub(super) fn drag(points: &[[f32; 2]], size: [f32; 2], per_frame: usize) -> Recording {
        let mut steps = Vec::new();
        let Some((first, rest)) = points.split_first() else {
            return Recording::default();
        };
        steps.push(Step::Pointer(Pointed::Moved { at: *first, size }));
        steps.push(Step::Frame);
        steps.push(Step::Pointer(Pointed::Pressed {
            button: PointerButton::Left,
            at: *first,
            size,
        }));
        steps.push(Step::Frame);
        for moves in rest.chunks(per_frame.max(1)) {
            for at in moves {
                steps.push(Step::Pointer(Pointed::Moved { at: *at, size }));
            }
            steps.push(Step::Frame);
        }
        steps.push(Step::Pointer(Pointed::Released { button: PointerButton::Left }));
        steps.push(Step::Frame);
        Recording { steps }
    }
}

/// A recording being written.
#[derive(Debug)]
pub(super) struct Recorder {
    file: std::io::BufWriter<std::fs::File>,
    /// Whether anything has been written since the last frame line, so that
    /// an idle minute is one line and not three thousand.
    since_frame: bool,
}

impl Recorder {
    fn create(path: &Path) -> std::io::Result<Recorder> {
        let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
        writeln!(file, "{HEADING}")?;
        Ok(Recorder { file, since_frame: false })
    }

    fn write(&mut self, step: &Step) {
        if *step == Step::Frame {
            if !self.since_frame {
                return;
            }
            self.since_frame = false;
            // A frame line is where a stroke's worth is safe to lose no more
            // than: the application may be closed at any time.
            let _ = writeln!(self.file, "{}", step.line());
            let _ = self.file.flush();
            return;
        }
        self.since_frame = true;
        let _ = writeln!(self.file, "{}", step.line());
    }
}

/// A recording being played.
#[derive(Debug)]
pub(super) struct Player {
    steps: std::vec::IntoIter<Step>,
    /// Frames still to draw before the first step.
    settle: u32,
    began: Option<Instant>,
    /// Close the application when it is over: a replay asked for at launch.
    then_quit: bool,
}

/// The recorder and the player, either of which may be there.
#[derive(Debug, Default)]
pub(super) struct Replay {
    recorder: Option<Recorder>,
    player: Option<Player>,
    /// The frames of the replay that last ended.
    finished: Vec<Frame>,
}

impl Replay {
    #[cfg(test)]
    pub(super) fn playing(&self) -> bool {
        self.player.is_some()
    }
}

impl Hani {
    /// What the environment asked for at launch. See the module's heading.
    pub(super) fn replay_from_environment(&mut self) {
        let path = |key: &str| std::env::var_os(key).map(PathBuf::from);
        if let Some(log) = path("HANI_PHASE_LOG") {
            match self.phases.log_to(&log) {
                Ok(()) => log::info!("a line a frame of a stroke goes to {}", log.display()),
                Err(why) => log::warn!("no phase log at {}: {why}", log.display()),
            }
        }
        if let Some(to) = path("HANI_RECORD") {
            match Recorder::create(&to) {
                Ok(recorder) => {
                    log::info!("recording the pointer to {}", to.display());
                    self.replay.recorder = Some(recorder);
                }
                Err(why) => log::warn!("no recording at {}: {why}", to.display()),
            }
        }
        if let Some(from) = path("HANI_REPLAY") {
            let read = std::fs::read_to_string(&from)
                .map_err(|why| why.to_string())
                .and_then(|text| Recording::parse(&text));
            match read {
                Ok(recording) => {
                    // Nothing is in the way of a replay: not the welcome, and
                    // not a card over the model.
                    self.welcome = false;
                    if let Some(project) = path("HANI_REPLAY_PROJECT") {
                        self.open_project(&project);
                    }
                    log::info!("replaying {} steps of {}", recording.steps.len(), from.display());
                    self.begin_replay(recording, SETTLE_FRAMES, true);
                }
                Err(why) => {
                    log::error!("no replay of {}: {why}", from.display());
                    eprintln!("no replay of {}: {why}", from.display());
                }
            }
        }
    }

    pub(super) fn begin_replay(&mut self, recording: Recording, settle: u32, then_quit: bool) {
        self.replay.player =
            Some(Player { steps: recording.steps.into_iter(), settle, began: None, then_quit });
    }

    /// A pointer event came in: into the recording, if there is one.
    pub(super) fn record_pointer(&mut self, event: PointerEvent) {
        if self.replay.recorder.is_none() {
            return;
        }
        // What a press is made with, so that a replay makes it with the same.
        let before: Vec<Step> =
            if matches!(event, PointerEvent::Pressed { button: PointerButton::Left, .. }) {
                vec![
                    Step::Camera {
                        target: self.camera.target,
                        distance: self.camera.distance,
                        yaw: self.camera.yaw,
                        pitch: self.camera.pitch,
                        roll: self.camera.roll,
                    },
                    Step::Brush {
                        radius: self.brush.radius,
                        strength: self.brush.strength,
                        name: self.live_brush_name(),
                    },
                ]
            } else {
                Vec::new()
            };
        if let Some(recorder) = &mut self.replay.recorder {
            for step in &before {
                recorder.write(step);
            }
            recorder.write(&Step::Pointer(event.into()));
        }
    }

    /// A frame was drawn: into the recording, and the next frame's worth of a
    /// replay out of it. Returns the task that closes the application when a
    /// replay asked for at launch is over.
    pub(super) fn replay_frame(&mut self) -> Task<Message> {
        if let Some(recorder) = &mut self.replay.recorder {
            recorder.write(&Step::Frame);
        }
        let Some(player) = &mut self.replay.player else {
            return Task::none();
        };
        if player.settle > 0 {
            player.settle -= 1;
            return Task::none();
        }
        if player.began.is_none() {
            player.began = Some(Instant::now());
            self.phases.begin_run();
        }
        // Up to the next frame line, or to the end.
        let mut steps = Vec::new();
        let mut over = true;
        if let Some(player) = &mut self.replay.player {
            for step in player.steps.by_ref() {
                if step == Step::Frame {
                    over = false;
                    break;
                }
                steps.push(step);
            }
        }
        for step in steps {
            self.play(step);
        }
        if !over {
            return Task::none();
        }
        let Some(player) = self.replay.player.take() else {
            return Task::none();
        };
        // A stroke left down by a recording that was cut short is ended, so
        // that its undo entry is in the table and the document is whole.
        if self.drag.is_some_and(|drag| matches!(drag.kind, DragKind::Sculpt(_))) {
            self.pointer_in(PointerEvent::Released { button: PointerButton::Left });
        }
        let frames = self.phases.end_run();
        let seconds = player.began.map_or(0.0, |began| began.elapsed().as_secs_f32());
        let report = format!(
            "replay over in {seconds:.2} s on {}\n{}",
            self.shared.adapter_summary(),
            phases::table(&frames)
        );
        log::info!("{report}");
        if player.then_quit {
            // To the terminal as well, whatever the log's level: it is what
            // the replay was run for.
            println!("{report}");
        }
        self.phases.flush();
        self.replay.finished = frames;
        self.status = "the replay is over: its table is on the terminal".to_string();
        if player.then_quit { iced::exit() } else { Task::none() }
    }

    /// Do one step of a recording.
    fn play(&mut self, step: Step) {
        match step {
            Step::Frame => {}
            Step::Pointer(pointed) => self.pointer_in(pointed.into()),
            Step::Camera { target, distance, yaw, pitch, roll } => {
                self.flight = None;
                self.camera.target = target;
                self.camera.distance = distance;
                self.camera.yaw = yaw;
                self.camera.pitch = pitch;
                self.camera.roll = roll;
                self.publish_camera();
            }
            Step::Brush { radius, strength, name } => self.take_up_by_name(&name, radius, strength),
        }
    }

    /// Take a brush up by the name the panel's heading gave it.
    fn take_up_by_name(&mut self, name: &str, radius: f32, strength: f32) {
        let on_palette = self
            .studio
            .presets
            .palette()
            .iter()
            .position(|(preset, _)| preset.name.eq_ignore_ascii_case(name));
        let classic =
            BrushKind::ALL.into_iter().find(|kind| kind.label().eq_ignore_ascii_case(name));
        let chosen = match (on_palette, classic) {
            (Some(index), _) => Some(Message::Studio(StudioMessage::PresetChosen(index))),
            (None, Some(kind)) => Some(Message::BrushKindChanged(kind)),
            (None, None) => {
                log::warn!("the recording's brush \"{name}\" is not one of this palette's");
                None
            }
        };
        for message in chosen
            .into_iter()
            .chain([Message::BrushRadiusChanged(radius), Message::BrushStrengthChanged(strength)])
        {
            drop(self.update(message));
        }
    }

    /// Play a recording to its end with no window, a frame tick at each of
    /// its frame lines, and return what each frame cost. For tests and for
    /// measuring on a machine with nothing to draw on.
    #[cfg(test)]
    pub(super) fn replay_now(&mut self, recording: Recording) -> Vec<Frame> {
        self.begin_replay(recording, 0, false);
        while self.replay.playing() {
            drop(self.update(Message::Frame));
        }
        std::mem::take(&mut self.replay.finished)
    }
}

#[cfg(test)]
mod tests {
    use super::super::phases::Phase;
    use super::*;
    use crate::tablet::Tablet;

    const SIZE: [f32; 2] = [800.0, 600.0];

    fn app() -> Hani {
        Hani::with_tablet(Tablet::inert())
    }

    /// A ring of points about the middle of the viewport, over the ball.
    fn ring(points: usize, radius: f32) -> Vec<[f32; 2]> {
        (0..=points)
            .map(|index| {
                let turn = index as f32 / points as f32 * std::f32::consts::TAU;
                [SIZE[0] / 2.0 + radius * turn.cos(), SIZE[1] / 2.0 + radius * turn.sin()]
            })
            .collect()
    }

    #[test]
    fn a_recording_reads_back_as_it_was_written() {
        let recording = Recording {
            steps: vec![
                Step::Camera {
                    target: Vec3::new(1.5, -2.25, 0.125),
                    distance: 80.0,
                    yaw: 0.5,
                    pitch: -0.25,
                    roll: 0.0,
                },
                Step::Brush { radius: 3.5, strength: 0.25, name: "CLAY STRIPS".to_string() },
                Step::Pointer(Pointed::Modifiers { shift: true, control: false, alt: true }),
                Step::Pointer(Pointed::Pressed {
                    button: PointerButton::Left,
                    at: [400.5, 300.25],
                    size: SIZE,
                }),
                Step::Frame,
                Step::Pointer(Pointed::Moved { at: [410.0, 300.0], size: SIZE }),
                Step::Pointer(Pointed::Scrolled { amount: -1.5, at: [1.0, 2.0], size: SIZE }),
                Step::Frame,
                Step::Pointer(Pointed::Released { button: PointerButton::Left }),
            ],
        };
        assert_eq!(Recording::parse(&recording.text()), Ok(recording));
    }

    #[test]
    fn a_line_that_cannot_be_read_is_named() {
        let text = format!("{HEADING}\n# a note\n\nmove 1 2 800 600\nmove 1 two 800 600\n");
        let why = Recording::parse(&text).expect_err("a word where a number goes");
        assert!(why.contains("line 5"), "{why}");
        assert!(Recording::parse("move 1 2 3 4\n").is_err(), "no heading and it was read");
    }

    /// The pointer is recorded as it comes in, with the camera and the brush
    /// at the press, and a run of idle frames is one line.
    #[test]
    fn what_is_recorded_is_what_came_in() {
        let directory = std::env::temp_dir().join(format!("hani-replay-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("stroke.txt");
        let played = Recording::drag(&ring(12, 40.0), SIZE, 3);

        let mut app = app();
        app.replay.recorder = Some(Recorder::create(&path).unwrap());
        for _ in 0..5 {
            drop(app.update(Message::Frame));
        }
        for step in &played.steps {
            match step {
                Step::Frame => drop(app.update(Message::Frame)),
                Step::Pointer(pointed) => drop(app.update(Message::Pointer((*pointed).into()))),
                _ => unreachable!("a drag is pointer events and frames"),
            }
        }
        app.replay.recorder = None;

        let kept = Recording::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let pointer_and_frames: Vec<Step> = kept
            .steps
            .iter()
            .filter(|step| matches!(step, Step::Frame | Step::Pointer(_)))
            .cloned()
            .collect();
        assert_eq!(pointer_and_frames, played.steps, "the idle frames at the start were kept");
        let press = kept
            .steps
            .iter()
            .position(|step| matches!(step, Step::Pointer(Pointed::Pressed { .. })))
            .expect("a press");
        assert!(matches!(kept.steps[press - 2], Step::Camera { .. }), "{:?}", kept.steps);
        assert!(
            matches!(&kept.steps[press - 1], Step::Brush { name, .. } if name == "DRAW"),
            "{:?}",
            kept.steps
        );
        let _ = std::fs::remove_dir_all(&directory);
    }

    /// A replay does to the document what the hand did: the same field, to
    /// the bit, from the same events.
    #[test]
    fn a_replay_does_what_the_hand_did() {
        let recording = Recording::drag(&ring(24, 60.0), SIZE, 2);

        let mut by_hand = app();
        for step in &recording.steps {
            match step {
                Step::Frame => drop(by_hand.update(Message::Frame)),
                Step::Pointer(pointed) => by_hand.on_pointer((*pointed).into()),
                _ => unreachable!(),
            }
        }

        let mut replayed = app();
        let frames = replayed.replay_now(recording.clone());
        let (one, other) = (by_hand.doc.active_volume(), replayed.doc.active_volume());
        let mut differing = 0;
        let mut carved = 0;
        for index in 0..60 * 60 * 60 {
            let cell = glam::IVec3::new(index % 60, index / 60 % 60, index / 3600);
            let point = (cell.as_vec3() - 30.0) * 0.8;
            let (a, b) = (one.sample_world(point), other.sample_world(point));
            differing += usize::from(a.to_bits() != b.to_bits());
            carved += usize::from(a.abs() < 2.5);
        }
        assert!(carved > 1000, "the points compared miss the surface: {carved} are near it");
        assert_eq!(differing, 0, "a replayed stroke left a different field");
        assert!(replayed.history_stats.undo_entries > 0, "the replay carved nothing");

        // One frame for each frame line, and the time is where it was spent.
        let lines = recording.steps.iter().filter(|step| **step == Step::Frame).count();
        assert_eq!(frames.len(), lines, "a frame for each frame line");
        let down = frames.iter().filter(|frame| frame.stroke).count();
        assert!(down >= 12, "only {down} frames had a stroke down");
        let total = |phase: Phase| frames.iter().map(|frame| frame.of(phase)).sum::<f32>();
        for phase in [Phase::Pick, Phase::Edit, Phase::Remesh, Phase::Overlay, Phase::Finish] {
            assert!(total(phase) > 0.0, "nothing was timed as {}", phase.label());
        }
        let stamps: u32 = frames.iter().map(|frame| frame.stamps).sum();
        let meshed: u32 = frames.iter().map(|frame| frame.meshed).sum();
        assert!(stamps > 20 && meshed > 20, "{stamps} stamps and {meshed} bricks meshed");
    }

    /// How many points of a grid through the ball two applications hold
    /// different values at, and how many of those points are near a surface.
    fn fields_differ(one: &Hani, other: &Hani) -> (usize, usize) {
        let (one, other) = (one.doc.active_volume(), other.doc.active_volume());
        let (mut differing, mut near) = (0, 0);
        for index in 0..60 * 60 * 60 {
            let cell = glam::IVec3::new(index % 60, index / 60 % 60, index / 3600);
            let point = (cell.as_vec3() - 30.0) * 0.8;
            let (a, b) = (one.sample_world(point), other.sample_world(point));
            differing += usize::from(a.to_bits() != b.to_bits());
            near += usize::from(a.abs() < 2.5);
        }
        (differing, near)
    }

    /// Moves that arrive between two frames are handled together: the field
    /// is what one move a frame leaves, to the bit, with the same stamps, and
    /// the bricks are meshed once for the lot.
    #[test]
    fn moves_between_two_frames_are_handled_together() {
        let points = ring(96, 70.0);
        let mut one_a_frame = app();
        let slow = one_a_frame.replay_now(Recording::drag(&points, SIZE, 1));
        let mut twelve_a_frame = app();
        let fast = twelve_a_frame.replay_now(Recording::drag(&points, SIZE, 12));

        let (differing, near) = fields_differ(&one_a_frame, &twelve_a_frame);
        assert!(near > 1000, "the points compared miss the surface");
        assert_eq!(differing, 0, "moves handled together left a different field");
        let sum = |frames: &[Frame], of: fn(&Frame) -> u32| frames.iter().map(of).sum::<u32>();
        assert_eq!(sum(&slow, |frame| frame.stamps), sum(&fast, |frame| frame.stamps));
        assert_eq!(sum(&slow, |frame| frame.events), sum(&fast, |frame| frame.events));
        let (slow_meshed, fast_meshed) =
            (sum(&slow, |frame| frame.meshed), sum(&fast, |frame| frame.meshed));
        assert!(
            fast_meshed * 2 < slow_meshed,
            "{fast_meshed} bricks meshed for twelve moves a frame, {slow_meshed} for one"
        );
        assert_eq!(
            one_a_frame.history_stats.undo_entries, twelve_a_frame.history_stats.undo_entries,
            "a stroke is one undo entry however its moves arrive"
        );
    }

    /// Moves that are waiting for a frame come before whatever arrives
    /// next: a release with no frame since the press ends the stroke those
    /// moves drew, and does not lose them.
    #[test]
    fn a_release_handles_the_moves_before_it() {
        let recording = Recording::drag(&ring(24, 60.0), SIZE, 1);
        let mut with_frames = app();
        let _ = with_frames.replay_now(recording.clone());

        let mut without = app();
        for step in &recording.steps {
            if let Step::Pointer(pointed) = step {
                drop(without.update(Message::Pointer((*pointed).into())));
            }
        }
        assert!(without.moves.is_empty(), "moves were left waiting after the release");
        let (differing, _) = fields_differ(&with_frames, &without);
        assert_eq!(differing, 0, "a stroke with no frame in it lost its moves");
        assert_eq!(without.history_stats.undo_entries, with_frames.history_stats.undo_entries);
    }

    /// The camera and the brush of a recording are taken up before its press.
    #[test]
    fn a_replay_takes_up_the_camera_and_the_brush() {
        let mut recording = Recording::drag(&ring(4, 30.0), SIZE, 1);
        let press = recording
            .steps
            .iter()
            .position(|step| matches!(step, Step::Pointer(Pointed::Pressed { .. })))
            .unwrap();
        recording.steps.splice(
            press..press,
            [
                Step::Camera {
                    target: Vec3::ZERO,
                    distance: 70.0,
                    yaw: 0.75,
                    pitch: 0.25,
                    roll: 0.0,
                },
                Step::Brush { radius: 4.5, strength: 0.25, name: "INFLATE".to_string() },
            ],
        );
        let mut app = app();
        let _ = app.replay_now(recording);
        assert_eq!(app.camera.yaw, 0.75);
        assert_eq!(app.camera.distance, 70.0);
        assert_eq!(app.brush.kind, BrushKind::Inflate);
        assert_eq!(app.brush.radius, 4.5);
        assert_eq!(app.brush.strength, 0.25);
    }

    /// What setting a shape down by a drag costs, with a move to a frame and
    /// with eight. Printed, not asserted.
    ///
    /// ```text
    /// cargo test --release -p hani-app -- --ignored --nocapture what_setting_a_shape_down_costs
    /// ```
    #[test]
    #[ignore = "a measurement, not a test"]
    fn what_setting_a_shape_down_costs() {
        use super::super::adding::Insert;
        use crate::message::AddMessage;
        let points: Vec<[f32; 2]> = (0..=48)
            .map(|step| [SIZE[0] / 2.0 + step as f32 * 2.5, SIZE[1] / 2.0 + step as f32 * 0.5])
            .collect();
        for per_frame in [1, 8] {
            let mut app = app();
            drop(app.update(Message::Resample(super::super::VOXEL_SIZE_MM / 2.0)));
            let ball = Insert::Shape(hani_core::solid::Form::Ball);
            drop(app.update(Message::Add(AddMessage::Chosen(Some(ball)))));
            drop(app.update(Message::Frame));
            let frames = app.replay_now(Recording::drag(&points, SIZE, per_frame));
            println!(
                "\n== a ball set down by a drag of 48 moves, {per_frame} to a frame ==\n{}",
                phases::table(&frames)
            );
        }
    }

    /// What a stroke costs by phase, on a model of a size worth asking
    /// about. Printed, not asserted: run it before and after a change.
    ///
    /// ```text
    /// cargo test --release -p hani-app -- --ignored --nocapture what_a_stroke_costs
    /// ```
    ///
    /// No window and no graphics card are involved, so the upload and encode
    /// rows are empty: those want a replay in the running application.
    #[test]
    #[ignore = "a measurement, not a test"]
    fn what_a_stroke_costs() {
        for (label, voxel, radius, per_frame) in [
            ("the ball it opens with, 3 mm brush", super::super::VOXEL_SIZE_MM, 3.0, 2),
            ("four times the voxels across, 3 mm brush", super::super::VOXEL_SIZE_MM / 4.0, 3.0, 2),
            (
                "four times the voxels across, 3 mm, 8 events a frame",
                super::super::VOXEL_SIZE_MM / 4.0,
                3.0,
                8,
            ),
            ("four times the voxels across, 8 mm brush", super::super::VOXEL_SIZE_MM / 4.0, 8.0, 2),
        ] {
            let mut app = app();
            if voxel != super::super::VOXEL_SIZE_MM {
                drop(app.update(Message::Resample(voxel)));
            }
            drop(app.update(Message::BrushRadiusChanged(radius)));
            drop(app.update(Message::BrushStrengthChanged(0.5)));
            drop(app.update(Message::Frame));
            let stats = app.doc.totals();
            let frames = app.replay_now(Recording::drag(&ring(240, 120.0), SIZE, per_frame));
            println!(
                "\n== {label}: {} dense bricks, voxel {voxel} mm ==\n{}",
                stats.dense_bricks,
                phases::table(&frames)
            );
        }
    }
}
