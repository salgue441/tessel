//! Parsing of hotkey strings like `alt+shift+h` into Win32 modifier flags and
//! virtual-key codes.

pub const MOD_ALT: u32 = 0x1;
pub const MOD_CONTROL: u32 = 0x2;
pub const MOD_SHIFT: u32 = 0x4;
pub const MOD_WIN: u32 = 0x8;

/// Returns `(modifiers, virtual_key)`.
pub fn parse(s: &str) -> Result<(u32, u32), String> {
    let mut mods = 0;
    let mut vk = None;
    for part in s.split('+').map(|p| p.trim().to_ascii_lowercase()) {
        match part.as_str() {
            "alt" => mods |= MOD_ALT,
            "ctrl" | "control" => mods |= MOD_CONTROL,
            "shift" => mods |= MOD_SHIFT,
            "win" | "super" | "meta" => mods |= MOD_WIN,
            key => {
                if vk.is_some() {
                    return Err(format!("`{s}` has more than one non-modifier key"));
                }
                vk = Some(virtual_key(key).ok_or_else(|| format!("unknown key `{key}` in `{s}`"))?);
            }
        }
    }
    let vk = vk.ok_or_else(|| format!("`{s}` has no key, only modifiers"))?;
    if mods == 0 {
        return Err(format!("`{s}` needs at least one modifier (alt, ctrl, shift, win)"));
    }
    Ok((mods, vk))
}

fn virtual_key(key: &str) -> Option<u32> {
    let mut chars = key.chars();
    if let (Some(c), None) = (chars.next(), chars.next())
        && c.is_ascii_alphanumeric()
    {
        return Some(c.to_ascii_uppercase() as u32);
    }
    if let Some(n) = key.strip_prefix('f').and_then(|n| n.parse::<u32>().ok())
        && (1..=24).contains(&n)
    {
        return Some(0x70 + n - 1);
    }
    Some(match key {
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        "enter" | "return" => 0x0D,
        "space" => 0x20,
        "tab" => 0x09,
        "escape" | "esc" => 0x1B,
        "backspace" => 0x08,
        "delete" => 0x2E,
        "insert" => 0x2D,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" => 0x21,
        "pagedown" => 0x22,
        "minus" | "-" => 0xBD,
        "equal" | "=" => 0xBB,
        "comma" | "," => 0xBC,
        "period" | "." => 0xBE,
        "slash" | "/" => 0xBF,
        "semicolon" | ";" => 0xBA,
        "quote" | "'" => 0xDE,
        "bracketleft" | "[" => 0xDB,
        "bracketright" | "]" => 0xDD,
        "backslash" | "\\" => 0xDC,
        "grave" | "`" => 0xC0,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hotkeys() {
        assert_eq!(parse("alt+h"), Ok((MOD_ALT, 'H' as u32)));
        assert_eq!(parse("Alt+Shift+1"), Ok((MOD_ALT | MOD_SHIFT, '1' as u32)));
        assert_eq!(parse("win+ctrl+f5"), Ok((MOD_WIN | MOD_CONTROL, 0x74)));
        assert_eq!(parse("alt+enter"), Ok((MOD_ALT, 0x0D)));
        assert_eq!(parse("alt+minus"), Ok((MOD_ALT, 0xBD)));
    }

    #[test]
    fn rejects_bad_hotkeys() {
        assert!(parse("h").is_err());
        assert!(parse("alt").is_err());
        assert!(parse("alt+h+j").is_err());
        assert!(parse("alt+nope").is_err());
        assert!(parse("alt+f25").is_err());
    }
}
