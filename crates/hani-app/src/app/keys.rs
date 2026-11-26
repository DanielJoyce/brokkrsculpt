// SPDX-License-Identifier: AGPL-3.0-only

//! Which brush each of the keys 1 to 8 picks, in each mode: R2 of
//! `docs/WORKSPACES.md`.
//!
//! The keys pick from the open mode's palette, and the brushes on them are
//! the user's to choose: right click a tile, and press the key to put it
//! on. A brush is named rather than placed, so a key goes on picking the
//! same brush whatever is added to the palette before it. A key nobody has
//! set picks what the palette has in that place, which for Sculpt is
//! Draw, Clay, Smooth, Inflate, Pinch, Flatten, Widen and Scrape, and for Transform the first
//! eight parts.
//!
//! Kept in `keys.conf` in the settings directory, one `mode.key = name`
//! a line: what a user chose, which survives a reinstall. Paint's keys are
//! the rack in hand's first eight, painting in colour, and the touches by
//! filament slot; neither is set here. See `app/rack.rs`.

use std::path::Path;

use crate::message::Workspace;

/// How many keys there are.
pub(crate) const KEYS: usize = crate::viewport::KEYED_BRUSHES;

/// The file the keys are kept in.
const FILE: &str = "keys.conf";

/// The brush named on each key of the modes whose palette is the brushes':
/// `None` where the key is the palette's own place.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Keys {
    sculpt: [Option<String>; KEYS],
    transform: [Option<String>; KEYS],
}

impl Keys {
    fn of(&self, mode: Workspace) -> Option<&[Option<String>; KEYS]> {
        match mode {
            Workspace::Sculpt => Some(&self.sculpt),
            Workspace::Transform => Some(&self.transform),
            Workspace::Paint | Workspace::Print => None,
        }
    }

    fn of_mut(&mut self, mode: Workspace) -> Option<&mut [Option<String>; KEYS]> {
        match mode {
            Workspace::Sculpt => Some(&mut self.sculpt),
            Workspace::Transform => Some(&mut self.transform),
            Workspace::Paint | Workspace::Print => None,
        }
    }

    /// The name of the brush on `key` (0 for the key 1) in `mode`: the one
    /// put there, or the one `default` says is in that place.
    pub(crate) fn name(
        &self,
        mode: Workspace,
        key: usize,
        default: impl Fn(usize) -> Option<String>,
    ) -> Option<String> {
        let set = self.of(mode)?.get(key)?;
        set.clone().or_else(|| default(key))
    }

    /// Which key, if any, picks the brush called `name` in `mode`.
    pub(crate) fn key_of(
        &self,
        mode: Workspace,
        name: &str,
        default: impl Fn(usize) -> Option<String>,
    ) -> Option<usize> {
        (0..KEYS).find(|key| self.name(mode, *key, &default).as_deref() == Some(name))
    }

    /// Put the brush called `name` on `key` in `mode`. A brush is on one
    /// key at most: if it was on another, the brush that was on `key`
    /// goes to that one, so the two swap. A brush that was on no key
    /// takes `key` from the one there, which is still on the palette.
    pub(crate) fn put(
        &mut self,
        mode: Workspace,
        key: usize,
        name: &str,
        default: impl Fn(usize) -> Option<String>,
    ) {
        if key >= KEYS {
            return;
        }
        let was = self.key_of(mode, name, &default);
        let displaced = self.name(mode, key, &default);
        let Some(keys) = self.of_mut(mode) else {
            return;
        };
        if let Some(was) = was.filter(|was| *was != key) {
            keys[was] = displaced;
        }
        keys[key] = Some(name.to_string());
    }

    /// The keys written as `keys.conf` holds them.
    pub(crate) fn to_text(&self) -> String {
        let mut text = String::from("# Which brush each key from 1 to 8 picks, by mode.\n");
        for (mode, keys) in [("sculpt", &self.sculpt), ("transform", &self.transform)] {
            for (key, name) in keys.iter().enumerate() {
                if let Some(name) = name {
                    text.push_str(&format!("{mode}.{} = {name}\n", key + 1));
                }
            }
        }
        text
    }

    /// The keys from what `keys.conf` holds. A line it cannot read is left
    /// out, and its key is the palette's own.
    pub(crate) fn from_text(text: &str) -> Self {
        let mut keys = Self::default();
        for (key, name) in crate::paths::entries(text) {
            let Some((mode, number)) = key.split_once('.') else {
                continue;
            };
            let Some(index) = number.parse::<usize>().ok().and_then(|n| n.checked_sub(1)) else {
                continue;
            };
            let slots = match mode {
                "sculpt" => &mut keys.sculpt,
                "transform" => &mut keys.transform,
                _ => continue,
            };
            if index < KEYS && !name.is_empty() {
                slots[index] = Some(name.to_string());
            }
        }
        keys
    }

    /// The keys this machine has kept.
    pub(crate) fn load() -> Self {
        crate::paths::config_file(FILE)
            .and_then(|path| std::fs::read_to_string(path).ok())
            .map(|text| Self::from_text(&text))
            .unwrap_or_default()
    }

    /// Keep them. Not from a test, which must not write the user's
    /// settings.
    pub(crate) fn save(&self) -> Result<(), String> {
        if cfg!(test) {
            return Ok(());
        }
        let path = crate::paths::config_file(FILE).ok_or("no settings directory")?;
        save_to(&path, &self.to_text())
    }
}

fn save_to(path: &Path, text: &str) -> Result<(), String> {
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory).map_err(|why| why.to_string())?;
    }
    std::fs::write(path, text).map_err(|why| why.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const PLACES: [&str; 10] = [
        "Draw", "Clay", "Smooth", "Inflate", "Pinch", "Flatten", "Widen", "Scrape", "Fill",
        "Polish",
    ];

    fn default(key: usize) -> Option<String> {
        PLACES.get(key).map(|name| name.to_string())
    }

    /// With nothing set, the keys are the palette's places.
    #[test]
    fn unset_keys_are_the_palettes_places() {
        let keys = Keys::default();
        for (key, place) in PLACES.iter().enumerate().take(KEYS) {
            assert_eq!(keys.name(Workspace::Sculpt, key, default).as_deref(), Some(*place));
        }
        assert_eq!(keys.name(Workspace::Paint, 0, default), None);
    }

    proptest! {
        /// Whatever is put on whichever keys, each brush is on one key at
        /// most, the last brush put on a key is on it, and the keys read
        /// back from their file as they were.
        #[test]
        fn a_brush_is_on_one_key_and_the_keys_keep(
            puts in proptest::collection::vec((0usize..KEYS, 0usize..PLACES.len()), 0..20),
        ) {
            let mut keys = Keys::default();
            for (key, brush) in &puts {
                keys.put(Workspace::Sculpt, *key, PLACES[*brush], default);
                prop_assert_eq!(keys.name(Workspace::Sculpt, *key, default), Some(PLACES[*brush].to_string()));
            }
            let names: Vec<String> =
                (0..KEYS).filter_map(|key| keys.name(Workspace::Sculpt, key, default)).collect();
            for name in &names {
                prop_assert_eq!(names.iter().filter(|other| *other == name).count(), 1,
                    "{} is on two keys", name);
            }
            prop_assert_eq!(Keys::from_text(&keys.to_text()), keys.clone());
            // Transform's keys are their own.
            prop_assert_eq!(keys.name(Workspace::Transform, 0, default), Some("Draw".to_string()));
        }
    }

    /// A line the file cannot read leaves its key the palette's.
    #[test]
    fn a_bad_line_is_left_out() {
        let keys =
            Keys::from_text("sculpt.3 = Clay strips\nsculpt.9 = Bead\npaint.1 = x\nnonsense\n");
        assert_eq!(keys.name(Workspace::Sculpt, 2, default).as_deref(), Some("Clay strips"));
        assert_eq!(keys.name(Workspace::Sculpt, 0, default).as_deref(), Some("Draw"));
    }
}
