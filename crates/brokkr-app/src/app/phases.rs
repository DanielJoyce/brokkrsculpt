// SPDX-License-Identifier: AGPL-3.0-only

//! Where a frame's time goes, phase by phase.
//!
//! The readout behind the Info toggle used to hold two numbers about a
//! stroke: the last field edit and the last remesh. Everything else a
//! pointer event pays for (the ray marches under the pointer, the totals
//! taken after a remesh, the ring, the undo entry at the end of a stroke,
//! the upload) had no timer, and a change to any of it could not be said to
//! have helped. This is that timer.
//!
//! **A phase's time is its own and not its callers'.** The phases nest: an
//! edit asks for a pick, and ends in a remesh. Each `enter` charges the time
//! since the last mark to whatever phase was running and starts the new one,
//! so the columns of a frame add up to the time spent and nothing is counted
//! twice.
//!
//! **By the frame, not by the event.** Several pointer events can arrive
//! between two frames, and what the hand feels is the frame. Times are added
//! up as they happen and closed off by [`Phases::end_frame`], which the
//! application calls on its frame tick.
//!
//! Three readers: the Info readout (the last frame and the worst of the last
//! second), a log of one line a frame while a stroke is down
//! (`BROKKR_PHASE_LOG=<file>`), and the table a replayed stroke prints. See
//! `replay.rs`.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::io::Write;
use std::time::Duration;
use web_time::Instant;

/// What the time was spent on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Phase {
    /// Ray marches: the surface under the pointer, for a stroke, for the ring
    /// and for a choice of body.
    Pick,
    /// Writing the field: the stamps of one pointer event, or a studio
    /// gesture.
    Edit,
    /// Meshing the bricks an edit dirtied and handing them to the renderer.
    Remesh,
    /// What is worked out after a remesh: the document's totals, its radius.
    After,
    /// The ring and the other lines drawn over the model.
    Overlay,
    /// Ending a stroke: its undo entry, its picture.
    Finish,
    /// Building the interface, `view()`.
    View,
    /// The renderer's `prepare`: uploads of the frame's meshes.
    Upload,
    /// The renderer encoding its passes: the draw calls.
    Encode,
    /// Saving in the background's place: an autosave on this thread.
    Save,
}

impl Phase {
    pub(super) const ALL: [Phase; COUNT] = [
        Phase::Pick,
        Phase::Edit,
        Phase::Remesh,
        Phase::After,
        Phase::Overlay,
        Phase::Finish,
        Phase::View,
        Phase::Upload,
        Phase::Encode,
        Phase::Save,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Phase::Pick => "pick",
            Phase::Edit => "edit",
            Phase::Remesh => "remesh",
            Phase::After => "after",
            Phase::Overlay => "overlay",
            Phase::Finish => "finish",
            Phase::View => "view",
            Phase::Upload => "upload",
            Phase::Encode => "encode",
            Phase::Save => "save",
        }
    }
}

const COUNT: usize = 10;

/// How long the readout's "worst" looks back.
const RECENT: Duration = Duration::from_secs(1);

/// What one frame cost.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Frame {
    /// Milliseconds by phase, in [`Phase::ALL`]'s order.
    pub ms: [f32; COUNT],
    /// From the frame before to this one.
    pub interval_ms: f32,
    /// Pointer events handled.
    pub events: u32,
    /// Stamps laid.
    pub stamps: u32,
    /// Times a studio gesture that is set down whole (a shape, a body, a
    /// map, a hung cable, one stamp) was set down again.
    pub redone: u32,
    /// Bricks meshed.
    pub meshed: u32,
    /// Bricks the renderer took in, and their bytes.
    pub uploaded: u32,
    pub uploaded_bytes: u64,
    /// Whether a stroke was down at any point of the frame.
    pub stroke: bool,
}

impl Frame {
    pub(super) fn of(&self, phase: Phase) -> f32 {
        self.ms[phase as usize]
    }

    /// Everything this thread was timed doing.
    pub(super) fn timed_ms(&self) -> f32 {
        self.ms.iter().sum()
    }
}

/// A token for one `enter`, handed back to `leave`.
#[derive(Debug, Clone, Copy)]
#[must_use = "a phase that is entered has to be left"]
pub(super) struct Entered(usize);

/// The timers. Interior mutability throughout, because the picks and
/// `view()` are `&self`.
#[derive(Debug)]
pub(super) struct Phases {
    stack: RefCell<Vec<Phase>>,
    mark: Cell<Instant>,
    current: Cell<Frame>,
    last: Frame,
    last_frame_at: Option<Instant>,
    recent: VecDeque<(Instant, Frame)>,
    log: Option<std::io::BufWriter<std::fs::File>>,
    /// Whether the log holds lines its file does not.
    log_unwritten: bool,
    /// Every frame since [`Phases::begin_run`], for a table.
    run: Option<Vec<Frame>>,
}

impl Default for Phases {
    fn default() -> Self {
        Self {
            stack: RefCell::new(Vec::with_capacity(8)),
            mark: Cell::new(Instant::now()),
            current: Cell::new(Frame::default()),
            last: Frame::default(),
            last_frame_at: None,
            recent: VecDeque::new(),
            log: None,
            log_unwritten: false,
            run: None,
        }
    }
}

impl Phases {
    /// Start a phase. Time since the last mark goes to the phase it
    /// interrupts.
    pub(super) fn enter(&self, phase: Phase) -> Entered {
        let now = Instant::now();
        let mut stack = self.stack.borrow_mut();
        if let Some(running) = stack.last() {
            self.charge(*running, now);
        }
        self.mark.set(now);
        stack.push(phase);
        Entered(stack.len() - 1)
    }

    /// End the phase `entered` began, and any begun inside it that were not
    /// ended (a function that returned early).
    pub(super) fn leave(&self, entered: Entered) {
        let now = Instant::now();
        let mut stack = self.stack.borrow_mut();
        if let Some(running) = stack.last() {
            self.charge(*running, now);
        }
        self.mark.set(now);
        stack.truncate(entered.0);
    }

    fn charge(&self, phase: Phase, now: Instant) {
        let mut frame = self.current.get();
        frame.ms[phase as usize] += now.duration_since(self.mark.get()).as_secs_f32() * 1000.0;
        self.current.set(frame);
    }

    /// Add time measured somewhere else: the renderer's own clock.
    pub(super) fn add(&self, phase: Phase, ms: f32) {
        let mut frame = self.current.get();
        frame.ms[phase as usize] += ms;
        self.current.set(frame);
    }

    /// Count into the frame being built.
    pub(super) fn count(&self, with: impl FnOnce(&mut Frame)) {
        let mut frame = self.current.get();
        with(&mut frame);
        self.current.set(frame);
    }

    /// Close the frame off. Returns it.
    pub(super) fn end_frame(&mut self) -> Frame {
        let now = Instant::now();
        // A phase left open across a frame is a fault of the caller's, and
        // must not eat every later frame's time.
        self.stack.borrow_mut().clear();
        let mut frame = self.current.replace(Frame::default());
        frame.interval_ms = self
            .last_frame_at
            .replace(now)
            .map_or(0.0, |before| now.duration_since(before).as_secs_f32() * 1000.0);
        self.last = frame;
        self.recent.push_back((now, frame));
        while self.recent.front().is_some_and(|(at, _)| now.duration_since(*at) > RECENT) {
            self.recent.pop_front();
        }
        if let Some(log) = &mut self.log {
            if frame.stroke {
                let _ = writeln!(log, "{}", line(&frame));
                self.log_unwritten = true;
            } else if self.log_unwritten {
                // The stroke is over: its lines go to the file now, because
                // the application may be closed, or killed, at any time.
                let _ = log.flush();
                self.log_unwritten = false;
            }
        }
        if let Some(run) = &mut self.run {
            run.push(frame);
        }
        frame
    }

    /// The frame before this one.
    pub(super) fn last(&self) -> Frame {
        self.last
    }

    /// The worst of each phase over the last second.
    pub(super) fn worst(&self) -> Frame {
        let mut worst = Frame::default();
        for (_, frame) in &self.recent {
            for (slot, ms) in worst.ms.iter_mut().zip(frame.ms) {
                *slot = slot.max(ms);
            }
            worst.interval_ms = worst.interval_ms.max(frame.interval_ms);
            worst.uploaded = worst.uploaded.max(frame.uploaded);
            worst.uploaded_bytes = worst.uploaded_bytes.max(frame.uploaded_bytes);
        }
        worst
    }

    /// Write a line a frame to `path` while a stroke is down.
    pub(super) fn log_to(&mut self, path: &std::path::Path) -> std::io::Result<()> {
        let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
        writeln!(file, "{}", heading())?;
        self.log = Some(file);
        Ok(())
    }

    /// Push what the log holds to its file.
    pub(super) fn flush(&mut self) {
        if let Some(log) = &mut self.log {
            let _ = log.flush();
        }
    }

    /// Keep every frame from here, for a table.
    pub(super) fn begin_run(&mut self) {
        self.run = Some(Vec::new());
    }

    /// The frames kept since [`Phases::begin_run`].
    pub(super) fn end_run(&mut self) -> Vec<Frame> {
        self.run.take().unwrap_or_default()
    }
}

fn heading() -> String {
    let mut text = String::from("interval_ms");
    for phase in Phase::ALL {
        text.push(',');
        text.push_str(phase.label());
    }
    text.push_str(",events,stamps,redone,meshed,uploaded,uploaded_bytes");
    text
}

fn line(frame: &Frame) -> String {
    let mut text = format!("{:.3}", frame.interval_ms);
    for ms in frame.ms {
        text.push_str(&format!(",{ms:.3}"));
    }
    text.push_str(&format!(
        ",{},{},{},{},{},{}",
        frame.events,
        frame.stamps,
        frame.redone,
        frame.meshed,
        frame.uploaded,
        frame.uploaded_bytes
    ));
    text
}

/// A table of a run's frames: by phase, the total, and the middle, the
/// ninety-fifth in a hundred and the worst frame.
///
/// Only frames with a stroke down are counted, when there are any: the frames
/// either side of a stroke are idle and would halve every figure.
pub(super) fn table(frames: &[Frame]) -> String {
    let down: Vec<&Frame> = frames.iter().filter(|frame| frame.stroke).collect();
    let (counted, which): (Vec<&Frame>, &str) = if down.is_empty() {
        (frames.iter().collect(), "all frames")
    } else {
        (down, "frames with a stroke down")
    };
    let mut text = format!("{} {which} of {}\n", counted.len(), frames.len());
    if counted.is_empty() {
        return text;
    }
    text.push_str(&format!(
        "{:<10}{:>11}{:>10}{:>10}{:>10}\n",
        "phase", "total ms", "median", "95th", "worst"
    ));
    let mut row = |label: &str, values: Vec<f32>, total: bool| {
        let (sum, median, high, worst) = spread(values);
        let sum = if total { format!("{sum:.1}") } else { String::from("-") };
        text.push_str(&format!("{label:<10}{sum:>11}{median:>10.3}{high:>10.3}{worst:>10.3}\n"));
    };
    for phase in Phase::ALL {
        row(phase.label(), counted.iter().map(|frame| frame.of(phase)).collect(), true);
    }
    row("timed", counted.iter().map(|frame| frame.timed_ms()).collect(), true);
    row("interval", counted.iter().map(|frame| frame.interval_ms).collect(), false);
    let sum = |of: fn(&Frame) -> u64| counted.iter().map(|frame| of(frame)).sum::<u64>();
    text.push_str(&format!(
        "{} pointer events, {} stamps, {} gestures set down again, {} bricks meshed, {} \
         uploaded ({:.1} MB)\n",
        sum(|frame| u64::from(frame.events)),
        sum(|frame| u64::from(frame.stamps)),
        sum(|frame| u64::from(frame.redone)),
        sum(|frame| u64::from(frame.meshed)),
        sum(|frame| u64::from(frame.uploaded)),
        sum(|frame| frame.uploaded_bytes) as f64 / (1024.0 * 1024.0),
    ));
    text
}

/// The sum, the median, the ninety-fifth in a hundred and the largest.
fn spread(mut values: Vec<f32>) -> (f32, f32, f32, f32) {
    if values.is_empty() {
        return (0.0, 0.0, 0.0, 0.0);
    }
    values.sort_by(f32::total_cmp);
    let at = |part: f32| values[((values.len() - 1) as f32 * part).round() as usize];
    (values.iter().sum(), at(0.5), at(0.95), values[values.len() - 1])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A phase begun inside another takes its own time out of the outer
    /// one's: the two add up to the whole and neither holds the other's.
    #[test]
    fn a_phase_inside_another_is_not_counted_twice() {
        let mut phases = Phases::default();
        let began = Instant::now();
        let outer = phases.enter(Phase::Edit);
        std::thread::sleep(Duration::from_millis(4));
        let inner = phases.enter(Phase::Pick);
        std::thread::sleep(Duration::from_millis(8));
        phases.leave(inner);
        std::thread::sleep(Duration::from_millis(4));
        phases.leave(outer);
        let whole = began.elapsed().as_secs_f32() * 1000.0;
        let frame = phases.end_frame();
        assert!(frame.of(Phase::Pick) >= 8.0, "the inner phase lost time: {frame:?}");
        assert!(frame.of(Phase::Edit) >= 8.0, "the outer phase lost time: {frame:?}");
        assert!(
            frame.timed_ms() <= whole + 0.01,
            "{} ms were counted in {whole} ms",
            frame.timed_ms()
        );
    }

    /// A function that returned between an `enter` and its `leave` must not
    /// leave its phase running: leaving the outer one ends it too.
    #[test]
    fn leaving_an_outer_phase_ends_what_was_begun_inside_it() {
        let mut phases = Phases::default();
        let outer = phases.enter(Phase::Edit);
        let _forgotten = phases.enter(Phase::Pick);
        phases.leave(outer);
        assert!(phases.stack.borrow().is_empty());
        std::thread::sleep(Duration::from_millis(5));
        let frame = phases.end_frame();
        assert!(frame.timed_ms() < 4.0, "time ran on after the phase was left: {frame:?}");
    }

    #[test]
    fn the_worst_of_the_last_second_is_by_phase() {
        let mut phases = Phases::default();
        phases.add(Phase::Remesh, 3.0);
        phases.add(Phase::Edit, 1.0);
        phases.end_frame();
        phases.add(Phase::Remesh, 1.0);
        phases.add(Phase::Edit, 2.0);
        phases.end_frame();
        let worst = phases.worst();
        assert_eq!(worst.of(Phase::Remesh), 3.0);
        assert_eq!(worst.of(Phase::Edit), 2.0);
        assert_eq!(phases.last().of(Phase::Remesh), 1.0);
    }

    /// The table counts the frames a stroke was down in and leaves the idle
    /// ones either side out of every figure.
    #[test]
    fn the_table_is_of_the_frames_with_a_stroke_down() {
        let mut phases = Phases::default();
        phases.begin_run();
        phases.end_frame();
        for ms in [1.0, 2.0, 9.0] {
            phases.add(Phase::Edit, ms);
            phases.count(|frame| {
                frame.stroke = true;
                frame.stamps += 2;
            });
            phases.end_frame();
        }
        phases.end_frame();
        let frames = phases.end_run();
        assert_eq!(frames.len(), 5);
        let text = table(&frames);
        assert!(text.starts_with("3 frames with a stroke down of 5"), "{text}");
        let edit = text.lines().find(|line| line.starts_with("edit")).expect("an edit row");
        let numbers: Vec<f32> =
            edit.split_whitespace().skip(1).map(|number| number.parse().unwrap()).collect();
        assert_eq!(numbers, [12.0, 2.0, 9.0, 9.0], "{text}");
        assert!(text.contains("6 stamps"), "{text}");
    }

    #[test]
    fn a_log_line_has_a_number_for_every_heading() {
        let frame = Frame { stroke: true, events: 3, ..Frame::default() };
        assert_eq!(heading().split(',').count(), line(&frame).split(',').count());
    }
}
