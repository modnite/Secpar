//! PRS sends Windows virtual keys; macOS wants ANSI virtual key codes
//! (kVK_* in HIToolbox/Events.h). Physical position is preserved.

pub fn mac_key(vk: u32) -> Option<u16> {
    Some(match vk {
        0x08 => 51,        // Backspace -> Delete
        0x09 => 48,        // Tab
        0x0d => 36,        // Return
        0x10 | 0xa0 => 56, // Shift
        0xa1 => 60,
        0x11 | 0xa2 => 59, // Control
        0xa3 => 62,
        0x12 | 0xa4 => 58, // Alt -> Option
        0xa5 => 61,
        0x14 => 57, // Caps Lock
        0x1b => 53, // Escape
        0x20 => 49, // Space
        0x21 => 116,
        0x22 => 121,
        0x23 => 119,
        0x24 => 115,
        0x25 => 123,
        0x26 => 126,
        0x27 => 124,
        0x28 => 125,
        0x2e => 117, // Delete -> Forward Delete
        0x30 => 29,
        0x31 => 18,
        0x32 => 19,
        0x33 => 20,
        0x34 => 21,
        0x35 => 23,
        0x36 => 22,
        0x37 => 26,
        0x38 => 28,
        0x39 => 25,
        0x41 => 0,
        0x42 => 11,
        0x43 => 8,
        0x44 => 2,
        0x45 => 14,
        0x46 => 3,
        0x47 => 5,
        0x48 => 4,
        0x49 => 34,
        0x4a => 38,
        0x4b => 40,
        0x4c => 37,
        0x4d => 46,
        0x4e => 45,
        0x4f => 31,
        0x50 => 35,
        0x51 => 12,
        0x52 => 15,
        0x53 => 1,
        0x54 => 17,
        0x55 => 32,
        0x56 => 9,
        0x57 => 13,
        0x58 => 7,
        0x59 => 16,
        0x5a => 6,
        0x5b => 55, // Left Windows -> Command
        0x5c => 54,
        0x60 => 82,
        0x61 => 83,
        0x62 => 84,
        0x63 => 85,
        0x64 => 86,
        0x65 => 87,
        0x66 => 88,
        0x67 => 89,
        0x68 => 91,
        0x69 => 92,
        0x6a => 67,
        0x6b => 69,
        0x6d => 78,
        0x6e => 65,
        0x6f => 75,
        0x70 => 122, // F1
        0x71 => 120,
        0x72 => 99,
        0x73 => 118,
        0x74 => 96,
        0x75 => 97,
        0x76 => 98,
        0x77 => 100,
        0x78 => 101,
        0x79 => 109, // F10
        0x7a => 103,
        0x7b => 111,
        0xba => 41, // ;
        0xbb => 24, // =
        0xbc => 43, // ,
        0xbd => 27, // -
        0xbe => 47, // .
        0xbf => 44, // /
        0xc0 => 50, // `
        0xdb => 33, // [
        0xdc => 42, // \
        0xdd => 30, // ]
        0xde => 39, // '
        0xe2 => 10, // ISO extra key
        _ => return None,
    })
}
/// CGEventSourceButtonState / CGMouseButton numbering: 0 left, 1 right, 2 centre, 3+ other.
pub fn mouse_button(name: &str) -> Option<u32> {
    Some(match name {
        "left" => 0,
        "right" => 1,
        "middle" => 2,
        "back" | "x1" => 3,
        "forward" | "x2" => 4,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn letters_modifiers_arrows_and_function_keys_map_to_mac_codes() {
        assert_eq!(mac_key(0x41), Some(0)); // A
        assert_eq!(mac_key(0x5a), Some(6)); // Z
        assert_eq!(mac_key(0x5b), Some(55)); // Win -> Command
        assert_eq!(mac_key(0x25), Some(123));
        assert_eq!(mac_key(0x70), Some(122));
        assert_eq!(mac_key(0xffff), None);
    }
    #[test]
    fn no_two_virtual_keys_collide_except_deliberate_modifier_aliases() {
        let mut seen = std::collections::HashMap::new();
        for vk in 0..0x100u32 {
            if let Some(code) = mac_key(vk) {
                if let Some(other) = seen.insert(code, vk) {
                    assert!(matches!((other, vk), (0x10, 0xa0) | (0x11, 0xa2) | (0x12, 0xa4)), "{other:#x} and {vk:#x} share {code}");
                }
            }
        }
    }
    #[test]
    fn auxiliary_buttons_are_preserved() {
        assert_eq!(mouse_button("back"), Some(3));
        assert_eq!(mouse_button("nope"), None);
    }
}
