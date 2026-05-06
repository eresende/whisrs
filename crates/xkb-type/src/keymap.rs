//! XKB reverse lookup table: char → (Keycode, Modifiers).

use crate::{KeyMapping, KeyTap};
use std::collections::HashMap;
use std::process::Command;

#[derive(Debug, Clone)]
pub struct KeyboardLayout {
    pub layout: String,
    pub variant: String,
}

impl KeyboardLayout {
    pub fn detect() -> Self {
        if let Some(kl) = Self::from_hyprland() {
            return kl;
        }
        if let Some(kl) = Self::from_sway() {
            return kl;
        }
        if let Some(kl) = Self::from_env() {
            return kl;
        }
        Self {
            layout: String::new(),
            variant: String::new(),
        }
    }

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

    fn from_sway() -> Option<Self> {
        let output = Command::new("swaymsg")
            .args(["-t", "get_inputs", "--raw"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let inputs: Vec<serde_json::Value> = serde_json::from_slice(&output.stdout).ok()?;
        let kb = inputs.iter().find(|i| {
            i.get("type").and_then(|t| t.as_str()) == Some("keyboard")
                && i.get("xkb_active_layout_name").is_some()
        })?;
        let active_name = kb.get("xkb_active_layout_name")?.as_str()?.to_string();
        let layout_names = kb.get("xkb_layout_names")?.as_array()?;
        let active_idx = kb
            .get("xkb_active_layout_index")
            .and_then(|i| i.as_u64())
            .unwrap_or(0) as usize;
        let layout = layout_names
            .get(active_idx)
            .and_then(|n| n.as_str())
            .map(|s| s.to_string())
            .unwrap_or(active_name);
        Some(Self {
            layout,
            variant: String::new(),
        })
    }

    fn from_env() -> Option<Self> {
        let layout = std::env::var("XKB_DEFAULT_LAYOUT").ok()?;
        if layout.is_empty() {
            return None;
        }
        let variant = std::env::var("XKB_DEFAULT_VARIANT").unwrap_or_default();
        Some(Self { layout, variant })
    }
}

pub struct XkbKeymap {
    map: HashMap<char, KeyMapping>,
}

impl XkbKeymap {
    pub fn from_layout(detected: &KeyboardLayout) -> anyhow::Result<Self> {
        let context = xkbcommon::xkb::Context::new(xkbcommon::xkb::CONTEXT_NO_FLAGS);
        let keymap = xkbcommon::xkb::Keymap::new_from_names(
            &context,
            "",
            "",
            &detected.layout,
            &detected.variant,
            None,
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
        Ok(Self { map })
    }

    pub fn lookup(&self, ch: char) -> Option<&KeyMapping> {
        self.map.get(&ch)
    }
    pub fn len(&self) -> usize {
        self.map.len()
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

use xkbcommon::xkb::keysyms::{
    KEY_dead_acute, KEY_dead_cedilla, KEY_dead_circumflex, KEY_dead_diaeresis, KEY_dead_grave,
    KEY_dead_tilde,
};

const ACCENTED_VIA_DEAD_KEY: &[(char, char, u32)] = &[
    ('ã', 'a', KEY_dead_tilde),
    ('õ', 'o', KEY_dead_tilde),
    ('ñ', 'n', KEY_dead_tilde),
    ('Ã', 'A', KEY_dead_tilde),
    ('Õ', 'O', KEY_dead_tilde),
    ('Ñ', 'N', KEY_dead_tilde),
    ('á', 'a', KEY_dead_acute),
    ('é', 'e', KEY_dead_acute),
    ('í', 'i', KEY_dead_acute),
    ('ó', 'o', KEY_dead_acute),
    ('ú', 'u', KEY_dead_acute),
    ('ý', 'y', KEY_dead_acute),
    ('Á', 'A', KEY_dead_acute),
    ('É', 'E', KEY_dead_acute),
    ('Í', 'I', KEY_dead_acute),
    ('Ó', 'O', KEY_dead_acute),
    ('Ú', 'U', KEY_dead_acute),
    ('â', 'a', KEY_dead_circumflex),
    ('ê', 'e', KEY_dead_circumflex),
    ('î', 'i', KEY_dead_circumflex),
    ('ô', 'o', KEY_dead_circumflex),
    ('û', 'u', KEY_dead_circumflex),
    ('Â', 'A', KEY_dead_circumflex),
    ('Ê', 'E', KEY_dead_circumflex),
    ('Ô', 'O', KEY_dead_circumflex),
    ('à', 'a', KEY_dead_grave),
    ('è', 'e', KEY_dead_grave),
    ('ì', 'i', KEY_dead_grave),
    ('ò', 'o', KEY_dead_grave),
    ('ù', 'u', KEY_dead_grave),
    ('À', 'A', KEY_dead_grave),
    ('ä', 'a', KEY_dead_diaeresis),
    ('ë', 'e', KEY_dead_diaeresis),
    ('ï', 'i', KEY_dead_diaeresis),
    ('ö', 'o', KEY_dead_diaeresis),
    ('ü', 'u', KEY_dead_diaeresis),
    ('ç', 'c', KEY_dead_cedilla),
    ('Ç', 'C', KEY_dead_cedilla),
];

#[allow(non_upper_case_globals)]
pub(crate) fn build_reverse_map(keymap: &xkbcommon::xkb::Keymap) -> HashMap<char, KeyMapping> {
    let mut map: HashMap<char, KeyMapping> = HashMap::new();
    let mut dead_keys: HashMap<u32, KeyTap> = HashMap::new();
    let min = keymap.min_keycode().raw();
    let max = keymap.max_keycode().raw();
    let num_layouts = keymap.num_layouts();

    for raw_keycode in min..=max {
        let keycode = xkbcommon::xkb::Keycode::new(raw_keycode);
        for layout in 0..num_layouts {
            let num_levels = keymap.num_levels_for_key(keycode, layout);
            for level in 0..num_levels {
                if level > 3 {
                    continue;
                }
                let syms = keymap.key_get_syms_by_level(keycode, layout, level);
                let evdev_keycode: u16 =
                    raw_keycode.saturating_sub(8).try_into().unwrap_or(u16::MAX);
                let shift = level == 1 || level == 3;
                let altgr = level == 2 || level == 3;

                for &sym in syms {
                    let raw = sym.raw();
                    if matches!(
                        raw,
                        KEY_dead_grave
                            | KEY_dead_acute
                            | KEY_dead_circumflex
                            | KEY_dead_tilde
                            | KEY_dead_diaeresis
                            | KEY_dead_cedilla
                    ) {
                        dead_keys.entry(raw).or_insert(KeyTap {
                            keycode: evdev_keycode,
                            shift,
                            altgr,
                        });
                        continue;
                    }
                    let unicode = xkbcommon::xkb::keysym_to_utf32(sym);
                    if unicode == 0 {
                        continue;
                    }
                    if let Some(ch) = char::from_u32(unicode) {
                        map.entry(ch).or_insert(KeyMapping {
                            main: KeyTap {
                                keycode: evdev_keycode,
                                shift,
                                altgr,
                            },
                            follow: None,
                        });
                    }
                }
            }
        }
    }

    // Pass 1: dead_X + Space for literal punctuation
    for (ch, dead_sym) in [
        ('\'', KEY_dead_acute),
        ('"', KEY_dead_diaeresis),
        ('~', KEY_dead_tilde),
        ('`', KEY_dead_grave),
        ('^', KEY_dead_circumflex),
    ] {
        if map.contains_key(&ch) {
            continue;
        }
        if let Some(dk) = dead_keys.get(&dead_sym) {
            map.insert(
                ch,
                KeyMapping {
                    main: *dk,
                    follow: Some(KeyTap {
                        keycode: evdev::Key::KEY_SPACE.code(),
                        shift: false,
                        altgr: false,
                    }),
                },
            );
        }
    }

    // Pass 2: dead_X + base_letter for accented letters
    for &(ch, base, dead_sym) in ACCENTED_VIA_DEAD_KEY {
        if map.contains_key(&ch) {
            continue;
        }
        let Some(dk) = dead_keys.get(&dead_sym) else {
            continue;
        };
        let Some(base_map) = map.get(&base).copied() else {
            continue;
        };
        if base_map.follow.is_some() {
            continue;
        }
        map.insert(
            ch,
            KeyMapping {
                main: *dk,
                follow: Some(base_map.main),
            },
        );
    }

    map
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(name: &str, variant: &str) -> KeyboardLayout {
        KeyboardLayout {
            layout: name.to_string(),
            variant: variant.to_string(),
        }
    }

    fn assert_key(
        km: &XkbKeymap,
        ch: char,
        expected_keycode: u16,
        expected_shift: bool,
        label: &str,
    ) {
        assert_key_full(km, ch, expected_keycode, expected_shift, false, label);
    }

    fn assert_key_full(
        km: &XkbKeymap,
        ch: char,
        expected_keycode: u16,
        expected_shift: bool,
        expected_altgr: bool,
        label: &str,
    ) {
        let mapping = km
            .lookup(ch)
            .unwrap_or_else(|| panic!("'{ch}' should be in {label} keymap"));
        assert_eq!(
            mapping.main.keycode, expected_keycode,
            "'{ch}' keycode mismatch on {label}"
        );
        assert_eq!(
            mapping.main.shift, expected_shift,
            "'{ch}' shift mismatch on {label}"
        );
        assert_eq!(
            mapping.main.altgr, expected_altgr,
            "'{ch}' altgr mismatch on {label}"
        );
    }

    #[test]
    fn german() {
        let km = XkbKeymap::from_layout(&layout("de", "")).unwrap();
        assert_key(&km, 'z', 21, false, "de");
        assert_key(&km, 'y', 44, false, "de");
    }
    #[test]
    fn swiss() {
        let km = XkbKeymap::from_layout(&layout("ch", "")).unwrap();
        assert_key(&km, 'z', 21, false, "ch");
        assert_key(&km, 'y', 44, false, "ch");
    }
    #[test]
    fn czech() {
        let km = XkbKeymap::from_layout(&layout("cz", "")).unwrap();
        assert_key(&km, 'z', 21, false, "cz");
        assert_key(&km, 'y', 44, false, "cz");
    }
    #[test]
    fn slovak() {
        let km = XkbKeymap::from_layout(&layout("sk", "")).unwrap();
        assert_key(&km, 'z', 21, false, "sk");
        assert_key(&km, 'y', 44, false, "sk");
    }
    #[test]
    fn hungarian() {
        let km = XkbKeymap::from_layout(&layout("hu", "")).unwrap();
        assert_key(&km, 'z', 21, false, "hu");
        assert_key(&km, 'y', 44, false, "hu");
    }
    #[test]
    fn french() {
        let km = XkbKeymap::from_layout(&layout("fr", "")).unwrap();
        assert_key(&km, 'a', 16, false, "fr");
        assert_key(&km, 'q', 30, false, "fr");
        assert_key(&km, 'z', 17, false, "fr");
        assert_key(&km, 'w', 44, false, "fr");
    }
    #[test]
    fn belgian() {
        let km = XkbKeymap::from_layout(&layout("be", "")).unwrap();
        assert_key(&km, 'a', 16, false, "be");
        assert_key(&km, 'q', 30, false, "be");
    }
    #[test]
    fn spanish() {
        let km = XkbKeymap::from_layout(&layout("es", "")).unwrap();
        assert_key(&km, 'ñ', 39, false, "es");
    }
    #[test]
    fn portuguese() {
        let km = XkbKeymap::from_layout(&layout("pt", "")).unwrap();
        assert_key(&km, 'a', 30, false, "pt");
    }
    #[test]
    fn italian() {
        let km = XkbKeymap::from_layout(&layout("it", "")).unwrap();
        assert_key(&km, 'a', 30, false, "it");
    }
    #[test]
    fn uk() {
        let km = XkbKeymap::from_layout(&layout("gb", "")).unwrap();
        assert_key(&km, '#', 43, false, "gb");
    }
    #[test]
    fn swedish() {
        let km = XkbKeymap::from_layout(&layout("se", "")).unwrap();
        assert_key(&km, 'ö', 39, false, "se");
        assert_key(&km, 'ä', 40, false, "se");
    }
    #[test]
    fn norwegian() {
        let km = XkbKeymap::from_layout(&layout("no", "")).unwrap();
        assert_key(&km, 'ø', 39, false, "no");
        assert_key(&km, 'æ', 40, false, "no");
    }
    #[test]
    fn danish() {
        let km = XkbKeymap::from_layout(&layout("dk", "")).unwrap();
        assert_key(&km, 'ø', 40, false, "dk");
        assert_key(&km, 'æ', 39, false, "dk");
    }
    #[test]
    fn finnish() {
        let km = XkbKeymap::from_layout(&layout("fi", "")).unwrap();
        assert_key(&km, 'ö', 39, false, "fi");
        assert_key(&km, 'ä', 40, false, "fi");
    }
    #[test]
    fn polish() {
        let km = XkbKeymap::from_layout(&layout("pl", "")).unwrap();
        assert_key(&km, 'a', 30, false, "pl");
        assert_key_full(&km, 'ą', 30, false, true, "pl");
    }
    #[test]
    fn russian() {
        let km = XkbKeymap::from_layout(&layout("ru", "")).unwrap();
        assert_key(&km, 'ф', 30, false, "ru");
        assert_key(&km, 'я', 44, false, "ru");
    }
    #[test]
    fn ukrainian() {
        let km = XkbKeymap::from_layout(&layout("ua", "")).unwrap();
        assert_key(&km, 'ф', 30, false, "ua");
    }
    #[test]
    fn greek() {
        let km = XkbKeymap::from_layout(&layout("gr", "")).unwrap();
        assert_key(&km, 'α', 30, false, "gr");
    }
    #[test]
    fn japanese() {
        let km = XkbKeymap::from_layout(&layout("jp", "")).unwrap();
        assert_key(&km, 'a', 30, false, "jp");
    }
    #[test]
    fn dvorak() {
        let km = XkbKeymap::from_layout(&layout("us", "dvorak")).unwrap();
        assert_key(&km, 'o', 31, false, "dvorak");
    }
    #[test]
    fn colemak() {
        let km = XkbKeymap::from_layout(&layout("us", "colemak")).unwrap();
        assert_key(&km, 's', 32, false, "colemak");
    }

    #[test]
    fn us_intl_typeable_via_uinput() {
        let km = XkbKeymap::from_layout(&layout("us", "intl")).unwrap();
        for ch in [
            '\'', '"', '~', '`', '^', 'ç', 'á', 'é', 'í', 'ó', 'ú', 'ã', 'ñ',
        ] {
            assert!(
                km.lookup(ch).is_some(),
                "'{ch}' must be reachable on us:intl"
            );
        }
    }

    #[test]
    fn build_us_keymap() {
        let km = XkbKeymap::from_layout(&KeyboardLayout {
            layout: "us".into(),
            variant: String::new(),
        });
        if let Ok(km) = km {
            assert!(!km.is_empty());
            assert!(km.lookup('a').is_some());
        }
    }
}
