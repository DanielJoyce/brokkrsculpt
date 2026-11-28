#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Drive the running HaniSculpt window with a virtual input device.

There is no ydotool, wtype or dotool on this machine and XTEST silently does
nothing under KWin Wayland, which is why `scripts/shot.sh` says there is no
input injection here. That was true of the routes tried; it is not true of
`/dev/uinput`, which is writable by the `input` group and creates a device the
kernel and the compositor cannot tell from real hardware.

# Two things that are not obvious and cost a session each

**Absolute, never relative.** libinput accelerates relative motion, so a delta
of 500 does not travel 500 pixels and nothing ever lands where it was aimed.
The ABS range is set to the logical desktop union, read from `kscreen-doctor`,
so a coordinate here is a coordinate on screen.

**Coordinates are WINDOW-LOCAL.** The window moves between sessions and the
desktop is two outputs at an offset, so a desktop coordinate written down today
aims at nothing tomorrow. The window's position comes from KWin at run time and
local coordinates are converted against it. Local (0, 0) is the top-left of the
window, in the same logical pixels the application lays out in -- so a position
read off a `shot.sh` capture converts by dividing by the capture's scale
(1.5 on this desktop: a 1024x768 window captures at 1536x1152).

This device deliberately does NOT advertise `ABS_PRESSURE` or `BTN_TOOL_PEN`
(how the application recognises a tablet) nor all six of `REL_X`..`REL_RZ` (how
it recognises a SpaceMouse), so the application under test will not adopt it as
either and the pen and puck paths stay untouched.

**The pen is a second device, made only when a script asks for it.** The `pen`
commands below build a virtual tablet, which libinput takes for one and the
compositor hands to the window over the tablet protocol: the way a real pen
arrives, pressure and tilt with it, and the only way to drive `src/pen.rs`. A
tablet is not laid over the desktop as the pointer is. KWin lays it over ONE
output, the one the active window is on unless the tablet's settings say
another, so the pen's coordinates are worked out against that output. If the
pen lands somewhere else, that mapping is why: check System Settings, Drawing
Tablet, for what the tablet named `hani-pen` is mapped to.

Usage
-----
    scripts/drive.py <script-file>          # one command per line
    scripts/drive.py --geometry             # print the window rectangle
    scripts/drive.py --ask 'JS'             # ask KWin; see `kwin_ask`

Commands, all coordinates window-local:

    move X Y                    click [left|right|middle]
    down [button]               up [button]
    drag X1 Y1 X2 Y2 [steps]    mdrag X1 Y1 X2 Y2 [steps]   # middle: orbits
    key NAME                    keydown NAME / keyup NAME
    scroll N                    sleep MS

    pen X Y [PRESSURE]          # the pen in range there; touching if PRESSURE > 0
    pendrag X1 Y1 X2 Y2 [P1 P2 [steps]]   # a stroke, pressure P1 rising to P2
    pentilt TX TY               # lean, each -1 to 1, kept until changed
    penbutton lower|upper down|up
    eraser on|off               # which end the next `pen` brings into range
    penout                      # out of range

PRESSURE is 0 to 1.

`#` starts a comment. Key names are in KEYS below; `ctrl+z` is
`keydown ctrl`, `key z`, `keyup ctrl`.
"""
import fcntl
import os
import re
import shutil
import struct
import subprocess
import sys
import time

UINPUT = "/dev/uinput"
# Qt's D-Bus tool, by whichever name the distribution gives it: `qdbus6` on
# most, `qdbus-qt6` on Fedora.
QDBUS = shutil.which("qdbus6") or shutil.which("qdbus-qt6") or "qdbus6"
# Which window to drive. Overridable the way `shot.sh` already takes
# `HANI_WINDOW_CLASS`, because the application is not the only window a test
# has to reach: an export puts up a native save dialog, which is its own window
# with its own class (`org.freedesktop.impl.portal.desktop.kde` here), and
# without this the driver activates the application behind the dialog and every
# keystroke lands in the wrong place. Geometry is read for whichever window
# this names, so coordinates stay window-local either way.
WINDOW_CLASS = os.environ.get("HANI_WINDOW_CLASS", "hanisculpt")

EV_SYN, EV_KEY, EV_REL, EV_ABS = 0, 1, 2, 3
SYN_REPORT = 0
ABS_X, ABS_Y = 0, 1
REL_WHEEL = 8
BTN = {"left": 0x110, "right": 0x111, "middle": 0x112}
KEYS = {
    "esc": 1, "1": 2, "2": 3, "3": 4, "4": 5, "5": 6, "6": 7, "7": 8, "8": 9,
    "q": 16, "w": 17, "e": 18, "r": 19, "t": 20, "y": 21, "u": 22, "[": 26, "]": 27,
    "s": 31, "f": 33, "h": 35, "z": 44, "x": 45, "c": 46, "v": 47, "m": 50,
    "ctrl": 29, "shift": 42, "alt": 56, "space": 57, "tab": 15, "enter": 28,
}


def _ioc(direction, typ, nr, size):
    return (direction << 30) | (size << 16) | (typ << 8) | nr


UI_SET_EVBIT = _ioc(1, 0x55, 100, 4)
UI_SET_KEYBIT = _ioc(1, 0x55, 101, 4)
UI_SET_RELBIT = _ioc(1, 0x55, 102, 4)
UI_SET_ABSBIT = _ioc(1, 0x55, 103, 4)
UI_DEV_CREATE = _ioc(0, 0x55, 1, 0)
UI_DEV_DESTROY = _ioc(0, 0x55, 2, 0)
# The newer way to describe a device, which is the only one that can say an
# axis's resolution: libinput refuses a tablet whose X and Y have none.
UI_DEV_SETUP = _ioc(1, 0x55, 3, 92)
UI_ABS_SETUP = _ioc(1, 0x55, 4, 28)
UI_SET_PROPBIT = _ioc(1, 0x55, 110, 4)
INPUT_PROP_POINTER = 0
ABS_PRESSURE, ABS_TILT_X, ABS_TILT_Y = 0x18, 0x1A, 0x1B
BTN_TOOL_PEN, BTN_TOOL_RUBBER, BTN_TOUCH = 0x140, 0x141, 0x14A
PEN_BUTTON = {"lower": 0x14B, "upper": 0x14C}
# What the virtual tablet counts in: units across the output per logical
# pixel, levels of pressure, and degrees of lean either way.
PEN_UNITS, PEN_LEVELS, PEN_LEAN = 20, 4095, 60


def outputs():
    """Every output that is on, as its logical rectangle, from `kscreen-doctor`.

    One that is off is still listed, with the place it would have: a laptop's
    own panel with the lid shut, beside the monitor. Counted, it stretched the
    union past the desktop and every coordinate fell short of where it aimed.
    """
    try:
        out = subprocess.run(
            ["kscreen-doctor", "-o"], capture_output=True, text=True, timeout=15
        ).stdout
    except Exception:
        return []
    # Geometry lines look like "Geometry:  786,0 1536x864", with colour codes.
    plain = re.sub(r"\x1b\[[0-9;]*m", "", out)
    found = []
    for block in re.split(r"(?m)^Output:", plain)[1:]:
        if re.search(r"(?m)^\s*disabled\s*$", block):
            continue
        geometry = re.search(r"Geometry:\s*(\d+),(\d+)\s+(\d+)x(\d+)", block)
        if geometry:
            found.append(tuple(int(v) for v in geometry.groups()))
    return found


def desktop_union():
    """The logical rectangle every output sits inside.

    The ABS range maps onto this, so it has to cover both monitors or the far
    one is unreachable.
    """
    width = height = 0
    for x, y, w, h in outputs():
        width = max(width, x + w)
        height = max(height, y + h)
    return (width or 3840), (height or 2160)


def output_of(rect):
    """The output a window's middle is on: the one KWin lays a tablet over."""
    x, y, w, h = rect
    cx, cy = x + w / 2, y + h / 2
    for ox, oy, ow, oh in outputs():
        if ox <= cx < ox + ow and oy <= cy < oy + oh:
            return ox, oy, ow, oh
    raise SystemExit("the window's middle is on no output kscreen-doctor lists")


_EAR = None


def _ear():
    """A name on the session bus for a KWin script to answer to, made once.

    `None` where Python has no D-Bus: the journal is then the only way back.
    """
    global _EAR
    if _EAR is None:
        try:
            import dbus
            import dbus.service
            from dbus.mainloop.glib import DBusGMainLoop
            from gi.repository import GLib
        except ImportError:
            _EAR = False
            return None
        DBusGMainLoop(set_as_default=True)
        bus = dbus.SessionBus()
        name = f"org.hanisculpt.Drive{os.getpid()}"
        heard = []

        class Ear(dbus.service.Object):
            @dbus.service.method("org.hanisculpt.Drive", in_signature="s", out_signature="")
            def Say(self, text):
                heard.append(str(text))

        # Kept: the name and the object last as long as these are held.
        _EAR = (name, heard, GLib, dbus.service.BusName(name, bus), Ear(bus, "/"))
    return _EAR or None


def kwin_ask(body):
    """Ask KWin a question and read the answer.

    A KWin script has no return value. Its `print` goes to the journal on
    some desktops and nowhere at all on others (Fedora's, as found on
    2026-11-27: every question came back empty and the window "was not
    running"). So `print` is made a call back to this process over the
    session bus, which a script can make anywhere, and the journal is read
    only where Python has no D-Bus to be called on.

    Each question carries a unique marker and finds its own answer by it:
    reusing one reads a previous run's reply. Same device as
    `scripts/shot.sh`, which asks through this file's `--ask`.
    """
    marker = f"drive-{os.getpid()}-{time.time_ns() % 100000}"
    path = f"/tmp/kwin-drive-{marker}.js"
    ear = _ear()
    prelude = f'const MARKER = "{marker}";\n'
    if ear:
        prelude += (f'function print(text) {{ callDBus("{ear[0]}", "/", '
                    '"org.hanisculpt.Drive", "Say", "" + text); }\n')
    with open(path, "w") as handle:
        handle.write(f"{prelude}{body}\n")
    name = f"drive-{marker}"
    try:
        ident = subprocess.run(
            [QDBUS, "org.kde.KWin", "/Scripting",
             "org.kde.kwin.Scripting.loadScript", path, name],
            capture_output=True, text=True, timeout=15,
        ).stdout.strip()
        subprocess.run([QDBUS, "org.kde.KWin", f"/Scripting/Script{ident}",
                        "org.kde.kwin.Script.run"], capture_output=True, timeout=15)
        if ear:
            _, heard, glib, *_ = ear
            # The call back is on its way as `run` returns. Let it land.
            context = glib.MainContext.default()
            deadline = time.time() + 2.0
            while time.time() < deadline and not any(marker in line for line in heard):
                context.iteration(False)
                time.sleep(0.01)
            lines = list(heard)
        else:
            time.sleep(0.3)
            lines = subprocess.run(
                ["journalctl", "--user", "-b", "--since", "-20s"],
                capture_output=True, text=True, timeout=20,
            ).stdout.splitlines()
        subprocess.run([QDBUS, "org.kde.KWin", "/Scripting",
                        "org.kde.kwin.Scripting.unloadScript", name],
                       capture_output=True, timeout=15)
        hits = [line for line in lines if marker in line]
        if not hits:
            return ""
        return hits[-1].split(marker, 1)[1].strip()
    finally:
        try:
            os.unlink(path)
        except OSError:
            pass


def window_rect():
    """Where the application's window is, in logical desktop pixels."""
    answer = kwin_ask(
        'for (const w of workspace.windowList()) {'
        f'  const k = (w.resourceClass || "").toString().toLowerCase();'
        f'  if (k.includes("{WINDOW_CLASS}")) {{'
        '    print(MARKER + " " + Math.round(w.frameGeometry.x) + " "'
        '          + Math.round(w.frameGeometry.y) + " "'
        '          + Math.round(w.frameGeometry.width) + " "'
        '          + Math.round(w.frameGeometry.height)); break; }'
        '}'
    )
    parts = answer.split()
    if len(parts) != 4:
        raise SystemExit(
            f"could not find a {WINDOW_CLASS} window through KWin (got {answer!r}) -- "
            "is the application running?"
        )
    return tuple(int(p) for p in parts)


def activate():
    """Bring the window forward, and CHECK that it came.

    Activation is a request, not a guarantee. Every key below goes to whatever
    is focused, so a failed activation types into the user's terminal instead --
    which is how a screenshot once showed a brush changed and a mirror plane on
    that no code had touched.
    """
    active = ""
    # Asked up to three times: raising a window is a request to the compositor
    # and it loses to a window that is still mapping, or to one that takes focus
    # back. One attempt reads as "the application is not there".
    for _ in range(3):
        kwin_ask(
            'for (const w of workspace.windowList()) {'
            f'  const k = (w.resourceClass || "").toString().toLowerCase();'
            f'  if (k.includes("{WINDOW_CLASS}")) {{ workspace.activeWindow = w;'
            '    print(MARKER + " ok"); break; }}'
            '}'
        )
        time.sleep(0.8)
        active = kwin_ask('const a = workspace.activeWindow;'
                          'print(MARKER + " " + (a ? a.resourceClass : "none"));')
        if WINDOW_CLASS in active.lower():
            return
    raise SystemExit(
        f"refusing to drive: '{active}' has focus, not {WINDOW_CLASS}. "
        "Every keystroke would land in it."
    )


class Device:
    def __init__(self, origin):
        self.ox, self.oy = origin
        self.width, self.height = desktop_union()
        self.fd = os.open(UINPUT, os.O_WRONLY | os.O_NONBLOCK)
        for ev in (EV_KEY, EV_ABS, EV_REL, EV_SYN):
            fcntl.ioctl(self.fd, UI_SET_EVBIT, ev)
        for code in list(BTN.values()) + list(KEYS.values()):
            fcntl.ioctl(self.fd, UI_SET_KEYBIT, code)
        for axis in (ABS_X, ABS_Y):
            fcntl.ioctl(self.fd, UI_SET_ABSBIT, axis)
        fcntl.ioctl(self.fd, UI_SET_RELBIT, REL_WHEEL)

        absmax = [0] * 64
        absmax[ABS_X], absmax[ABS_Y] = self.width - 1, self.height - 1
        os.write(self.fd, struct.pack(
            "80sHHHHi" + "64i" * 4, b"hani-drive", 0x03, 0x1234, 0x5678, 1, 0,
            *absmax, *([0] * 64), *([0] * 64), *([0] * 64),
        ))
        fcntl.ioctl(self.fd, UI_DEV_CREATE)
        # The compositor has to notice the device before it will route to it.
        time.sleep(0.5)

    def _emit(self, typ, code, value):
        os.write(self.fd, struct.pack("llHHi", 0, 0, typ, code, value))

    def _syn(self):
        self._emit(EV_SYN, SYN_REPORT, 0)

    def move(self, lx, ly):
        x = max(0, min(self.width - 1, int(round(self.ox + lx))))
        y = max(0, min(self.height - 1, int(round(self.oy + ly))))
        self._emit(EV_ABS, ABS_X, x)
        self._emit(EV_ABS, ABS_Y, y)
        self._syn()
        time.sleep(0.012)

    def button(self, name, down):
        self._emit(EV_KEY, BTN[name], 1 if down else 0)
        self._syn()
        time.sleep(0.05)

    def key(self, name, down):
        if name not in KEYS:
            raise SystemExit(f"no key code for {name!r}; add it to KEYS")
        self._emit(EV_KEY, KEYS[name], 1 if down else 0)
        self._syn()
        time.sleep(0.04)

    def scroll(self, notches):
        step = 1 if notches > 0 else -1
        for _ in range(abs(notches)):
            self._emit(EV_REL, REL_WHEEL, step)
            self._syn()
            time.sleep(0.06)

    def drag(self, x1, y1, x2, y2, steps=24, button="left"):
        # Many small steps and never one jump: an application tells a drag from
        # a click by travel, and a single leap reads as neither.
        self.move(x1, y1)
        time.sleep(0.15)
        self.button(button, True)
        time.sleep(0.12)
        for i in range(1, steps + 1):
            t = i / steps
            self.move(x1 + (x2 - x1) * t, y1 + (y2 - y1) * t)
        time.sleep(0.15)
        self.button(button, False)
        time.sleep(0.25)

    def close(self):
        time.sleep(0.25)
        fcntl.ioctl(self.fd, UI_DEV_DESTROY)
        os.close(self.fd)


class Pen:
    """A virtual tablet with one pen. See the module's notes on the mapping."""

    def __init__(self, window):
        wx, wy, _, _ = window
        ox, oy, self.width, self.height = output_of(window)
        # The window's corner within the output the tablet is laid over.
        self.ox, self.oy = wx - ox, wy - oy
        self.tool = BTN_TOOL_PEN
        self.in_range = False
        self.primed = False
        self.touching = False
        self.tilt = (0, 0)
        self.fd = os.open(UINPUT, os.O_WRONLY | os.O_NONBLOCK)
        for ev in (EV_KEY, EV_ABS, EV_SYN):
            fcntl.ioctl(self.fd, UI_SET_EVBIT, ev)
        for code in (BTN_TOOL_PEN, BTN_TOOL_RUBBER, BTN_TOUCH, *PEN_BUTTON.values()):
            fcntl.ioctl(self.fd, UI_SET_KEYBIT, code)
        fcntl.ioctl(self.fd, UI_SET_PROPBIT, INPUT_PROP_POINTER)
        # Wacom's vendor, so the desktop treats it as it treats a tablet, and
        # a product of no real one.
        fcntl.ioctl(self.fd, UI_DEV_SETUP,
                    struct.pack("HHHH80sI", 0x03, 0x056A, 0x7E57, 1, b"hani-pen", 0))
        for axis, low, high, resolution in (
            (ABS_X, 0, self.width * PEN_UNITS - 1, PEN_UNITS * 4),
            (ABS_Y, 0, self.height * PEN_UNITS - 1, PEN_UNITS * 4),
            (ABS_PRESSURE, 0, PEN_LEVELS, 0),
            (ABS_TILT_X, -PEN_LEAN, PEN_LEAN, 57),
            (ABS_TILT_Y, -PEN_LEAN, PEN_LEAN, 57),
        ):
            fcntl.ioctl(self.fd, UI_SET_ABSBIT, axis)
            fcntl.ioctl(self.fd, UI_ABS_SETUP,
                        struct.pack("HHiiiiii", axis, 0, 0, low, high, 0, 0, resolution))
        fcntl.ioctl(self.fd, UI_DEV_CREATE)
        # Longer than the pointer's: the compositor makes a tablet of it and
        # tells the window, which is two more steps.
        time.sleep(1.5)

    def _emit(self, typ, code, value):
        os.write(self.fd, struct.pack("llHHi", 0, 0, typ, code, value))

    def at(self, lx, ly, pressure=0.0):
        x = (self.ox + lx) * PEN_UNITS
        y = (self.oy + ly) * PEN_UNITS
        self._emit(EV_ABS, ABS_X, int(max(0, min(self.width * PEN_UNITS - 1, round(x)))))
        self._emit(EV_ABS, ABS_Y, int(max(0, min(self.height * PEN_UNITS - 1, round(y)))))
        self._emit(EV_ABS, ABS_TILT_X, self.tilt[0])
        self._emit(EV_ABS, ABS_TILT_Y, self.tilt[1])
        if not self.in_range:
            if not self.primed:
                # libinput takes a pen that goes quiet for 50 ms to have
                # left, until the tablet has once said so itself: there are
                # tablets that never do. A real pen is never quiet in range
                # and this one is, at every `sleep`. So it leaves once, in
                # so many words, and is believed from then on. Without this
                # a pen held over a button was gone before the tooltip came.
                self._emit(EV_KEY, self.tool, 1)
                self._emit(EV_SYN, SYN_REPORT, 0)
                time.sleep(0.02)
                self._emit(EV_KEY, self.tool, 0)
                self._emit(EV_SYN, SYN_REPORT, 0)
                time.sleep(0.02)
                self.primed = True
            # A pen comes into range with a frame of its own, as a real one
            # does: where it is and which end, and nothing else.
            self._emit(EV_KEY, self.tool, 1)
            self._emit(EV_SYN, SYN_REPORT, 0)
            self.in_range = True
            time.sleep(0.05)
        self._emit(EV_ABS, ABS_PRESSURE, int(round(max(0.0, min(1.0, pressure)) * PEN_LEVELS)))
        touching = pressure > 0
        if touching != self.touching:
            self._emit(EV_KEY, BTN_TOUCH, 1 if touching else 0)
            self.touching = touching
        self._emit(EV_SYN, SYN_REPORT, 0)
        time.sleep(0.012)

    def drag(self, x1, y1, x2, y2, p1=0.5, p2=0.5, steps=24):
        self.at(x1, y1)
        time.sleep(0.15)
        self.at(x1, y1, p1)
        time.sleep(0.12)
        for i in range(1, steps + 1):
            t = i / steps
            self.at(x1 + (x2 - x1) * t, y1 + (y2 - y1) * t, p1 + (p2 - p1) * t)
        time.sleep(0.15)
        self.at(x2, y2)
        time.sleep(0.25)

    def button(self, name, down):
        self._emit(EV_KEY, PEN_BUTTON[name], 1 if down else 0)
        self._emit(EV_SYN, SYN_REPORT, 0)
        time.sleep(0.05)

    def out(self):
        if not self.in_range:
            return
        if self.touching:
            self._emit(EV_ABS, ABS_PRESSURE, 0)
            self._emit(EV_KEY, BTN_TOUCH, 0)
            self._emit(EV_SYN, SYN_REPORT, 0)
            self.touching = False
        self._emit(EV_KEY, self.tool, 0)
        self._emit(EV_SYN, SYN_REPORT, 0)
        self.in_range = False
        time.sleep(0.05)

    def close(self):
        self.out()
        time.sleep(0.25)
        fcntl.ioctl(self.fd, UI_DEV_DESTROY)
        os.close(self.fd)


def run(path):
    activate()
    x, y, w, h = window_rect()
    print(f"window at {x},{y} size {w}x{h}", flush=True)
    dev = Device((x, y))
    pen = None

    def the_pen():
        nonlocal pen
        if pen is None:
            pen = Pen((x, y, w, h))
        return pen

    try:
        for raw in open(path).read().splitlines():
            line = raw.split("#")[0].strip()
            if not line:
                continue
            cmd, *args = line.split()
            if cmd == "move":
                dev.move(float(args[0]), float(args[1]))
            elif cmd == "down":
                dev.button(args[0] if args else "left", True)
            elif cmd == "up":
                dev.button(args[0] if args else "left", False)
            elif cmd == "click":
                name = args[0] if args else "left"
                dev.button(name, True)
                dev.button(name, False)
            elif cmd in ("drag", "mdrag"):
                steps = int(args[4]) if len(args) > 4 else 24
                dev.drag(*[float(a) for a in args[:4]], steps=steps,
                         button="middle" if cmd == "mdrag" else "left")
            elif cmd == "key":
                dev.key(args[0], True)
                dev.key(args[0], False)
            elif cmd == "keydown":
                dev.key(args[0], True)
            elif cmd == "keyup":
                dev.key(args[0], False)
            elif cmd == "scroll":
                dev.scroll(int(args[0]))
            elif cmd == "sleep":
                time.sleep(float(args[0]) / 1000.0)
            elif cmd == "pen":
                the_pen().at(float(args[0]), float(args[1]),
                             float(args[2]) if len(args) > 2 else 0.0)
            elif cmd == "pendrag":
                p1 = float(args[4]) if len(args) > 4 else 0.5
                p2 = float(args[5]) if len(args) > 5 else p1
                steps = int(args[6]) if len(args) > 6 else 24
                the_pen().drag(*[float(a) for a in args[:4]], p1=p1, p2=p2, steps=steps)
            elif cmd == "pentilt":
                the_pen().tilt = tuple(int(round(float(a) * PEN_LEAN)) for a in args[:2])
            elif cmd == "penbutton":
                the_pen().button(args[0], args[1] == "down")
            elif cmd == "eraser":
                the_pen().out()
                the_pen().tool = BTN_TOOL_RUBBER if args[0] == "on" else BTN_TOOL_PEN
            elif cmd == "penout":
                the_pen().out()
            else:
                raise SystemExit(f"unknown command: {cmd}")
            print(f"ok: {line}", flush=True)
    finally:
        if pen is not None:
            pen.close()
        dev.close()


if __name__ == "__main__":
    if len(sys.argv) < 2:
        raise SystemExit(__doc__)
    if sys.argv[1] == "--geometry":
        print("%d %d %d %d" % window_rect())
    elif sys.argv[1] == "--ask":
        # A question for KWin, for `shot.sh`: the script's body, in which
        # `print(MARKER + " " + answer)` is the answer.
        print(kwin_ask(sys.argv[2]))
    else:
        run(sys.argv[1])
