//! Keyboard handling without a server round trip per key.
//!
//! The core keyboard map is fetched once and again on MappingNotify. Levels
//! follow nirulauncher's `keyboard_level`/`keyboard_symbol` (Shift, AltGr,
//! Caps Lock), with one correction for real XKB servers: the core map lists
//! a key as `G1L1 G1L2 G2L1 G2L2 G1L3 G1L4`, so AltGr's level 3 is index 4
//! (`xmodmap -pke` on othala: `keycode 11 = 2 quotedbl 2 quotedbl at …`).
//! Index 2 is still used when a map has no fifth column.

use x11rb::{
    connection::Connection,
    protocol::xproto::{ConnectionExt as _, KeyButMask},
    rust_connection::RustConnection,
};

pub struct Keymap {
    min: u8,
    per: usize,
    syms: Vec<u32>,
}

impl Keymap {
    pub fn load(c: &RustConnection) -> Self {
        let min = c.setup().min_keycode;
        let max = c.setup().max_keycode;
        let reply = c
            .get_keyboard_mapping(min, max - min + 1)
            .ok()
            .and_then(|r| r.reply().ok());
        match reply {
            Some(map) => Self {
                min,
                per: map.keysyms_per_keycode.max(1) as usize,
                syms: map.keysyms,
            },
            None => Self {
                min,
                per: 1,
                syms: Vec::new(),
            },
        }
    }

    fn syms(&self, code: u8) -> &[u32] {
        let Some(index) = code.checked_sub(self.min) else {
            return &[];
        };
        let start = index as usize * self.per;
        self.syms.get(start..start + self.per).unwrap_or(&[])
    }

    pub fn symbol(&self, code: u8, state: KeyButMask) -> u32 {
        keyboard_symbol(self.syms(code), state)
    }
}

fn is_keypad(sym: u32) -> bool {
    (0xffaa..=0xffb9).contains(&sym) || sym == 0xffac
}

/// X applies Caps Lock to alphabetic keysyms, including Latin-1 letters.
fn apply_caps(sym: u32, lock: bool) -> u32 {
    if !lock {
        return sym;
    }
    if (0x61..=0x7a).contains(&sym) || (0xe0..=0xfe).contains(&sym) && sym != 0xf7 {
        return sym - 0x20;
    }
    if (0x41..=0x5a).contains(&sym) || (0xc0..=0xde).contains(&sym) && sym != 0xd7 {
        return sym + 0x20;
    }
    sym
}

pub fn keyboard_symbol(syms: &[u32], state: KeyButMask) -> u32 {
    let get = |i: usize| syms.get(i).copied().filter(|s| *s != 0);
    let mut shift = state.contains(KeyButMask::SHIFT);
    // Num Lock selects the keypad's digits, and Shift undoes it.
    if state.contains(KeyButMask::MOD2) && get(1).is_some_and(is_keypad) {
        shift = !shift;
    }
    let base = if state.contains(KeyButMask::MOD5) {
        if get(4).is_some() {
            4
        } else {
            2
        }
    } else {
        0
    };
    let sym = (if shift { get(base + 1) } else { None })
        .or_else(|| get(base))
        .or_else(|| get(0))
        .unwrap_or(0);
    apply_caps(sym, state.contains(KeyButMask::LOCK))
}

/// The text a keysym types, if any.
pub fn keysym_char(sym: u32) -> Option<char> {
    match sym {
        0x20..=0x7e | 0xa0..=0xff => char::from_u32(sym),
        0x20ac => Some('€'),
        0xffaa => Some('*'),
        0xffab => Some('+'),
        0xffac => Some(','),
        0xffad => Some('-'),
        0xffae => Some('.'),
        0xffaf => Some('/'),
        0xffb0..=0xffb9 => char::from_u32(sym - 0xffb0 + '0' as u32),
        0x0100_0100..=0x0110_ffff => char::from_u32(sym - 0x0100_0000),
        _ => None,
    }
}

/// A letter keysym folded to lower case, so shortcuts ignore Shift and Caps.
pub fn shortcut(sym: u32) -> u32 {
    if (0x41..=0x5a).contains(&sym) {
        sym + 0x20
    } else {
        sym
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Rows from othala's Swedish map (`xmodmap -pke`).
    const TWO: [u32; 7] = [0x32, 0x22, 0x32, 0x22, 0x40, 0xb2, 0x40]; // 2 " 2 " @ ² @
    const E: [u32; 7] = [0x65, 0x45, 0x65, 0x45, 0x20ac, 0xa2, 0x20ac];
    const ARING: [u32; 7] = [0xe5, 0xc5, 0xe5, 0xc5, 0xfe57, 0xfe58, 0xfe57];
    const PLUS: [u32; 7] = [0x2b, 0x3f, 0x2b, 0x3f, 0x5c, 0xbf, 0x5c];
    const KP7: [u32; 2] = [0xff95, 0xffb7];

    #[test]
    fn altgr_uses_level_three_of_a_real_xkb_map() {
        assert_eq!(keyboard_symbol(&TWO, KeyButMask::MOD5), '@' as u32);
        assert_eq!(keyboard_symbol(&E, KeyButMask::MOD5), 0x20ac);
        assert_eq!(keyboard_symbol(&PLUS, KeyButMask::MOD5), '\\' as u32);
        assert_eq!(
            keyboard_symbol(&TWO, KeyButMask::MOD5 | KeyButMask::SHIFT),
            0xb2
        );
    }

    #[test]
    fn a_short_map_falls_back_to_index_two_for_altgr() {
        // nirulauncher's test layout.
        let syms = [0x32, 0x22, 0x40, 0x60];
        assert_eq!(keyboard_symbol(&syms, KeyButMask::MOD5), 0x40);
        assert_eq!(
            keyboard_symbol(&syms, KeyButMask::SHIFT | KeyButMask::MOD5),
            0x60
        );
    }

    #[test]
    fn shift_and_caps() {
        assert_eq!(keyboard_symbol(&TWO, KeyButMask::SHIFT), '"' as u32);
        assert_eq!(keyboard_symbol(&PLUS, KeyButMask::default()), '+' as u32);
        assert_eq!(keyboard_symbol(&ARING, KeyButMask::LOCK), 0xc5);
        assert_eq!(
            keyboard_symbol(&ARING, KeyButMask::LOCK | KeyButMask::SHIFT),
            0xe5
        );
        assert_eq!(keyboard_symbol(&TWO, KeyButMask::LOCK), '2' as u32);
    }

    #[test]
    fn num_lock_selects_keypad_digits() {
        assert_eq!(keyboard_symbol(&KP7, KeyButMask::default()), 0xff95);
        assert_eq!(keyboard_symbol(&KP7, KeyButMask::MOD2), 0xffb7);
        assert_eq!(
            keyboard_symbol(&KP7, KeyButMask::MOD2 | KeyButMask::SHIFT),
            0xff95
        );
        assert_eq!(keysym_char(0xffb7), Some('7'));
    }

    #[test]
    fn text_from_keysyms() {
        assert_eq!(keysym_char(0x40), Some('@'));
        assert_eq!(keysym_char(0xe5), Some('å'));
        assert_eq!(keysym_char(0x20ac), Some('€'));
        assert_eq!(keysym_char(0x0100_2192), Some('→'));
        assert_eq!(keysym_char(0xff0d), None, "Return");
        assert_eq!(keysym_char(0xfe57), None, "dead key");
        assert_eq!(keysym_char(0xffe3), None, "Control_L");
    }

    #[test]
    fn shortcuts_ignore_case() {
        assert_eq!(shortcut('Z' as u32), 'z' as u32);
        assert_eq!(shortcut('z' as u32), 'z' as u32);
        assert_eq!(shortcut('+' as u32), '+' as u32);
    }
}
