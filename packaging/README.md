<!-- SPDX-License-Identifier: AGPL-3.0-only -->

# Packaging bits

Not a packaged build — see the README's "Get it", which says to build from
source and means it. These are the two files a Linux desktop needs in order to
show HaniSculpt as an application rather than as an unnamed window.

## Why the icon lives here as well as in the application

The application sets Haniko's badge as its window icon (`logo.rs`), which X11
and Windows show. Wayland does not: there is no protocol for a client to hand
its compositor an icon, and `winit`'s Wayland backend implements
`set_window_icon` as an empty function. The compositor matches the surface's
`app_id`, `hanisculpt`, against an installed `.desktop` file and takes the
icon from the icon theme.

This is also what gives the application a menu entry with a name, which is the
larger part of what it is for.

## Installing for the current user

```fish
install -Dm644 packaging/hanisculpt.desktop ~/.local/share/applications/hanisculpt.desktop
install -Dm644 assets/brand/haniko/hanisculpt-icon-512.png ~/.local/share/icons/hicolor/512x512/apps/hanisculpt.png
install -Dm644 assets/brand/haniko/hanisculpt-icon-256.png ~/.local/share/icons/hicolor/256x256/apps/hanisculpt.png
install -Dm644 assets/brand/haniko/hanisculpt-icon-128.png ~/.local/share/icons/hicolor/128x128/apps/hanisculpt.png
install -Dm644 assets/brand/haniko/hanisculpt-icon-64.png ~/.local/share/icons/hicolor/64x64/apps/hanisculpt.png
install -Dm644 assets/brand/haniko/hanisculpt-icon-32.png ~/.local/share/icons/hicolor/32x32/apps/hanisculpt.png
update-desktop-database ~/.local/share/applications 2>/dev/null; gtk-update-icon-cache -f -t ~/.local/share/icons/hicolor 2>/dev/null; true
```

`Exec=brokkrsculpt` assumes the binary is on `PATH`. Running it from
`target/release` instead is fine — the icon matches on `StartupWMClass`, not on
the command.
