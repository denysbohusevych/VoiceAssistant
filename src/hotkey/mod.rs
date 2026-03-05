#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(not(target_os = "macos"))]
pub mod rdev_impl;

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct Hotkey {
    pub modifiers: Vec<HotkeyModifier>,
    pub key: HotkeyKey,
}

impl Hotkey {
    pub fn new(key: HotkeyKey, modifiers: Vec<HotkeyModifier>) -> Self {
        Self { modifiers, key }
    }
    pub fn single(key: HotkeyKey) -> Self {
        Self { modifiers: vec![], key }
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for m in &self.modifiers { write!(f, "{m}+")?; }
        write!(f, "{}", self.key)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum HotkeyModifier { Ctrl, Alt, Shift, Meta }

impl fmt::Display for HotkeyModifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ctrl  => write!(f, "Ctrl"),
            Self::Alt   => write!(f, "Alt"),
            Self::Shift => write!(f, "Shift"),
            Self::Meta  => write!(f, "Cmd"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum HotkeyKey {
    AltRight,
    CapsLock,
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12,
    Char(char),
}

impl fmt::Display for HotkeyKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AltRight => write!(f, "RightOption"),
            Self::CapsLock => write!(f, "CapsLock"),
            Self::F1  => write!(f, "F1"),  Self::F2  => write!(f, "F2"),
            Self::F3  => write!(f, "F3"),  Self::F4  => write!(f, "F4"),
            Self::F5  => write!(f, "F5"),  Self::F6  => write!(f, "F6"),
            Self::F7  => write!(f, "F7"),  Self::F8  => write!(f, "F8"),
            Self::F9  => write!(f, "F9"),  Self::F10 => write!(f, "F10"),
            Self::F11 => write!(f, "F11"), Self::F12 => write!(f, "F12"),
            Self::Char(c) => write!(f, "{c}"),
        }
    }
}

pub struct HotkeyConfig {
    pub push_to_talk: Hotkey,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self { push_to_talk: Hotkey::single(HotkeyKey::AltRight) }
    }
}

#[derive(Debug, Clone)]
pub enum HotkeyEvent {
    /// Клавиша нажата. pid — процесс который был активен В МОМЕНТ нажатия.
    PushToTalkPressed { pid: u32 },
    PushToTalkReleased,
}

#[derive(Debug)]
pub enum HotkeyError {
    PermissionDenied,
    InitError(String),
}

impl fmt::Display for HotkeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PermissionDenied => write!(
                f,
                "Нет доступа к Accessibility API.\n\
                 System Settings → Privacy & Security → Accessibility → добавь Terminal.app"
            ),
            Self::InitError(s) => write!(f, "Ошибка инициализации хоткеев: {s}"),
        }
    }
}

impl std::error::Error for HotkeyError {}

pub struct HotkeyHandle { stop_tx: crossbeam_channel::Sender<()> }

impl HotkeyHandle {
    pub fn new(stop_tx: crossbeam_channel::Sender<()>) -> Self { Self { stop_tx } }
    pub fn stop(&self) { let _ = self.stop_tx.send(()); }
}

impl Drop for HotkeyHandle { fn drop(&mut self) { self.stop(); } }