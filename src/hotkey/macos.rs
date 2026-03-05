use crossbeam_channel::{bounded, Receiver};
use rdev::{listen, Event, EventType, Key};

use super::{HotkeyConfig, HotkeyError, HotkeyEvent, HotkeyHandle, HotkeyKey, HotkeyModifier};

pub fn spawn(config: HotkeyConfig) -> Result<(HotkeyHandle, Receiver<HotkeyEvent>), HotkeyError> {
    let (event_tx, event_rx) = bounded::<HotkeyEvent>(32);
    let (stop_tx, _stop_rx)  = bounded::<()>(1);

    let ptt_key  = to_rdev_key(&config.push_to_talk.key);
    let ptt_mods = config.push_to_talk.modifiers.clone();

    std::thread::spawn(move || {
        let mut ctrl_held  = false;
        let mut alt_held   = false;
        let mut shift_held = false;
        let mut meta_held  = false;
        let mut ptt_held   = false;

        let callback = move |event: Event| {
            match &event.event_type {
                EventType::KeyPress(k)   => update_mods(k, &mut ctrl_held, &mut alt_held, &mut shift_held, &mut meta_held, true),
                EventType::KeyRelease(k) => update_mods(k, &mut ctrl_held, &mut alt_held, &mut shift_held, &mut meta_held, false),
                _ => {}
            }

            let mods_ok = ptt_mods.iter().all(|m| match m {
                HotkeyModifier::Ctrl  => ctrl_held,
                HotkeyModifier::Alt   => alt_held,
                HotkeyModifier::Shift => shift_held,
                HotkeyModifier::Meta  => meta_held,
            });

            match &event.event_type {
                EventType::KeyPress(k) if *k == ptt_key && mods_ok && !ptt_held => {
                    ptt_held = true;
                    let tx = event_tx.clone();

                    // Важно: Вызываем хелпер в отдельном микро-потоке!
                    // Глобальный хук rdev требует мгновенного возврата управления,
                    // поэтому мы не можем блокировать его вызовом Command::new.
                    std::thread::spawn(move || {
                        if let Some((pid, name)) = frontmost_app() {
                            eprintln!("[hotkey] нажат, цель: {} (pid={})", name, pid);
                            let _ = tx.try_send(HotkeyEvent::PushToTalkPressed { pid });
                        } else {
                            eprintln!("[hotkey] ⚠️ Не удалось определить активное окно");
                        }
                    });
                }
                EventType::KeyRelease(k) if *k == ptt_key && ptt_held => {
                    ptt_held = false;
                    let _ = event_tx.try_send(HotkeyEvent::PushToTalkReleased);
                }
                _ => {}
            }
        };

        if let Err(e) = listen(callback) {
            eprintln!("rdev: {e:?}");
        }
    });

    Ok((HotkeyHandle::new(stop_tx), event_rx))
}

fn frontmost_app() -> Option<(u32, String)> {
    let mut ax_helper_path = std::env::current_exe().unwrap_or_default();
    ax_helper_path.pop();
    ax_helper_path.push("ax-helper-bin");

    let output = std::process::Command::new(&ax_helper_path)
        .arg("frontmost")
        .output()
        .ok()?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let parts: Vec<&str> = stdout.splitn(2, '|').collect();
        if parts.len() == 2 {
            let pid = parts[0].parse().unwrap_or(0);
            let name = parts[1].to_string();
            return Some((pid, name));
        }
    }
    None
}

fn to_rdev_key(key: &HotkeyKey) -> Key {
    match key {
        HotkeyKey::AltRight  => Key::AltGr,
        HotkeyKey::CapsLock  => Key::CapsLock,
        HotkeyKey::F1  => Key::F1,  HotkeyKey::F2  => Key::F2,
        HotkeyKey::F3  => Key::F3,  HotkeyKey::F4  => Key::F4,
        HotkeyKey::F5  => Key::F5,  HotkeyKey::F6  => Key::F6,
        HotkeyKey::F7  => Key::F7,  HotkeyKey::F8  => Key::F8,
        HotkeyKey::F9  => Key::F9,  HotkeyKey::F10 => Key::F10,
        HotkeyKey::F11 => Key::F11, HotkeyKey::F12 => Key::F12,
        HotkeyKey::Char(c) => char_to_rdev(*c),
    }
}

fn update_mods(key: &Key, ctrl: &mut bool, alt: &mut bool, shift: &mut bool, meta: &mut bool, pressed: bool) {
    match key {
        Key::ControlLeft | Key::ControlRight => *ctrl  = pressed,
        Key::Alt                             => *alt   = pressed,
        Key::ShiftLeft | Key::ShiftRight     => *shift = pressed,
        Key::MetaLeft  | Key::MetaRight      => *meta  = pressed,
        _ => {}
    }
}

fn char_to_rdev(c: char) -> Key {
    match c.to_ascii_lowercase() {
        'a' => Key::KeyA, 'b' => Key::KeyB, 'c' => Key::KeyC, 'd' => Key::KeyD,
        'e' => Key::KeyE, 'f' => Key::KeyF, 'g' => Key::KeyG, 'h' => Key::KeyH,
        'i' => Key::KeyI, 'j' => Key::KeyJ, 'k' => Key::KeyK, 'l' => Key::KeyL,
        'm' => Key::KeyM, 'n' => Key::KeyN, 'o' => Key::KeyO, 'p' => Key::KeyP,
        'q' => Key::KeyQ, 'r' => Key::KeyR, 's' => Key::KeyS, 't' => Key::KeyT,
        'u' => Key::KeyU, 'v' => Key::KeyV, 'w' => Key::KeyW, 'x' => Key::KeyX,
        'y' => Key::KeyY, 'z' => Key::KeyZ,
        '0' => Key::Num0, '1' => Key::Num1, '2' => Key::Num2, '3' => Key::Num3,
        '4' => Key::Num4, '5' => Key::Num5, '6' => Key::Num6, '7' => Key::Num7,
        '8' => Key::Num8, '9' => Key::Num9,
        _   => Key::Unknown(0),
    }
}