// SPDX-License-Identifier: AGPL-3.0-only

//! The crash net, written without stopping the drawing.
//!
//! The autosave used to be one call on the thread that draws, and its own
//! comment had the figure: 1.55 GB/s, so 100 ms for a 159 MB model and about
//! 430 ms for the 671 MB one seen from a real scan. Twenty six frames, every
//! two minutes. Waiting for the pointer to rest hid it and did not shorten
//! it, and the comment named the next step.
//!
//! This is that step, taken a little differently from how it was named. A
//! document cannot be handed to another thread, because its bricks are owned
//! and not shared, and encoding all of it to a buffer first would still be
//! the whole copy in one frame. So the encoding is cut up
//! ([`brokkr_core::project::Pieces`]): between two frames this thread makes
//! one piece of a few megabytes, which is a copy and takes about a
//! millisecond, and hands it to a thread that does the writing. The file is
//! the same bytes.
//!
//! **The document must not change while a write is in pieces**, and the
//! application sees to it in two places. Any message but the frame tick
//! abandons a write in flight ([`Brokkr::update`]), and so does a remesh
//! that finds anything dirty. An abandoned write removes its temporary file
//! and leaves the autosave before it alone, and the next one starts when the
//! application is next idle. So a write only ever finishes across a run of
//! frames in which nothing was asked of the document.
//!
//! Through a temporary and a rename, as before, so that a crash during the
//! write cannot destroy the autosave before it.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use brokkr_core::ProjectState;
use brokkr_core::project::Pieces;

use super::Brokkr;

/// About how many bytes are encoded between two frames.
///
/// A piece of this size took 0.87 ms to make, and 2.45 ms at the worst, on
/// the machine `docs/PERFORMANCE.md` describes (`what_an_autosave_costs`, 83
/// pieces of a 327 MB document). At sixty frames a second it is 240 MB a
/// second by arithmetic, so the 671 MB document seen from a real scan is
/// encoded in about three seconds of the pointer resting.
const PIECE_BYTES: usize = 4 << 20;

/// Pieces that may wait for the disk. Past this the encoder makes no more
/// until one is written, so a slow disk costs time and not memory: at most
/// this many pieces, 32 MB, are ever held.
const WAITING_PIECES: usize = 8;

/// A write in flight.
#[derive(Debug)]
struct Job {
    pieces: Pieces,
    state: ProjectState,
    /// Dropped to tell the writer there is no more.
    pieces_out: SyncSender<Vec<u8>>,
    /// Buffers the writer has finished with, to be filled again: a fresh
    /// four megabytes a frame is a thousand page faults a frame.
    spare: Receiver<Vec<u8>>,
    /// A piece made and not yet taken: the disk is behind.
    held: Option<Vec<u8>>,
    /// Set before `pieces_out` is dropped when every piece went out. Unset,
    /// the writer throws its file away.
    whole: Arc<AtomicBool>,
    began: Instant,
    bytes: u64,
    frames: u32,
}

/// The autosave's state between frames.
#[derive(Debug, Default)]
pub(super) struct Autosave {
    job: Option<Job>,
    /// The thread of the write that last began, to wait for where it matters
    /// that it is over.
    writer: Mutex<Option<JoinHandle<()>>>,
}

impl Autosave {
    pub(super) fn in_flight(&self) -> bool {
        self.job.is_some()
    }

    /// How many bytes the write in flight has encoded.
    #[cfg(test)]
    fn encoded(&self) -> u64 {
        self.job.as_ref().map_or(0, |job| job.bytes)
    }

    /// Give a write in flight up. Its temporary file is removed by its
    /// writer, and the autosave before it stays.
    pub(super) fn abandon(&mut self) {
        if let Some(job) = self.job.take() {
            log::debug!(
                "an autosave was given up after {} MB: the document was wanted",
                job.bytes / (1024 * 1024)
            );
        }
    }

    /// Wait until no thread is writing. For the moments a file is about to
    /// be read or removed, so that a write just ending does not land after.
    pub(super) fn settle(&self) {
        let writer = self.writer.lock().expect("the autosave's writer").take();
        if let Some(writer) = writer {
            let _ = writer.join();
        }
    }
}

/// What the writing thread does: every piece into the temporary file, then
/// the rename if the pieces were the whole document, and the file removed if
/// they were not.
fn write_pieces(
    pieces: Receiver<Vec<u8>>,
    spare: std::sync::mpsc::Sender<Vec<u8>>,
    temporary: PathBuf,
    path: PathBuf,
    whole: Arc<AtomicBool>,
) {
    let written = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&temporary)?;
        for piece in pieces {
            file.write_all(&piece)?;
            // Back to be filled again. Nobody may be there to take it.
            let _ = spare.send(piece);
        }
        file.flush()
    })();
    match written {
        Ok(()) if whole.load(Ordering::Acquire) => match std::fs::rename(&temporary, &path) {
            Ok(()) => log::info!("autosaved to {}", path.display()),
            Err(error) => log::warn!("could not replace {}: {error}", path.display()),
        },
        // Given up part way: not a failure, and nothing to say.
        Ok(()) => {
            std::fs::remove_file(&temporary).ok();
        }
        Err(error) => {
            log::warn!("could not autosave to {}: {error}", temporary.display());
            std::fs::remove_file(&temporary).ok();
        }
    }
}

impl Brokkr {
    /// Begin writing the crash net. Reports only to the log.
    ///
    /// Deliberately does NOT clear `unsaved` and does NOT touch
    /// `project_path`: this is not a save, it is something to recover from,
    /// and treating it as a save would suppress the very prompt that protects
    /// the real file. It also never writes over the user's own file.
    ///
    /// Failures go to the log and nowhere else. `status` is the line the user
    /// is reading for the action they asked for, and an autosave is not one.
    ///
    /// Returns whether a write is now in flight.
    pub(super) fn begin_autosave(&mut self) -> bool {
        let Some(path) = self.autosave_file.clone() else {
            return false;
        };
        self.autosave.abandon();
        // One writer at a time: two would both be writing the one temporary.
        self.autosave.settle();
        if let Some(parent) = path.parent()
            && let Err(error) = std::fs::create_dir_all(parent)
        {
            log::warn!("could not create {}: {error}", parent.display());
            return false;
        }
        let pieces = match Pieces::begin(&self.doc) {
            Ok(pieces) => pieces,
            Err(error) => {
                log::warn!("could not autosave to {}: {error}", path.display());
                return false;
            }
        };
        let temporary = path.with_extension("brokkr.tmp");
        let (pieces_out, pieces_in) = sync_channel(WAITING_PIECES);
        let (spare_out, spare) = std::sync::mpsc::channel();
        let whole = Arc::new(AtomicBool::new(false));
        let for_writer = Arc::clone(&whole);
        let spawned = std::thread::Builder::new()
            .name("autosave".to_string())
            .spawn(move || write_pieces(pieces_in, spare_out, temporary, path, for_writer));
        match spawned {
            Ok(writer) => {
                *self.autosave.writer.lock().expect("the autosave's writer") = Some(writer);
            }
            Err(error) => {
                log::warn!("could not start the autosave's thread: {error}");
                return false;
            }
        }
        self.autosave.job = Some(Job {
            pieces,
            state: self.project_state(),
            pieces_out,
            spare,
            held: None,
            whole,
            began: Instant::now(),
            bytes: 0,
            frames: 0,
        });
        true
    }

    /// Give a write in flight up, because the document is wanted. The pause
    /// the next one waits for is counted from now.
    pub(super) fn give_up_autosave(&mut self) {
        if self.autosave.in_flight() {
            self.autosave.abandon();
            self.last_activity = Instant::now();
        }
    }

    /// Make the next piece of a write in flight and hand it over.
    pub(super) fn advance_autosave(&mut self) {
        let Some(job) = &mut self.autosave.job else {
            return;
        };
        job.frames += 1;
        // The piece the disk was not ready for goes first.
        let piece = match job.held.take() {
            Some(piece) => piece,
            None if job.pieces.done() => Vec::new(),
            None => {
                let mut piece = job
                    .spare
                    .try_recv()
                    .unwrap_or_else(|_| Vec::with_capacity(PIECE_BYTES + (1 << 18)));
                piece.clear();
                if let Err(error) = job.pieces.next(&mut piece, &self.doc, &job.state, PIECE_BYTES)
                {
                    log::warn!("the autosave was given up: {error}");
                    self.autosave.job = None;
                    return;
                }
                job.bytes += piece.len() as u64;
                piece
            }
        };
        if !piece.is_empty() {
            match job.pieces_out.try_send(piece) {
                Ok(()) => {}
                Err(TrySendError::Full(piece)) => {
                    job.held = Some(piece);
                    return;
                }
                // The writer has gone, which it does only on an error it has
                // already put in the log.
                Err(TrySendError::Disconnected(_)) => {
                    self.autosave.job = None;
                    return;
                }
            }
        }
        if !job.pieces.done() {
            return;
        }
        // Every piece is with the writer: it may keep the file.
        job.whole.store(true, Ordering::Release);
        let taken = job.began.elapsed().as_secs_f64();
        log::info!(
            "an autosave of {:.0} MB was encoded over {} frames in {taken:.2} s",
            job.bytes as f64 / (1024.0 * 1024.0),
            job.frames
        );
        self.autosave.job = None;
        self.last_autosave = Instant::now();
    }

    /// Write the crash net and do not return until it is on the disk. What a
    /// test wants, and nothing in the running application does.
    #[cfg(test)]
    pub(super) fn write_autosave(&mut self) {
        let _ = self.begin_autosave();
        while self.autosave.in_flight() {
            self.advance_autosave();
        }
        self.autosave.settle();
    }

    /// Throw the crash net away, once its work is safely somewhere else.
    pub(super) fn clear_autosave(&mut self) {
        // A write in flight is of a document that has just been saved, and
        // one just ending must not land after the removal below.
        self.autosave.abandon();
        self.autosave.settle();
        if let Some(path) = self.autosave_file.as_ref() {
            std::fs::remove_file(path).ok();
        }
    }

    /// Where the autosave's temporary file is, for a test to look for.
    #[cfg(test)]
    pub(super) fn autosave_temporary(&self) -> Option<PathBuf> {
        self.autosave_file.as_deref().map(|path| path.with_extension("brokkr.tmp"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Message, PointerButton, PointerEvent};
    use crate::tablet::Tablet;
    use iced::Vector;
    use std::time::Duration;

    const SIZE: Vector = Vector { x: 800.0, y: 600.0 };

    /// An application with something unsaved in it and somewhere of its own
    /// to autosave to.
    fn app(name: &str) -> (Brokkr, PathBuf) {
        let directory =
            std::env::temp_dir().join(format!("brokkr-autosave-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let mut app = Brokkr::with_tablet(Tablet::inert());
        app.autosave_file = Some(directory.join("autosave.brokkr"));
        carve(&mut app);
        (app, directory)
    }

    fn carve(app: &mut Brokkr) {
        let at = Vector::new(SIZE.x / 2.0, SIZE.y / 2.0);
        app.on_pointer(PointerEvent::Pressed {
            button: PointerButton::Left,
            position: at,
            size: SIZE,
        });
        app.on_pointer(PointerEvent::Released { button: PointerButton::Left });
        assert!(app.unsaved, "the press carved nothing");
    }

    /// Long enough ago that the interval and the pause have both passed.
    fn overdue(app: &mut Brokkr) {
        let long_ago = Instant::now() - Duration::from_secs(600);
        app.last_autosave = long_ago;
        app.last_activity = long_ago;
    }

    fn frame(app: &mut Brokkr) {
        drop(app.update(Message::Frame));
    }

    fn bytes_of(app: &Brokkr) -> Vec<u8> {
        let mut bytes = Vec::new();
        brokkr_core::project::write(&mut bytes, &app.doc, &app.project_state()).unwrap();
        bytes
    }

    /// The ball is tens of megabytes, so its crash net is several pieces: it
    /// is not there after the frame that begins it, it is there some frames
    /// later, and it is the bytes a save would have written.
    #[test]
    fn the_crash_net_is_written_over_several_frames() {
        let (mut app, directory) = app("frames");
        overdue(&mut app);
        frame(&mut app);
        assert!(app.autosave.in_flight(), "an overdue autosave did not begin");
        assert!(!app.has_autosave(), "the file was there after one frame's piece");

        let mut frames = 1;
        while app.autosave.in_flight() {
            frame(&mut app);
            frames += 1;
            assert!(frames < 10_000, "the autosave never ended");
        }
        app.autosave.settle();
        assert!(frames >= 4, "a document of tens of megabytes went out in {frames} frames");
        let path = app.autosave_file.clone().unwrap();
        assert!(
            std::fs::read(&path).unwrap() == bytes_of(&app),
            "the crash net is not the document"
        );
        assert!(!app.autosave_temporary().unwrap().exists(), "the temporary file was left");
        assert!(app.unsaved, "an autosave is not a save");
        // And it is not begun again on the next frame.
        frame(&mut app);
        assert!(!app.autosave.in_flight(), "it began again at once");
        std::fs::remove_dir_all(&directory).ok();
    }

    /// A stroke begun while the crash net is part written: the write is
    /// given up, the crash net from before is still whole, nothing is left
    /// beside it, and the next pause writes the document as it then is.
    #[test]
    fn a_write_is_given_up_when_the_document_is_wanted() {
        let (mut app, directory) = app("given-up");
        app.write_autosave();
        let path = app.autosave_file.clone().unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(before == bytes_of(&app));

        overdue(&mut app);
        frame(&mut app);
        frame(&mut app);
        assert!(app.autosave.in_flight());
        let at = Vector::new(SIZE.x / 2.0 + 30.0, SIZE.y / 2.0);
        drop(app.update(Message::Pointer(PointerEvent::Pressed {
            button: PointerButton::Left,
            position: at,
            size: SIZE,
        })));
        assert!(!app.autosave.in_flight(), "a press did not give the write up");
        drop(app.update(Message::Pointer(PointerEvent::Released { button: PointerButton::Left })));
        app.autosave.settle();
        assert!(std::fs::read(&path).unwrap() == before, "the crash net from before was touched");
        assert!(!app.autosave_temporary().unwrap().exists(), "the part written was left behind");
        assert!(before != bytes_of(&app), "the press carved nothing, so the test shows nothing");

        // Not at once: the pause is counted from the press.
        frame(&mut app);
        assert!(!app.autosave.in_flight(), "it began again with the hand still on the model");
        app.last_activity = Instant::now() - Duration::from_secs(60);
        frame(&mut app);
        assert!(app.autosave.in_flight(), "the next pause did not begin it again");
        while app.autosave.in_flight() {
            frame(&mut app);
        }
        app.autosave.settle();
        assert!(std::fs::read(&path).unwrap() == bytes_of(&app));
        std::fs::remove_dir_all(&directory).ok();
    }

    /// Bricks changed by something that is not a message (the frame tick
    /// settles a mask, a task lands): the remesh that follows gives the
    /// write up.
    #[test]
    fn a_remesh_gives_a_write_up() {
        let (mut app, directory) = app("remesh");
        overdue(&mut app);
        frame(&mut app);
        assert!(app.autosave.in_flight());
        app.doc.mark_everything_dirty();
        app.remesh_dirty();
        assert!(!app.autosave.in_flight(), "the document was remeshed under a write");
        app.autosave.settle();
        assert!(!app.has_autosave());
        assert!(!app.autosave_temporary().unwrap().exists());
        std::fs::remove_dir_all(&directory).ok();
    }

    /// A save clears the crash net, and a write in flight does not put one
    /// back after it.
    #[test]
    fn clearing_the_crash_net_ends_a_write_in_flight() {
        let (mut app, directory) = app("cleared");
        app.write_autosave();
        assert!(app.has_autosave());
        overdue(&mut app);
        frame(&mut app);
        assert!(app.autosave.in_flight());
        app.clear_autosave();
        assert!(!app.autosave.in_flight());
        assert!(!app.has_autosave(), "the crash net is still there after it was cleared");
        assert!(!app.autosave_temporary().unwrap().exists());
        std::fs::remove_dir_all(&directory).ok();
    }

    /// What the two ways of writing cost. Printed, not asserted.
    ///
    /// ```text
    /// cargo test --release -p brokkr-app -- --ignored --nocapture what_an_autosave_costs
    /// ```
    #[test]
    #[ignore = "a measurement, not a test"]
    fn what_an_autosave_costs() {
        let (mut app, directory) = app("cost");
        drop(app.update(Message::Resample(super::super::VOXEL_SIZE_MM / 3.0)));
        let megabytes = bytes_of(&app).len() as f64 / (1024.0 * 1024.0);

        // The whole write in one call, as the thread that draws used to make
        // it, to a file.
        let path = directory.join("whole.brokkr");
        let began = Instant::now();
        let mut writer = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
        brokkr_core::project::write(&mut writer, &app.doc, &app.project_state()).unwrap();
        drop(writer);
        let whole = began.elapsed().as_secs_f64() * 1000.0;

        // In pieces: what each frame's piece cost this thread. A call that
        // made no piece, because the disk was behind, is not a frame's work
        // and is waited out.
        let mut worst = 0.0f64;
        let mut sum = 0.0f64;
        let mut pieces = 0;
        let began = Instant::now();
        assert!(app.begin_autosave());
        while app.autosave.in_flight() {
            let before = app.autosave.encoded();
            let piece = Instant::now();
            app.advance_autosave();
            let taken = piece.elapsed().as_secs_f64() * 1000.0;
            if !app.autosave.in_flight() || app.autosave.encoded() > before {
                worst = worst.max(taken);
                sum += taken;
                pieces += 1;
            } else {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        app.autosave.settle();
        let all = began.elapsed().as_secs_f64() * 1000.0;
        println!(
            "{megabytes:.0} MB: in one call {whole:.0} ms ({:.2} GB/s). In {pieces} pieces: \
             {:.2} ms each on the thread that draws, {worst:.2} ms the worst, {sum:.0} ms in \
             all, and {all:.0} ms until it was on the disk with no frames between",
            megabytes / 1024.0 / (whole / 1000.0),
            sum / pieces as f64
        );
        std::fs::remove_dir_all(&directory).ok();
    }
}
