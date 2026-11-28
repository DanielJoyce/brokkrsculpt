// SPDX-License-Identifier: AGPL-3.0-only

//! The compositor's tablets, through `octotablet`. Linux, and Wayland of it.
//!
//! `octotablet` joins the connection the window already has to the
//! compositor and asks for the tablet protocol on a queue of its own. It
//! reads nothing itself: winit reads the socket, as it did, and what is the
//! tablet's waits on that queue until [`Seat::pump`] is called, which the
//! interface does each frame.

use std::mem::ManuallyDrop;

use glam::Vec2;
use octotablet::events::{Event, ToolEvent};

use super::Raw;
use crate::tablet::{Tablet, TabletDevice};

pub struct Seat {
    /// Never dropped. It holds the display the window's connection is made
    /// on by a bare pointer, and winit closes that when the event loop ends,
    /// ahead of this thread's own storage being taken down: dropping the
    /// manager then would be a use of a display that is gone. The process is
    /// ending when that matters, and the leak is one small allocation.
    manager: ManuallyDrop<octotablet::Manager>,
    /// Where the list of tablets goes, for the panel.
    tablet: Tablet,
    /// The connection failed once. Not asked again, nor said again.
    broken: bool,
}

impl Seat {
    pub fn open(window: &dyn iced::window::Window, tablet: Tablet) -> Result<Self, String> {
        // First, and without octotablet: see `offered`.
        ask_what_is_offered(window)?;
        // Safety: `build_raw` asks that the window's display stay valid for
        // as long as the manager is alive. The display is winit's, and lives
        // until the event loop ends. The manager is used only from the
        // interface's thread while the loop runs (`pump`, called from a
        // widget's update), and is never dropped: see `manager`.
        let manager = unsafe { octotablet::Builder::new().build_raw(window) }
            .map_err(|why| why.to_string())?;
        tablet.seat_attached();
        Ok(Self { manager: ManuallyDrop::new(manager), tablet, broken: false })
    }

    /// What the compositor has said since the last call, onto `raw`.
    pub fn pump(&mut self, raw: &mut Vec<Raw>) {
        if self.broken {
            return;
        }
        let events = match self.manager.pump() {
            Ok(events) => events,
            Err(why) => {
                log::warn!("pen: the compositor's tablet connection failed: {why}");
                self.broken = true;
                return;
            }
        };
        let mut listing_changed = false;
        for event in events {
            match event {
                Event::Tool { tool, event } => match event {
                    ToolEvent::Added => listing_changed = true,
                    ToolEvent::In { .. } => raw.push(Raw::In {
                        eraser: tool.tool_type == Some(octotablet::tool::Type::Eraser),
                    }),
                    ToolEvent::Down => raw.push(Raw::Down),
                    ToolEvent::Up => raw.push(Raw::Up),
                    ToolEvent::Pose(pose) => raw.push(Raw::Pose {
                        at: Vec2::from(pose.position),
                        pressure: pose.pressure.get(),
                        tilt: pose.tilt.map(Vec2::from),
                    }),
                    ToolEvent::Button { button_id, pressed } => {
                        if let Some(code) = code_of(button_id) {
                            raw.push(Raw::Button { code, pressed });
                        }
                    }
                    ToolEvent::Frame(_) => raw.push(Raw::Frame),
                    ToolEvent::Out => raw.push(Raw::Out),
                    // A pen unplugged while it is in range says no more of
                    // itself. It has left.
                    ToolEvent::Removed => {
                        listing_changed = true;
                        raw.push(Raw::Out);
                        raw.push(Raw::Frame);
                    }
                },
                Event::Tablet { .. } => listing_changed = true,
                // The buttons, rings and strips of the tablet itself are the
                // compositor's to give meaning to, in its settings.
                Event::Pad { .. } => {}
            }
        }
        if listing_changed {
            let listing = self.listing();
            let names: Vec<&str> = listing.iter().map(|device| device.name.as_str()).collect();
            log::info!(
                "pen: the compositor lists {} tablet(s) [{}] and {} tool(s)",
                listing.len(),
                names.join(", "),
                self.manager.tools().len(),
            );
            self.tablet.seat_lists(listing);
        }
    }

    /// The tablets as the panel shows them.
    ///
    /// A pen is made known when it first comes near, on most compositors, so
    /// a tablet lists with no tilt and no eraser until then.
    fn listing(&self) -> Vec<TabletDevice> {
        let tools = self.manager.tools();
        let has_tilt = tools.iter().any(|tool| tool.axes.tilt.is_some());
        let has_eraser =
            tools.iter().any(|tool| tool.tool_type == Some(octotablet::tool::Type::Eraser));
        self.manager
            .tablets()
            .iter()
            .map(|tablet| TabletDevice {
                name: tablet.name.clone().unwrap_or_else(|| "tablet".to_string()),
                path: String::new(),
                // The protocol gives every pen the same scale, whatever the
                // tablet counts in: its own range is not known here.
                pressure_max: 0,
                has_tilt,
                has_eraser,
            })
            .collect()
    }
}

/// Ask the compositor what it offers, and refuse what would end the process.
///
/// **octotablet binds the tablet protocol, and the seat, at whatever version
/// the compositor offers**, with no regard for the version its own
/// definitions were written to. A compositor a version ahead then sends an
/// event those definitions have no name for, and libwayland does not report
/// that: it ends the process. That is how octotablet 0.1.0 took the
/// application down at startup on KWin, on 2026-11-27 ("interface
/// 'zwp_tablet_v2' has no event 5"). The definitions are current now; the
/// protocol will have another version one day, and the day a compositor
/// ships it every copy of this would stop starting.
///
/// So the offer is read here first, on a queue of this function's own, and
/// octotablet is let near the connection only when everything it will bind
/// is at a version it knows. Refused, the pen is as it was before any of
/// this: a mouse, made by the compositor for a window that did not ask, with
/// the kernel reader for its pressure.
///
/// It also says whether there is a tablet protocol at all, which octotablet
/// does not.
fn ask_what_is_offered(window: &dyn iced::window::Window) -> Result<(), String> {
    use iced::window::raw_window_handle::RawDisplayHandle;
    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::{wl_registry, wl_seat};
    use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, backend::Backend};
    use wayland_protocols::wp::tablet::zv2::client::zwp_tablet_manager_v2::ZwpTabletManagerV2;

    /// Nothing is done with the registry but read its list.
    struct Asking;

    impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Asking {
        fn event(
            _: &mut Self,
            _: &wl_registry::WlRegistry,
            _: wl_registry::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    let display = window.display_handle().map_err(|why| why.to_string())?;
    let RawDisplayHandle::Wayland(display) = display.as_raw() else {
        return Err("the window is not on Wayland".to_string());
    };
    // Safety: the pointer is the live display the window is drawn on, and
    // is used only inside this call, on the thread the window belongs to.
    // A backend made of a display that is someone else's does not close it.
    let backend = unsafe { Backend::from_foreign_display(display.display.as_ptr().cast()) };
    let connection = Connection::from_backend(backend);
    // One round trip, which reads what is the window's too and leaves it on
    // the window's own queue for winit to find, as octotablet's reading
    // leaves octotablet's.
    let (globals, _queue) =
        registry_queue_init::<Asking>(&connection).map_err(|why| why.to_string())?;
    globals.contents().with_list(|list| {
        offered(
            list.iter().map(|global| (global.interface.as_str(), global.version)),
            ZwpTabletManagerV2::interface().version,
            wl_seat::WlSeat::interface().version,
        )
    })
}

/// Whether what a compositor offers is safe to hand to octotablet, given
/// the versions this build knows of the two things octotablet binds.
fn offered<'a>(
    globals: impl Iterator<Item = (&'a str, u32)>,
    knows_tablet: u32,
    knows_seat: u32,
) -> Result<(), String> {
    let mut has_tablets = false;
    for (interface, version) in globals {
        let knows = match interface {
            "zwp_tablet_manager_v2" => {
                has_tablets = true;
                knows_tablet
            }
            "wl_seat" => knows_seat,
            _ => continue,
        };
        if version > knows {
            return Err(format!(
                "the compositor offers {interface} at version {version} and this build knows \
                 it to version {knows}"
            ));
        }
    }
    if has_tablets { Ok(()) } else { Err("the compositor has no tablet protocol".to_string()) }
}

/// The kernel's code for a button of the pen.
///
/// `octotablet` keeps it, under Wayland, and keeps it to itself: a
/// `ButtonID` can be compared and hashed and nothing else. Its hash is the
/// code written to the hasher, so a hasher that only remembers what it was
/// given reads it back. Should a later version hash something else, the
/// buttons stop meaning anything and nothing worse: [`super::button_of`]
/// knows a handful of codes and passes the rest by.
fn code_of(button: octotablet::tool::ButtonID) -> Option<u32> {
    use std::hash::{Hash, Hasher};

    #[derive(Default)]
    struct Remember(Option<u32>);

    impl Hasher for Remember {
        fn finish(&self) -> u64 {
            0
        }
        fn write(&mut self, _bytes: &[u8]) {}
        fn write_u32(&mut self, value: u32) {
            self.0 = Some(value);
        }
    }

    let mut remember = Remember::default();
    button.hash(&mut remember);
    remember.0
}

#[cfg(test)]
mod tests {
    use super::offered;

    const KWIN: [(&str, u32); 4] =
        [("wl_compositor", 6), ("wl_seat", 9), ("zwp_tablet_manager_v2", 2), ("xdg_wm_base", 7)];

    #[test]
    fn a_compositor_at_versions_this_knows_is_asked() {
        assert_eq!(offered(KWIN.into_iter(), 2, 11), Ok(()));
    }

    /// The fault of 2026-11-27, as it would come again: the protocol a
    /// version past what the definitions here were written to.
    #[test]
    fn a_tablet_protocol_ahead_of_this_build_is_left_alone() {
        let refused = offered(KWIN.into_iter(), 1, 11).expect_err("version 2 offered, 1 known");
        assert!(refused.contains("zwp_tablet_manager_v2"), "{refused}");
        assert!(refused.contains("version 2"), "{refused}");
    }

    #[test]
    fn a_seat_ahead_of_this_build_is_left_alone_too() {
        let refused = offered(KWIN.into_iter(), 2, 8).expect_err("seat 9 offered, 8 known");
        assert!(refused.contains("wl_seat"), "{refused}");
    }

    #[test]
    fn a_compositor_with_no_tablet_protocol_says_so() {
        let globals = [("wl_compositor", 6), ("wl_seat", 9)];
        assert_eq!(
            offered(globals.into_iter(), 2, 11),
            Err("the compositor has no tablet protocol".to_string())
        );
    }
}
