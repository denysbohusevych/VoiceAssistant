use std::ffi::{c_void, CStr, CString};
use std::os::raw::{c_char, c_int};
use std::process::Command;
use std::path::Path;
use std::fs;

use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl};

use super::{AppSnapshot, ContextCapture, ContextError};

// ─── Типы ─────────────────────────────────────────────────────────────────────

type CFTypeRef       = *const c_void;
type CFArrayRef      = *const c_void;
type CFDictionaryRef = *const c_void;
type CFNumberRef     = *const c_void;
type CFStringRef     = *const c_void;
type CFAllocatorRef  = *const c_void;
type CFIndex         = isize;

// ─── FFI ──────────────────────────────────────────────────────────────────────

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: CFTypeRef);
    fn CFArrayGetCount(arr: CFArrayRef) -> CFIndex;
    fn CFArrayGetValueAtIndex(arr: CFArrayRef, idx: CFIndex) -> CFTypeRef;
    fn CFDictionaryGetValue(dict: CFDictionaryRef, key: CFTypeRef) -> CFTypeRef;
    fn CFNumberGetValue(n: CFNumberRef, ty: c_int, out: *mut c_void) -> bool;
    fn CFStringCreateWithCString(alloc: CFAllocatorRef, s: *const c_char, enc: u32) -> CFStringRef;
    static kCFAllocatorDefault: CFAllocatorRef;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGWindowListCopyWindowInfo(option: u32, relativeToWindow: u32) -> CFArrayRef;
}

const CF_STRING_UTF8:          u32   = 0x08000100;
const CF_NUMBER_INT32:         c_int = 3;
const CG_ON_SCREEN:            u32   = 1;
const CG_NULL_WINDOW:          u32   = 0;

// ─── Утилиты ──────────────────────────────────────────────────────────────────

unsafe fn cfstr(s: &str) -> CFStringRef {
    let c = CString::new(s).unwrap();
    CFStringCreateWithCString(kCFAllocatorDefault, c.as_ptr(), CF_STRING_UTF8)
}

unsafe fn dict_i32(dict: CFDictionaryRef, key: &str) -> Option<i32> {
    let k = cfstr(key);
    let v = CFDictionaryGetValue(dict, k as CFTypeRef);
    CFRelease(k as CFTypeRef);
    if v.is_null() { return None; }
    let mut val: i32 = 0;
    if CFNumberGetValue(v as CFNumberRef, CF_NUMBER_INT32, &mut val as *mut _ as *mut c_void) {
        Some(val)
    } else {
        None
    }
}

// ─── Интеграция с ax-helper ───────────────────────────────────────────────────

fn run_ax_helper(args: &[&str]) -> Option<String> {
    // 1. Получаем полный путь к текущему запущенному Rust-бинарнику (VoiceAssistant)
    let mut ax_helper_path = std::env::current_exe().expect("Не удалось получить путь к exe");

    // 2. Убираем имя файла VoiceAssistant, чтобы остаться в его директории (target/release)
    ax_helper_path.pop();

    // 3. Добавляем имя нашего Swift-скрипта
    ax_helper_path.push("ax-helper-bin");

    // ЛОГ: Показываем абсолютный путь, который будем запускать
    println!("  [ax-helper] Выполняю: {} {}", ax_helper_path.display(), args.join(" "));

    let output = match Command::new(&ax_helper_path).args(args).output() {
        Ok(out) => out,
        Err(e) => {
            eprintln!("  [ax-helper] ❌ Ошибка запуска процесса: {}", e);
            return None;
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !stderr.is_empty() {
        eprintln!("  [ax-helper] ⚠️ STDERR:\n{}", stderr);
    }

    if output.status.success() {
        if args[0] == "capture" {
            println!("  [ax-helper] ✅ Успешно. Получен JSON пути (длина: {})", stdout.len());
            println!("{}", stdout);
        } else {
            println!("  [ax-helper] ✅ Успешно: {}", stdout);
        }
        Some(stdout)
    } else {
        eprintln!("  [ax-helper] ❌ Команда {} завершилась с ошибкой: {}", args[0], output.status);
        None
    }
}

fn capture_screenshot_via_helper(pid: u32) -> Option<Vec<u8>> {
    let temp_path = format!("/tmp/capture_{}.png", pid);

    if run_ax_helper(&["screenshot", &temp_path]).is_some() {
        match fs::read(&temp_path) {
            Ok(bytes) => {
                let _ = fs::remove_file(&temp_path);
                println!("  [ax-helper] 📸 Скриншот прочитан ({} байт)", bytes.len());
                Some(bytes)
            }
            Err(e) => {
                eprintln!("  [ax-helper] ❌ Не удалось прочитать скриншот из {}: {}", temp_path, e);
                None
            }
        }
    } else {
        None
    }
}

fn capture_ax_path_via_helper(pid: u32) -> Option<String> {
    let pid_str = pid.to_string();
    run_ax_helper(&["capture", &pid_str])
}

// ─── Реализация ───────────────────────────────────────────────────────────────

pub struct MacOsContextCapture;

impl MacOsContextCapture {
    pub fn new() -> Self { Self }

    fn do_capture(&self, pid: u32) -> Result<AppSnapshot, ContextError> {
        let app_name        = app_name_for_pid(pid).unwrap_or_else(|| "Unknown".into());
        println!("  [capture] 🔍 Захват контекста для PID: {} ({})", pid, app_name);

        // Получаем координаты и id окна через нативные вызовы
        let cursor          = mouse_location();
        let window_id       = window_id_for_pid(pid);

        // Получаем скриншот и json-путь (для будущего инжекта) через наш бинарник
        let screenshot      = capture_screenshot_via_helper(pid);
        let ax_element_path = capture_ax_path_via_helper(pid);

        Ok(AppSnapshot {
            app_name,
            pid,
            cursor,
            window_id,
            screenshot,
            ax_element_path
        })
    }
}

impl ContextCapture for MacOsContextCapture {
    fn capture(&self) -> Result<AppSnapshot, ContextError> {
        let pid = frontmost_pid().ok_or(ContextError::NoFrontmostApp)?;
        self.do_capture(pid)
    }

    fn capture_for_pid(&self, pid: u32) -> Result<AppSnapshot, ContextError> {
        self.do_capture(pid)
    }
}

// ─── Системные вызовы ─────────────────────────────────────────────────────────

fn frontmost_pid() -> Option<u32> {
    unsafe {
        let ws:  *mut Object = msg_send![class!(NSWorkspace), sharedWorkspace];
        let app: *mut Object = msg_send![ws, frontmostApplication];
        if app.is_null() { return None; }
        let pid: i32 = msg_send![app, processIdentifier];
        Some(pid as u32)
    }
}

fn app_name_for_pid(pid: u32) -> Option<String> {
    unsafe {
        let app: *mut Object = msg_send![
            class!(NSRunningApplication),
            runningApplicationWithProcessIdentifier: pid as i32
        ];
        if app.is_null() { return None; }
        let name_obj: *mut Object = msg_send![app, localizedName];
        if name_obj.is_null() { return None; }
        let ptr: *const c_char = msg_send![name_obj, UTF8String];
        Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
    }
}

fn mouse_location() -> (f64, f64) {
    unsafe {
        #[repr(C)] struct NSPoint { x: f64, y: f64 }
        let pt: NSPoint = msg_send![class!(NSEvent), mouseLocation];
        (pt.x, pt.y)
    }
}

fn window_id_for_pid(pid: u32) -> Option<u32> {
    unsafe {
        let arr = CGWindowListCopyWindowInfo(CG_ON_SCREEN, CG_NULL_WINDOW);
        if arr.is_null() { return None; }

        let count = CFArrayGetCount(arr);
        let mut result = None;

        for i in 0..count {
            let dict = CFArrayGetValueAtIndex(arr, i) as CFDictionaryRef;
            if dict.is_null() { continue; }
            if dict_i32(dict, "kCGWindowLayer").unwrap_or(1) != 0 { continue; }
            if dict_i32(dict, "kCGWindowOwnerPID").unwrap_or(0) as u32 != pid { continue; }
            if let Some(wid) = dict_i32(dict, "kCGWindowNumber") {
                result = Some(wid as u32);
                break;
            }
        }

        CFRelease(arr as CFTypeRef);
        result
    }
}