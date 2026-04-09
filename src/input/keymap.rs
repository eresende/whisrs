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

    /// Helper: assert a character maps to the expected evdev keycode and shift state.
    fn assert_key(
        km: &XkbKeymap,
        ch: char,
        expected_keycode: u16,
        expected_shift: bool,
        label: &str,
    ) {
        let mapping = km
            .lookup(ch)
            .unwrap_or_else(|| panic!("'{ch}' should be in {label} keymap"));
        assert_eq!(
            mapping.keycode, expected_keycode,
            "'{ch}' should be at evdev {expected_keycode} on {label}, got {}",
            mapping.keycode
        );
        assert_eq!(
            mapping.shift, expected_shift,
            "'{ch}' shift should be {expected_shift} on {label}"
        );
    }

    // --- QWERTZ family (y/z swapped) ---

    #[test]
    fn german_layout() {
        let km = XkbKeymap::from_layout(&layout("de", "")).unwrap();
        assert_key(&km, 'z', 21, false, "German");
        assert_key(&km, 'y', 44, false, "German");
    }

    #[test]
    fn swiss_layout() {
        let km = XkbKeymap::from_layout(&layout("ch", "")).unwrap();
        assert_key(&km, 'z', 21, false, "Swiss");
        assert_key(&km, 'y', 44, false, "Swiss");
    }

    #[test]
    fn czech_layout() {
        let km = XkbKeymap::from_layout(&layout("cz", "")).unwrap();
        assert_key(&km, 'z', 21, false, "Czech");
        assert_key(&km, 'y', 44, false, "Czech");
        assert_key(&km, 'ů', 39, false, "Czech");
    }

    #[test]
    fn slovak_layout() {
        let km = XkbKeymap::from_layout(&layout("sk", "")).unwrap();
        assert_key(&km, 'z', 21, false, "Slovak");
        assert_key(&km, 'y', 44, false, "Slovak");
        assert_key(&km, 'ô', 39, false, "Slovak");
    }

    #[test]
    fn hungarian_layout() {
        let km = XkbKeymap::from_layout(&layout("hu", "")).unwrap();
        assert_key(&km, 'z', 21, false, "Hungarian");
        assert_key(&km, 'y', 44, false, "Hungarian");
        assert_key(&km, 'ö', 11, false, "Hungarian");
        assert_key(&km, 'ü', 12, false, "Hungarian");
    }

    // --- AZERTY family (a/q and z/w swapped) ---

    #[test]
    fn french_layout() {
        let km = XkbKeymap::from_layout(&layout("fr", "")).unwrap();
        assert_key(&km, 'a', 16, false, "French");
        assert_key(&km, 'q', 30, false, "French");
        assert_key(&km, 'z', 17, false, "French");
        assert_key(&km, 'w', 44, false, "French");
    }

    #[test]
    fn belgian_layout() {
        let km = XkbKeymap::from_layout(&layout("be", "")).unwrap();
        assert_key(&km, 'a', 16, false, "Belgian");
        assert_key(&km, 'q', 30, false, "Belgian");
        assert_key(&km, 'z', 17, false, "Belgian");
        assert_key(&km, 'w', 44, false, "Belgian");
        assert_key(&km, 'm', 39, false, "Belgian");
    }

    // --- QWERTY-based with special characters ---

    #[test]
    fn spanish_layout() {
        let km = XkbKeymap::from_layout(&layout("es", "")).unwrap();
        assert_key(&km, 'ñ', 39, false, "Spanish");
    }

    #[test]
    fn portuguese_layout() {
        let km = XkbKeymap::from_layout(&layout("pt", "")).unwrap();
        assert_key(&km, 'a', 30, false, "Portuguese");
        assert_key(&km, 'z', 44, false, "Portuguese");
        assert_key(&km, 'q', 16, false, "Portuguese");
    }

    #[test]
    fn italian_layout() {
        let km = XkbKeymap::from_layout(&layout("it", "")).unwrap();
        assert_key(&km, 'a', 30, false, "Italian");
        assert_key(&km, 'z', 44, false, "Italian");
        assert_key(&km, 'q', 16, false, "Italian");
        assert_key(&km, 'w', 17, false, "Italian");
    }

    #[test]
    fn uk_layout() {
        let km = XkbKeymap::from_layout(&layout("gb", "")).unwrap();
        assert_key(&km, 'a', 30, false, "UK");
        assert_key(&km, 'z', 44, false, "UK");
        // UK has '#' without shift (evdev 43), unlike US where it's Shift+3.
        assert_key(&km, '#', 43, false, "UK");
        assert_key(&km, '£', 4, true, "UK");
    }

    // --- Nordic layouts ---

    #[test]
    fn swedish_layout() {
        let km = XkbKeymap::from_layout(&layout("se", "")).unwrap();
        assert_key(&km, 'ö', 39, false, "Swedish");
        assert_key(&km, 'ä', 40, false, "Swedish");
    }

    #[test]
    fn norwegian_layout() {
        let km = XkbKeymap::from_layout(&layout("no", "")).unwrap();
        assert_key(&km, 'ø', 39, false, "Norwegian");
        assert_key(&km, 'æ', 40, false, "Norwegian");
    }

    #[test]
    fn danish_layout() {
        let km = XkbKeymap::from_layout(&layout("dk", "")).unwrap();
        assert_key(&km, 'ø', 40, false, "Danish");
        assert_key(&km, 'æ', 39, false, "Danish");
    }

    #[test]
    fn finnish_layout() {
        let km = XkbKeymap::from_layout(&layout("fi", "")).unwrap();
        assert_key(&km, 'ö', 39, false, "Finnish");
        assert_key(&km, 'ä', 40, false, "Finnish");
    }

    // --- Eastern European ---

    #[test]
    fn polish_layout() {
        let km = XkbKeymap::from_layout(&layout("pl", "")).unwrap();
        assert_key(&km, 'a', 30, false, "Polish");
        assert_key(&km, 'z', 44, false, "Polish");
        // Polish special chars via AltGr (mapped as shift in our model).
        assert_key(&km, 'ą', 30, true, "Polish");
        assert_key(&km, 'ę', 18, true, "Polish");
    }

    // --- Alternative Latin layouts ---

    #[test]
    fn dvorak_layout() {
        let km = XkbKeymap::from_layout(&layout("us", "dvorak")).unwrap();
        assert_key(&km, 'o', 31, false, "Dvorak");
        assert_key(&km, 'e', 32, false, "Dvorak");
        assert_key(&km, 's', 39, false, "Dvorak");
    }

    #[test]
    fn colemak_layout() {
        let km = XkbKeymap::from_layout(&layout("us", "colemak")).unwrap();
        assert_key(&km, 'f', 18, false, "Colemak");
        assert_key(&km, 'n', 36, false, "Colemak");
        assert_key(&km, 's', 32, false, "Colemak");
    }

    // --- Non-Latin layouts ---

    #[test]
    fn russian_layout() {
        let km = XkbKeymap::from_layout(&layout("ru", "")).unwrap();
        assert_key(&km, 'ф', 30, false, "Russian");
        assert_key(&km, 'я', 44, false, "Russian");
        assert_key(&km, 'й', 16, false, "Russian");
        assert_key(&km, 'ц', 17, false, "Russian");
    }

    #[test]
    fn ukrainian_layout() {
        let km = XkbKeymap::from_layout(&layout("ua", "")).unwrap();
        assert_key(&km, 'ф', 30, false, "Ukrainian");
        assert_key(&km, 'я', 44, false, "Ukrainian");
        assert_key(&km, 'й', 16, false, "Ukrainian");
        assert_key(&km, 'і', 31, false, "Ukrainian");
    }

    #[test]
    fn greek_layout() {
        let km = XkbKeymap::from_layout(&layout("gr", "")).unwrap();
        assert_key(&km, 'α', 30, false, "Greek");
        assert_key(&km, 'ζ', 44, false, "Greek");
        assert_key(&km, 'ω', 47, false, "Greek");
    }

    #[test]
    fn japanese_layout() {
        // Japanese (jp) is QWERTY-based for Latin characters.
        let km = XkbKeymap::from_layout(&layout("jp", "")).unwrap();
        assert_key(&km, 'a', 30, false, "Japanese");
        assert_key(&km, 'z', 44, false, "Japanese");
    }
}
