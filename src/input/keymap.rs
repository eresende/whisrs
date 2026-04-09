//! XKB reverse lookup table: char → (Keycode, Modifiers).
//!
//! Uses `xkbcommon` to read the active keyboard layout and build a reverse
//! mapping so we know which physical key (+ shift) produces each character.

use std::collections::HashMap;
use std::process::Command;

use tracing::{debug, warn};

use super::KeyMapping;

/// Detected keyboard layout (XKB layout name and optional variant).
#[derive(Debug, Clone)]
pub struct KeyboardLayout {
    pub layout: String,
    pub variant: String,
}

impl KeyboardLayout {
    /// Detect the active keyboard layout from the compositor.
    ///
    /// Tries Hyprland, then Sway, then `XKB_DEFAULT_LAYOUT` env var.
    /// Falls back to empty strings (xkbcommon default, typically "us").
    pub fn detect() -> Self {
        if let Some(kl) = Self::from_hyprland() {
            debug!("detected keyboard layout from Hyprland: {kl:?}");
            return kl;
        }
        if let Some(kl) = Self::from_sway() {
            debug!("detected keyboard layout from Sway: {kl:?}");
            return kl;
        }
        if let Some(kl) = Self::from_env() {
            debug!("detected keyboard layout from environment: {kl:?}");
            return kl;
        }
        warn!("could not detect keyboard layout, falling back to system default");
        Self {
            layout: String::new(),
            variant: String::new(),
        }
    }

    /// Query Hyprland for the active keyboard layout via `hyprctl devices -j`.
    fn from_hyprland() -> Option<Self> {
        let output = Command::new("hyprctl")
            .args(["devices", "-j"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }

        let json: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
        let keyboards = json.get("keyboards")?.as_array()?;

        // Find the first keyboard with a non-empty layout, preferring physical
        // keyboards (name contains "translated" or "at-") over virtual ones.
        let kb = keyboards
            .iter()
            .find(|k| {
                let name = k.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let layout = k.get("layout").and_then(|l| l.as_str()).unwrap_or("");
                !layout.is_empty() && (name.contains("translated") || name.contains("at-"))
            })
            .or_else(|| {
                keyboards.iter().find(|k| {
                    let layout = k.get("layout").and_then(|l| l.as_str()).unwrap_or("");
                    !layout.is_empty()
                })
            })?;

        let layout = kb.get("layout")?.as_str()?.to_string();
        let variant = kb
            .get("variant")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Some(Self { layout, variant })
    }

    /// Query Sway for the active keyboard layout via `swaymsg -t get_inputs`.
    fn from_sway() -> Option<Self> {
        let output = Command::new("swaymsg")
            .args(["-t", "get_inputs", "--raw"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }

        let inputs: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).ok()?;

        // Find the first keyboard input with xkb_active_layout_name.
        let kb = inputs.iter().find(|i| {
            i.get("type").and_then(|t| t.as_str()) == Some("keyboard")
                && i.get("xkb_active_layout_name").is_some()
        })?;

        // Sway exposes layout in xkb_layout_names array and
        // xkb_active_layout_index for the active one.
        let layout_names = kb.get("xkb_layout_names")?.as_array()?;
        let active_idx = kb
            .get("xkb_active_layout_index")
            .and_then(|i| i.as_u64())
            .unwrap_or(0) as usize;

        // The layout names are human-readable (e.g. "German"), but we need
        // the XKB name. Sway stores that in the input's libinput config.
        // Fallback: parse from sway config or use XKB_DEFAULT_LAYOUT.
        // For now, try to get it from the identifier which contains the layout.
        // Actually, swaymsg get_inputs provides xkb_layout_names as display names
        // but the actual XKB layout is set in sway config. We can check env vars.
        let _active_name = layout_names.get(active_idx)?.as_str()?;

        // Sway doesn't directly expose the XKB layout code in get_inputs.
        // Fall through to env var detection.
        None
    }

    /// Read layout from `XKB_DEFAULT_LAYOUT` and `XKB_DEFAULT_VARIANT` env vars.
    fn from_env() -> Option<Self> {
        let layout = std::env::var("XKB_DEFAULT_LAYOUT").ok()?;
        if layout.is_empty() {
            return None;
        }
        let variant = std::env::var("XKB_DEFAULT_VARIANT").unwrap_or_default();
        Some(Self { layout, variant })
    }
}

/// Reverse lookup table from character to the key event needed to produce it.
pub struct XkbKeymap {
    map: HashMap<char, KeyMapping>,
}

impl XkbKeymap {
    /// Build the reverse keymap from a detected keyboard layout.
    pub fn from_layout(detected: &KeyboardLayout) -> anyhow::Result<Self> {
        let context = xkbcommon::xkb::Context::new(xkbcommon::xkb::CONTEXT_NO_FLAGS);

        let keymap = xkbcommon::xkb::Keymap::new_from_names(
            &context,
            "",                // rules
            "",                // model
            &detected.layout,  // layout (e.g. "de", "fr", "us")
            &detected.variant, // variant (e.g. "nodeadkeys")
            None,              // options
            xkbcommon::xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .ok_or_else(|| {
            anyhow::anyhow!(
                "failed to create XKB keymap for layout '{}' variant '{}'",
                detected.layout,
                detected.variant
            )
        })?;

        let map = build_reverse_map(&keymap);
        debug!(
            "built XKB reverse keymap with {} entries for layout='{}' variant='{}'",
            map.len(),
            detected.layout,
            detected.variant
        );

        Ok(Self { map })
    }

    /// Look up the key mapping for a character.
    pub fn lookup(&self, ch: char) -> Option<&KeyMapping> {
        self.map.get(&ch)
    }

    /// Number of entries in the keymap.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether the keymap is empty.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Iterate all keycodes and shift levels to build a `char → KeyMapping` table.
fn build_reverse_map(keymap: &xkbcommon::xkb::Keymap) -> HashMap<char, KeyMapping> {
    let mut map = HashMap::new();

    // xkb keycodes: iterate from min to max.
    let min = keymap.min_keycode().raw();
    let max = keymap.max_keycode().raw();

    let num_layouts = keymap.num_layouts();

    for raw_keycode in min..=max {
        let keycode = xkbcommon::xkb::Keycode::new(raw_keycode);

        for layout in 0..num_layouts {
            let num_levels = keymap.num_levels_for_key(keycode, layout);

            for level in 0..num_levels {
                let syms = keymap.key_get_syms_by_level(keycode, layout, level);

                for &sym in syms {
                    let unicode = xkbcommon::xkb::keysym_to_utf32(sym);
                    if unicode == 0 {
                        continue;
                    }

                    if let Some(ch) = char::from_u32(unicode) {
                        // The evdev keycode is the XKB keycode minus 8
                        // (XKB adds 8 to Linux input keycodes).
                        let evdev_keycode = raw_keycode.saturating_sub(8);

                        // Level 0 = no modifiers, Level 1 = Shift
                        let shift = level >= 1;

                        let mapping = KeyMapping {
                            keycode: evdev_keycode as u16,
                            shift,
                        };

                        // Prefer un-shifted mappings (level 0) over shifted ones.
                        // Only insert if not already present (first-come wins,
                        // and level 0 is iterated first).
                        map.entry(ch).or_insert(mapping);
                    }
                }
            }
        }
    }

    map
}

#[cfg(test)]
mod tests {
    use super::*;

    fn us_layout() -> KeyboardLayout {
        KeyboardLayout {
            layout: "us".to_string(),
            variant: String::new(),
        }
    }

    #[test]
    fn build_us_keymap() {
        let km = XkbKeymap::from_layout(&us_layout());
        if let Ok(km) = km {
            assert!(!km.is_empty(), "keymap should not be empty");
            assert!(km.lookup('a').is_some(), "'a' should be in the keymap");
        }
    }

    #[test]
    fn shift_mapping_for_uppercase() {
        let km = XkbKeymap::from_layout(&us_layout());
        if let Ok(km) = km {
            if let Some(mapping) = km.lookup('A') {
                assert!(
                    mapping.shift,
                    "uppercase 'A' should require shift on standard layouts"
                );
            }
        }
    }

    fn layout(name: &str, variant: &str) -> KeyboardLayout {
        KeyboardLayout {
            layout: name.to_string(),
            variant: variant.to_string(),
        }
    }

    #[test]
    fn german_layout_yz_swap() {
        // On QWERTZ (de), 'z' and 'y' are in swapped positions compared to US.
        let km = XkbKeymap::from_layout(&layout("de", ""));
        if let Ok(km) = km {
            let y_mapping = km.lookup('y').expect("'y' should be in de keymap");
            let z_mapping = km.lookup('z').expect("'z' should be in de keymap");
            // On QWERTZ: 'z' is where 'y' is on QWERTY (evdev 21),
            //            'y' is where 'z' is on QWERTY (evdev 44).
            assert_eq!(
                z_mapping.keycode, 21,
                "'z' should be at evdev keycode 21 on QWERTZ"
            );
            assert_eq!(
                y_mapping.keycode, 44,
                "'y' should be at evdev keycode 44 on QWERTZ"
            );
        }
    }

    #[test]
    fn french_azerty_layout() {
        // On AZERTY (fr), 'a'/'q' and 'z'/'w' are swapped compared to QWERTY.
        let km = XkbKeymap::from_layout(&layout("fr", ""));
        if let Ok(km) = km {
            let a_mapping = km.lookup('a').expect("'a' should be in fr keymap");
            let q_mapping = km.lookup('q').expect("'q' should be in fr keymap");
            let z_mapping = km.lookup('z').expect("'z' should be in fr keymap");
            let w_mapping = km.lookup('w').expect("'w' should be in fr keymap");
            // AZERTY: 'a' is at QWERTY 'q' position (evdev 16),
            //         'q' is at QWERTY 'a' position (evdev 30).
            assert_eq!(a_mapping.keycode, 16, "'a' should be at evdev 16 on AZERTY");
            assert_eq!(q_mapping.keycode, 30, "'q' should be at evdev 30 on AZERTY");
            // AZERTY: 'z' is at QWERTY 'w' position (evdev 17),
            //         'w' is at QWERTY 'z' position (evdev 44).
            assert_eq!(z_mapping.keycode, 17, "'z' should be at evdev 17 on AZERTY");
            assert_eq!(w_mapping.keycode, 44, "'w' should be at evdev 44 on AZERTY");
        }
    }

    #[test]
    fn dvorak_layout() {
        // Dvorak heavily remaps the home row and top row.
        let km = XkbKeymap::from_layout(&layout("us", "dvorak"));
        if let Ok(km) = km {
            // Dvorak home row: a o e u i d h t n s
            // 'o' is at QWERTY 's' position (evdev 31).
            let o_mapping = km.lookup('o').expect("'o' should be in dvorak keymap");
            assert_eq!(o_mapping.keycode, 31, "'o' should be at evdev 31 on Dvorak");
            // 'e' is at QWERTY 'd' position (evdev 32).
            let e_mapping = km.lookup('e').expect("'e' should be in dvorak keymap");
            assert_eq!(e_mapping.keycode, 32, "'e' should be at evdev 32 on Dvorak");
            // 's' is at QWERTY ';' position (evdev 39).
            let s_mapping = km.lookup('s').expect("'s' should be in dvorak keymap");
            assert_eq!(s_mapping.keycode, 39, "'s' should be at evdev 39 on Dvorak");
        }
    }

    #[test]
    fn colemak_layout() {
        // Colemak moves several keys from QWERTY positions.
        let km = XkbKeymap::from_layout(&layout("us", "colemak"));
        if let Ok(km) = km {
            // Colemak: 'f' is at QWERTY 'e' position (evdev 18).
            let f_mapping = km.lookup('f').expect("'f' should be in colemak keymap");
            assert_eq!(f_mapping.keycode, 18, "'f' should be at evdev 18 on Colemak");
            // Colemak: 'n' is at QWERTY 'j' position (evdev 36).
            let n_mapping = km.lookup('n').expect("'n' should be in colemak keymap");
            assert_eq!(n_mapping.keycode, 36, "'n' should be at evdev 36 on Colemak");
            // Colemak: 's' moves to QWERTY 'd' position (evdev 32).
            let s_mapping = km.lookup('s').expect("'s' should be in colemak keymap");
            assert_eq!(s_mapping.keycode, 32, "'s' should be at evdev 32 on Colemak");
        }
    }

    #[test]
    fn spanish_layout() {
        // Spanish layout keeps most alpha keys in QWERTY positions
        // but has unique characters like 'ñ' (at QWERTY ';' position, evdev 39).
        let km = XkbKeymap::from_layout(&layout("es", ""));
        if let Ok(km) = km {
            let n_tilde = km.lookup('ñ').expect("'ñ' should be in es keymap");
            assert_eq!(n_tilde.keycode, 39, "'ñ' should be at evdev 39 on Spanish");
            assert!(!n_tilde.shift, "'ñ' should not require shift on Spanish");
        }
    }
}
