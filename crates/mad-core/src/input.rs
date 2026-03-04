use anyhow::Result;
#[cfg(not(target_os = "macos"))]
use enigo::{Button, Coordinate, Mouse};
use enigo::{Direction, Enigo, Key, Keyboard, Settings};

use arboard::Clipboard;

pub struct ComputerController {
    enigo: Enigo,
    clipboard: Clipboard,
}

impl ComputerController {
    pub fn new() -> Result<Self> {
        let clipboard =
            Clipboard::new().map_err(|e| anyhow::anyhow!("Failed to init clipboard: {}", e))?;
        let enigo = Enigo::new(&Settings::default())
            .map_err(|e| anyhow::anyhow!("Failed to init enigo: {}", e))?;
        Ok(Self { enigo, clipboard })
    }

    pub async fn paste_text(&mut self, text: &str) -> Result<()> {
        tracing::debug!("Pasting text: '{}'", text);
        self.clipboard
            .set_text(text)
            .map_err(|e| anyhow::anyhow!("Failed to set clipboard: {}", e))?;

        // Wait a bit for clipboard to sync
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        #[cfg(target_os = "macos")]
        {
            use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation};
            use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
            use dispatch::Queue;

            tracing::debug!("Sending Command+V for paste on main thread");

            Queue::main().exec_sync(|| {
                let source = match CGEventSource::new(CGEventSourceStateID::HIDSystemState) {
                    Ok(s) => s,
                    Err(_) => return,
                };

                // Keycodes: Command (55), 'v' (9)
                let cmd_down = CGEvent::new_keyboard_event(source.clone(), 55, true).ok();
                let v_down = CGEvent::new_keyboard_event(source.clone(), 9, true).ok();
                let v_up = CGEvent::new_keyboard_event(source.clone(), 9, false).ok();
                let cmd_up = CGEvent::new_keyboard_event(source, 55, false).ok();

                if let (Some(cmd_down), Some(v_down), Some(v_up), Some(cmd_up)) =
                    (cmd_down, v_down, v_up, cmd_up)
                {
                    // Set flags for all events (MaskCommand = 1 << 20)
                    if let Some(flags) = CGEventFlags::from_bits(1 << 20) {
                        v_down.set_flags(flags);
                        v_up.set_flags(flags);
                    }

                    cmd_down.post(CGEventTapLocation::HID);
                    v_down.post(CGEventTapLocation::HID);
                    v_up.post(CGEventTapLocation::HID);
                    cmd_up.post(CGEventTapLocation::HID);
                }
            });
        }
        #[cfg(not(target_os = "macos"))]
        {
            tracing::debug!("Sending Control+V for paste");
            self.enigo.key(Key::Control, Direction::Press)?;
            self.enigo.key(Key::Unicode('v'), Direction::Click)?;
            self.enigo.key(Key::Control, Direction::Release)?;
        }
        Ok(())
    }

    pub fn move_mouse(&mut self, x: i32, y: i32) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            use core_graphics::event::{CGEvent, CGEventTapLocation, CGEventType, CGMouseButton};
            use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
            use core_graphics::geometry::CGPoint;
            use dispatch::Queue;

            Queue::main().exec_sync(|| {
                if let Ok(source) = CGEventSource::new(CGEventSourceStateID::HIDSystemState) {
                    let point = CGPoint::new(x as f64, y as f64);
                    if let Ok(event) = CGEvent::new_mouse_event(
                        source,
                        CGEventType::MouseMoved,
                        point,
                        CGMouseButton::Left,
                    ) {
                        event.post(CGEventTapLocation::HID);
                    }
                }
            });
            Ok(())
        }

        #[cfg(not(target_os = "macos"))]
        {
            self.enigo.move_mouse(x, y, Coordinate::Abs)?;
            Ok(())
        }
    }

    pub fn click(&mut self) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            use core_graphics::event::{CGEvent, CGEventTapLocation, CGEventType, CGMouseButton};
            use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
            use dispatch::Queue;

            Queue::main().exec_sync(|| {
                if let Ok(source) = CGEventSource::new(CGEventSourceStateID::HIDSystemState) {
                    // Get current mouse position from a new event
                    if let Ok(event) = CGEvent::new(source.clone()) {
                        let current_pos = event.location();

                        if let (Ok(down), Ok(up)) = (
                            CGEvent::new_mouse_event(
                                source.clone(),
                                CGEventType::LeftMouseDown,
                                current_pos,
                                CGMouseButton::Left,
                            ),
                            CGEvent::new_mouse_event(
                                source,
                                CGEventType::LeftMouseUp,
                                current_pos,
                                CGMouseButton::Left,
                            ),
                        ) {
                            down.post(CGEventTapLocation::HID);
                            up.post(CGEventTapLocation::HID);
                        }
                    }
                }
            });
            Ok(())
        }

        #[cfg(not(target_os = "macos"))]
        {
            self.enigo.button(Button::Left, Direction::Click)?;
            Ok(())
        }
    }

    pub async fn type_text(&mut self, text: &str) -> Result<()> {
        // Use paste for text input to avoid IME interference and speed up input
        // This is much more reliable than typing char-by-char, especially for non-ASCII text
        self.paste_text(text).await
    }

    pub fn press_enter(&mut self) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            use core_graphics::event::{CGEvent, CGEventTapLocation};
            use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
            use dispatch::Queue;

            Queue::main().exec_sync(|| {
                if let Ok(source) = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
                    && let (Ok(event_down), Ok(event_up)) = (
                        CGEvent::new_keyboard_event(source.clone(), 36, true),
                        CGEvent::new_keyboard_event(source, 36, false),
                    ) {
                        event_down.post(CGEventTapLocation::HID);
                        event_up.post(CGEventTapLocation::HID);
                    }
            });
            Ok(())
        }
        #[cfg(not(target_os = "macos"))]
        {
            self.enigo.key(Key::Return, Direction::Click)?;
            Ok(())
        }
    }

    pub async fn key_sequence(&mut self, keys: Vec<String>) -> Result<()> {
        tracing::debug!("Executing key sequence: {:?}", keys);
        let mut held_keys = Vec::new();
        #[cfg(target_os = "macos")]
        let mut current_flags = core_graphics::event::CGEventFlags::empty();

        for (i, key_str) in keys.iter().enumerate() {
            let is_last = i == keys.len() - 1;
            let key_parsed = parse_key(key_str);

            if let Some(key) = key_parsed {
                if is_modifier(key) {
                    #[cfg(target_os = "macos")]
                    {
                        let flag_bits = match key {
                            Key::Meta => 1 << 20,
                            Key::Control => 1 << 18,
                            Key::Shift => 1 << 17,
                            Key::Alt => 1 << 19,
                            _ => 0,
                        };
                        if let Some(flag) = core_graphics::event::CGEventFlags::from_bits(flag_bits)
                        {
                            current_flags |= flag;
                        }
                    }

                    if is_last {
                        // Just click the modifier if it's the last one (unusual but possible)
                        tracing::debug!("Clicking modifier: {:?}", key);
                        #[cfg(target_os = "macos")]
                        {
                            let keycode = match key {
                                Key::Meta => 55,
                                Key::Control => 59,
                                Key::Shift => 56,
                                Key::Alt => 58,
                                _ => 0,
                            };
                            use dispatch::Queue;
                            Queue::main().exec_sync(|| {
                                if keycode != 0 {
                                    use core_graphics::event::{CGEvent, CGEventTapLocation};
                                    use core_graphics::event_source::{
                                        CGEventSource, CGEventSourceStateID,
                                    };
                                    if let Ok(source) =
                                        CGEventSource::new(CGEventSourceStateID::HIDSystemState)
                                        && let (Ok(event_down), Ok(event_up)) = (
                                            CGEvent::new_keyboard_event(
                                                source.clone(),
                                                keycode,
                                                true,
                                            ),
                                            CGEvent::new_keyboard_event(source, keycode, false),
                                        ) {
                                            event_down.set_flags(current_flags);
                                            event_up.set_flags(current_flags);
                                            event_down.post(CGEventTapLocation::HID);
                                            event_up.post(CGEventTapLocation::HID);
                                        }
                                } else {
                                    let _ = self.enigo.key(key, Direction::Click);
                                }
                            });
                        }
                        #[cfg(not(target_os = "macos"))]
                        self.enigo.key(key, Direction::Click)?;
                    } else {
                        // Hold modifier
                        tracing::debug!("Holding modifier: {:?}", key);
                        #[cfg(target_os = "macos")]
                        {
                            let keycode = match key {
                                Key::Meta => 55,
                                Key::Control => 59,
                                Key::Shift => 56,
                                Key::Alt => 58,
                                _ => 0,
                            };
                            use dispatch::Queue;
                            Queue::main().exec_sync(|| {
                                if keycode != 0 {
                                    use core_graphics::event::{CGEvent, CGEventTapLocation};
                                    use core_graphics::event_source::{
                                        CGEventSource, CGEventSourceStateID,
                                    };
                                    if let Ok(source) =
                                        CGEventSource::new(CGEventSourceStateID::HIDSystemState)
                                        && let Ok(event) =
                                            CGEvent::new_keyboard_event(source, keycode, true)
                                        {
                                            event.set_flags(current_flags);
                                            event.post(CGEventTapLocation::HID);
                                        }
                                } else {
                                    let _ = self.enigo.key(key, Direction::Press);
                                }
                            });
                        }
                        #[cfg(not(target_os = "macos"))]
                        self.enigo.key(key, Direction::Press)?;

                        held_keys.push(key);
                    }
                } else {
                    // Click non-modifier key
                    tracing::debug!("Clicking key: {:?}", key);

                    #[cfg(target_os = "macos")]
                    {
                        let keycode = match key {
                            Key::Return => 36,
                            Key::Space => 49,
                            Key::Backspace => 51,
                            Key::Delete => 117,
                            Key::Tab => 48,
                            Key::Escape => 53,
                            Key::UpArrow => 126,
                            Key::DownArrow => 125,
                            Key::LeftArrow => 123,
                            Key::RightArrow => 124,
                            Key::Home => 115,
                            Key::End => 119,
                            Key::PageUp => 116,
                            Key::PageDown => 121,
                            Key::F1 => 122,
                            Key::F2 => 120,
                            Key::F3 => 99,
                            Key::F4 => 118,
                            Key::F5 => 96,
                            Key::F6 => 97,
                            Key::F7 => 98,
                            Key::F8 => 100,
                            Key::F9 => 101,
                            Key::F10 => 109,
                            Key::F11 => 103,
                            Key::F12 => 111,
                            _ => 0,
                        };
                        use dispatch::Queue;
                        Queue::main().exec_sync(|| {
                            if keycode != 0 {
                                use core_graphics::event::{CGEvent, CGEventTapLocation};
                                use core_graphics::event_source::{
                                    CGEventSource, CGEventSourceStateID,
                                };
                                if let Ok(source) =
                                    CGEventSource::new(CGEventSourceStateID::HIDSystemState)
                                    && let (Ok(event_down), Ok(event_up)) = (
                                        CGEvent::new_keyboard_event(source.clone(), keycode, true),
                                        CGEvent::new_keyboard_event(source, keycode, false),
                                    ) {
                                        event_down.set_flags(current_flags);
                                        event_up.set_flags(current_flags);
                                        event_down.post(CGEventTapLocation::HID);
                                        event_up.post(CGEventTapLocation::HID);
                                    }
                            } else {
                                let _ = self.enigo.key(key, Direction::Click);
                            }
                        });
                    }
                    #[cfg(not(target_os = "macos"))]
                    self.enigo.key(key, Direction::Click)?;

                    #[cfg(target_os = "macos")]
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            } else {
                // Not a special key, treat as character(s)
                if key_str.len() == 1 {
                    let c = key_str.chars().next().unwrap();
                    let is_holding_modifiers = !held_keys.is_empty();

                    if is_holding_modifiers {
                        // When holding modifiers, always use key() instead of text() to ensure shortcuts work
                        // For letters, we must use lowercase base key to avoid double-shift issues
                        let key_char = if c.is_ascii_alphabetic() {
                            c.to_ascii_lowercase()
                        } else {
                            c
                        };
                        tracing::debug!(
                            "Clicking char key with modifiers: '{}' (from '{}')",
                            key_char,
                            c
                        );

                        #[cfg(target_os = "macos")]
                        {
                            // Map some common keys to keycodes to avoid trace trap with Key::Unicode
                            let keycode = match key_char {
                                'a' => 0,
                                's' => 1,
                                'd' => 2,
                                'f' => 3,
                                'h' => 4,
                                'g' => 5,
                                'z' => 6,
                                'x' => 7,
                                'c' => 8,
                                'v' => 9,
                                'b' => 11,
                                'q' => 12,
                                'w' => 13,
                                'e' => 14,
                                'r' => 15,
                                'y' => 16,
                                't' => 17,
                                '1' => 18,
                                '2' => 19,
                                '3' => 20,
                                '4' => 21,
                                '6' => 22,
                                '5' => 23,
                                '=' => 24,
                                '9' => 25,
                                '7' => 26,
                                '-' => 27,
                                '8' => 28,
                                '0' => 29,
                                ']' => 30,
                                'o' => 31,
                                'u' => 32,
                                '[' => 33,
                                'i' => 34,
                                'p' => 35,
                                'l' => 37,
                                'j' => 38,
                                '\'' => 39,
                                'k' => 40,
                                ';' => 41,
                                '\\' => 42,
                                ',' => 43,
                                '/' => 44,
                                'n' => 45,
                                'm' => 46,
                                '.' => 47,
                                '`' => 50,
                                _ => 0,
                            };
                            use dispatch::Queue;
                            Queue::main().exec_sync(|| {
                                if keycode != 0 || key_char == 'a' {
                                    use core_graphics::event::{CGEvent, CGEventTapLocation};
                                    use core_graphics::event_source::{
                                        CGEventSource, CGEventSourceStateID,
                                    };
                                    if let Ok(source) =
                                        CGEventSource::new(CGEventSourceStateID::HIDSystemState)
                                        && let (Ok(event_down), Ok(event_up)) = (
                                            CGEvent::new_keyboard_event(
                                                source.clone(),
                                                keycode,
                                                true,
                                            ),
                                            CGEvent::new_keyboard_event(source, keycode, false),
                                        ) {
                                            event_down.set_flags(current_flags);
                                            event_up.set_flags(current_flags);
                                            event_down.post(CGEventTapLocation::HID);
                                            event_up.post(CGEventTapLocation::HID);
                                        }
                                } else {
                                    let _ =
                                        self.enigo.key(Key::Unicode(key_char), Direction::Click);
                                }
                            });
                        }
                        #[cfg(not(target_os = "macos"))]
                        self.enigo.key(Key::Unicode(key_char), Direction::Click)?;
                    } else {
                        // For single char, use Key::Unicode instead of text()
                        // text() can be unreliable on macOS with certain layouts or permissions
                        // Key::Unicode is closer to a physical key press
                        tracing::debug!("Clicking char key: '{}'", c);
                        #[cfg(target_os = "macos")]
                        {
                            let keycode = match c {
                                'a' => 0,
                                's' => 1,
                                'd' => 2,
                                'f' => 3,
                                'h' => 4,
                                'g' => 5,
                                'z' => 6,
                                'x' => 7,
                                'c' => 8,
                                'v' => 9,
                                'b' => 11,
                                'q' => 12,
                                'w' => 13,
                                'e' => 14,
                                'r' => 15,
                                'y' => 16,
                                't' => 17,
                                '1' => 18,
                                '2' => 19,
                                '3' => 20,
                                '4' => 21,
                                '6' => 22,
                                '5' => 23,
                                '=' => 24,
                                '9' => 25,
                                '7' => 26,
                                '-' => 27,
                                '8' => 28,
                                '0' => 29,
                                ']' => 30,
                                'o' => 31,
                                'u' => 32,
                                '[' => 33,
                                'i' => 34,
                                'p' => 35,
                                'l' => 37,
                                'j' => 38,
                                '\'' => 39,
                                'k' => 40,
                                ';' => 41,
                                '\\' => 42,
                                ',' => 43,
                                '/' => 44,
                                'n' => 45,
                                'm' => 46,
                                '.' => 47,
                                '`' => 50,
                                _ => 0,
                            };
                            use dispatch::Queue;
                            Queue::main().exec_sync(|| {
                                if keycode != 0 || c == 'a' {
                                    use core_graphics::event::{CGEvent, CGEventTapLocation};
                                    use core_graphics::event_source::{
                                        CGEventSource, CGEventSourceStateID,
                                    };
                                    if let Ok(source) =
                                        CGEventSource::new(CGEventSourceStateID::HIDSystemState)
                                        && let (Ok(event_down), Ok(event_up)) = (
                                            CGEvent::new_keyboard_event(
                                                source.clone(),
                                                keycode,
                                                true,
                                            ),
                                            CGEvent::new_keyboard_event(source, keycode, false),
                                        ) {
                                            event_down.set_flags(current_flags);
                                            event_up.set_flags(current_flags);
                                            event_down.post(CGEventTapLocation::HID);
                                            event_up.post(CGEventTapLocation::HID);
                                        }
                                } else {
                                    let _ = self.enigo.key(Key::Unicode(c), Direction::Click);
                                }
                            });
                        }
                        #[cfg(not(target_os = "macos"))]
                        self.enigo.key(Key::Unicode(c), Direction::Click)?;
                    }
                } else {
                    // For strings, type them out (modifiers might not apply as expected to text())
                    tracing::debug!("Typing string text: '{}'", key_str);
                    #[cfg(target_os = "macos")]
                    self.type_text(key_str).await?;
                    #[cfg(not(target_os = "macos"))]
                    self.enigo.text(key_str)?;
                }
                #[cfg(target_os = "macos")]
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }

        // Release modifiers in reverse order
        for key in held_keys.into_iter().rev() {
            tracing::debug!("Releasing modifier: {:?}", key);
            #[cfg(target_os = "macos")]
            {
                let keycode = match key {
                    Key::Meta => 55,
                    Key::Control => 59,
                    Key::Shift => 56,
                    Key::Alt => 58,
                    _ => 0,
                };
                use dispatch::Queue;
                Queue::main().exec_sync(|| {
                    if keycode != 0 {
                        use core_graphics::event::{CGEvent, CGEventTapLocation};
                        use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
                        if let Ok(source) = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
                            && let Ok(event) = CGEvent::new_keyboard_event(source, keycode, false) {
                                event.post(CGEventTapLocation::HID);
                            }
                    } else {
                        let _ = self.enigo.key(key, Direction::Release);
                    }
                });
            }
            #[cfg(not(target_os = "macos"))]
            self.enigo.key(key, Direction::Release)?;

            #[cfg(target_os = "macos")]
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }

        Ok(())
    }
}

fn parse_key(k: &str) -> Option<Key> {
    match k.to_lowercase().as_str() {
        "command" | "cmd" | "meta" | "super" => Some(Key::Meta),
        "control" | "ctrl" => Some(Key::Control),
        "shift" => Some(Key::Shift),
        "alt" | "option" => Some(Key::Alt),
        "enter" | "return" => Some(Key::Return),
        "space" => Some(Key::Space),
        "backspace" => Some(Key::Backspace),
        "delete" | "del" => Some(Key::Delete),
        "tab" => Some(Key::Tab),
        "escape" | "esc" => Some(Key::Escape),
        "up" | "arrowup" => Some(Key::UpArrow),
        "down" | "arrowdown" => Some(Key::DownArrow),
        "left" | "arrowleft" => Some(Key::LeftArrow),
        "right" | "arrowright" => Some(Key::RightArrow),
        "home" => Some(Key::Home),
        "end" => Some(Key::End),
        "pageup" | "pgup" => Some(Key::PageUp),
        "pagedown" | "pgdn" => Some(Key::PageDown),
        "f1" => Some(Key::F1),
        "f2" => Some(Key::F2),
        "f3" => Some(Key::F3),
        "f4" => Some(Key::F4),
        "f5" => Some(Key::F5),
        "f6" => Some(Key::F6),
        "f7" => Some(Key::F7),
        "f8" => Some(Key::F8),
        "f9" => Some(Key::F9),
        "f10" => Some(Key::F10),
        "f11" => Some(Key::F11),
        "f12" => Some(Key::F12),
        _ => None,
    }
}

fn is_modifier(key: Key) -> bool {
    matches!(key, Key::Meta | Key::Control | Key::Shift | Key::Alt)
}
