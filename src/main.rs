#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // /SUBSYSTEM:WINDOWS hide console

use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, Ordering};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_CANCELLED, ERROR_FILE_NOT_FOUND,
    ERROR_GEN_FAILURE, ERROR_INVALID_PARAMETER, ERROR_SUCCESS, HWND, LPARAM, LRESULT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::HBRUSH;
use windows_sys::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_BINARY,
};
use windows_sys::Win32::System::RemoteDesktop::{
    WTSRegisterSessionNotification, WTSUnRegisterSessionNotification,
};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, GetExitCodeProcess, OpenProcess, WaitForSingleObject, INFINITE,
    PROCESS_SYNCHRONIZE,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, KEYBDINPUT, INPUT_KEYBOARD, KEYEVENTF_KEYUP,
};
use windows_sys::Win32::UI::Shell::{
    ShellExecuteExW, Shell_NotifyIconW,
    NIM_ADD, NIM_DELETE, NIF_ICON, NIF_MESSAGE, NIF_TIP, SEE_MASK_NOCLOSEPROCESS, SEE_MASK_NOASYNC,
    NOTIFYICONDATAW, SHELLEXECUTEINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CallNextHookEx, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon,
    DestroyMenu, DestroyWindow, DispatchMessageW, GetCursorPos, GetMessageW, MessageBoxW,
    PostMessageW, PostQuitMessage, RegisterClassW,  RegisterWindowMessageW, LoadImageW,
    SetForegroundWindow, SetWindowsHookExW, TrackPopupMenu, TranslateMessage,
    UnhookWindowsHookEx, UnregisterClassW,
    HCURSOR, HICON, KBDLLHOOKSTRUCT, MSG, WNDCLASSW,
    TPM_BOTTOMALIGN, TPM_LEFTALIGN, WH_KEYBOARD_LL, WM_COMMAND, WM_DESTROY, WM_KEYDOWN, WM_KEYUP,
    WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_USER, LLKHF_INJECTED, IMAGE_ICON,
    LR_DEFAULTCOLOR, LR_DEFAULTSIZE,
    MB_ICONERROR, MB_ICONINFORMATION, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, WM_NULL,
    MF_CHECKED, MF_GRAYED, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, SW_HIDE,
};

// virtual key code
const VK_LMENU:   u32 = 0xA4; // left alt
const VK_RMENU:   u32 = 0xA5; // right alt
const VK_IME_OFF: u16 = 0x1A; // ime off
const VK_IME_ON:  u16 = 0x16; // ime on

// custom window message for system tray notifications
const WM_MY_TRAYICON: u32 = WM_USER + 1;
// custom window message posted by the registry worker thread when it finishes
// (wParam: Win32 error code, 0 = success / lParam: 1 = enabled, 0 = disabled)
const WM_CAPS_DONE: u32 = WM_USER + 2;
// session change notification (sent when the session is locked / unlocked)
const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
const WTS_SESSION_LOCK: usize = 0x7;
const WTS_SESSION_UNLOCK: usize = 0x8;
const NOTIFY_FOR_THIS_SESSION: u32 = 0;

// system tray menu item id
const IDM_EXIT: usize = 1001;
const IDM_ALT_IME: usize = 1002;   // toggle Alt -> IME
const IDM_CAPS_CTRL: usize = 1003; // toggle CapsLock -> Ctrl
const IDM_RESTART: usize = 1004;   // restart this application

// registry: CapsLock -> Ctrl is done with the "Scancode Map" value
const KEYBOARD_LAYOUT_KEY: &str = r"SYSTEM\CurrentControlSet\Control\Keyboard Layout";
const SCANCODE_MAP_VALUE: &str = "Scancode Map";
const SC_CAPSLOCK: u16 = 0x3A;
const SC_LCTRL: u16 = 0x1D;

// command line of the elevated helper process: "--set-caps-ctrl on|off"
const ARG_SET_CAPS_CTRL: &str = "--set-caps-ctrl";
// command line of the restarted process: "--wait-pid <pid>"
// the new process waits for the old one to exit before it takes the single instance mutex
const ARG_WAIT_PID: &str = "--wait-pid";

// hook handle and keyboard state flags accessed by the keyboard hook and window procedure.
// atomic types are used for thread-safe interior mutability without `static mut`.
static HOOK_HANDLE: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(null_mut());

// tray icon handle, needed to re-add the icon from the window procedure
static TRAY_ICON: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(null_mut());
// registered message id of "TaskbarCreated" (broadcast when Explorer restarts)
static WM_TASKBAR_CREATED: AtomicU32 = AtomicU32::new(0);

static LALT_PRESSED: AtomicBool = AtomicBool::new(false);
static RALT_PRESSED: AtomicBool = AtomicBool::new(false);

// per-side state of "the Alt key was passed through to the OS" (combination use).
// it must be kept per side, otherwise LAlt and RAlt overwrite each other
static LALT_COMB: AtomicBool = AtomicBool::new(false);
static RALT_COMB: AtomicBool = AtomicBool::new(false);

// keys currently held (virtual-key code bitmap, 256 bits), tracked only from real
// (non-injected) hook events. GetAsyncKeyState is not used because a stuck key
// state survives an application restart
static KEYS_DOWN: [AtomicU64; 4] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];

// Alt -> IME switching on/off (toggled from the tray menu, enabled at startup)
static ALT_IME_ENABLED: AtomicBool = AtomicBool::new(true);

// true while the registry worker thread is running
static CAPS_JOB_RUNNING: AtomicBool = AtomicBool::new(false);

fn main() {
    // elevated helper mode:
    // apply the registry change, then exit without creating a tray icon or a hook
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 3 && args[1] == ARG_SET_CAPS_CTRL {
        let code = match args[2].as_str() {
            "on" | "off" => match write_scancode_map(args[2] == "on") {
                Ok(()) => 0,
                Err(e) => e as i32,
            },
            _ => ERROR_INVALID_PARAMETER as i32,
        };
        std::process::exit(code);
    }

    // restarted by the tray menu:
    // wait until the previous process exits, so that its mutex and keyboard hook are gone
    if args.len() >= 3 && args[1] == ARG_WAIT_PID {
        if let Ok(pid) = args[2].parse::<u32>() {
            wait_for_process_exit(pid);
        }
    }

    unsafe {
        // single instance guard:
        // a second instance would install a second hook, and every Alt press would toggle the IME twice
        // (the "Local\" prefix makes the name per-session)
        let mutex_name = encode_utf16(r"Local\ime-alt-single-instance");
        let mutex = CreateMutexW(null(), 0, mutex_name.as_ptr());
        // GetLastError must be called right after CreateMutexW
        let mutex_error = GetLastError();
        if mutex.is_null() || mutex_error == ERROR_ALREADY_EXISTS {
            // there is no console (Windows subsystem), so tell the user with a dialog
            let text = if mutex.is_null() {
                format!("Failed to create the single instance mutex. (error code: {})", mutex_error)
            } else {
                "ime-alt is already running.\nCheck the system tray or the task manager.".to_string()
            };
            let icon = if mutex.is_null() { MB_ICONERROR } else { MB_ICONINFORMATION };
            let text_w = encode_utf16(&text);
            let title_w = encode_utf16("ime-alt");
            MessageBoxW(null_mut(), text_w.as_ptr(), title_w.as_ptr(), MB_OK | icon | MB_SETFOREGROUND);

            if !mutex.is_null() {
                CloseHandle(mutex);
            }
            return;
        }
        let h_instance = GetModuleHandleW(null_mut());

        // create window class (dummy)
        let class_name = encode_utf16("ImeAltClass");

        // register before the window is created, so that no broadcast is missed
        let taskbar_created = RegisterWindowMessageW(encode_utf16("TaskbarCreated").as_ptr());
        WM_TASKBAR_CREATED.store(taskbar_created, Ordering::Relaxed);

        let wnd_class = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(window_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: h_instance,
            hIcon: 0 as HICON,
            hCursor: 0 as HCURSOR,
            hbrBackground: 0 as HBRUSH,
            lpszMenuName: null_mut(),
            lpszClassName: class_name.as_ptr(),
        };
        RegisterClassW(&wnd_class);

        // create hidden window
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            null_mut(),
            0,
            0, 0, 0, 0,
            null_mut(),
            null_mut(),
            h_instance,
            null_mut()
        );
        if hwnd.is_null() {
            eprintln!("failed to create window");
            UnregisterClassW(class_name.as_ptr(), h_instance);
            return;
        }

        // register system tray icon
        // If you set the width and height to 0 and specify LR_DEFAULTSIZE,
        // an appropriate size is automatically selected—such as 16x16 at 100% DPI, 20x20 at 125% DPI, and so on.
        let h_icon = LoadImageW(
            h_instance,
            1 as *const u16, // resource id 1
            IMAGE_ICON,
            0,
            0,
            LR_DEFAULTCOLOR | LR_DEFAULTSIZE,
        ) as HICON;
        TRAY_ICON.store(h_icon, Ordering::Relaxed);
        add_tray_icon(hwnd, h_icon);

        // get lock / unlock notifications of this session
        WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION);

        // register keyboard hook
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), h_instance, 0);
        if hook.is_null() {
            eprintln!("failed to register the hook");
            remove_tray_icon(hwnd);
            if !h_icon.is_null() {
                DestroyIcon(h_icon);
            }
            WTSUnRegisterSessionNotification(hwnd);
            DestroyWindow(hwnd);
            UnregisterClassW(class_name.as_ptr(), h_instance);
            return;
        }
        HOOK_HANDLE.store(hook, Ordering::Relaxed);

        // message loop
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        // clear event hook and cleanup resources
        UnhookWindowsHookEx(HOOK_HANDLE.load(Ordering::Relaxed));
        remove_tray_icon(hwnd);
        if !h_icon.is_null() {
            DestroyIcon(h_icon);
        }
        WTSUnRegisterSessionNotification(hwnd);
        DestroyWindow(hwnd);
        UnregisterClassW(class_name.as_ptr(), h_instance);
    }
}


unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    match msg {
        WM_MY_TRAYICON => {
            // mouse event (lParam LOWORD)
            let event = (l_param & 0xFFFF) as u32;
            if event == WM_RBUTTONUP {
                unsafe {
                    // create a menu when the system tray icon is clicked
                    let h_menu = CreatePopupMenu();

                    // add "Alt -> IME" item (checked while enabled)
                    let alt_text = encode_utf16("Alt to IME");
                    let alt_flags = MF_STRING
                        | if ALT_IME_ENABLED.load(Ordering::Relaxed) { MF_CHECKED } else { MF_UNCHECKED };
                    AppendMenuW(h_menu, alt_flags, IDM_ALT_IME, alt_text.as_ptr());

                    // add "CapsLock -> Ctrl" item
                    // the check mark reflects the registry (it is not the active state
                    // until the next restart / sign-out); grayed out while a change is in progress
                    let caps_text = encode_utf16("CapsLock to Ctrl");
                    let mut caps_flags = MF_STRING
                        | if is_caps_ctrl_registered() { MF_CHECKED } else { MF_UNCHECKED };
                    if CAPS_JOB_RUNNING.load(Ordering::SeqCst) {
                        caps_flags |= MF_GRAYED;
                    }
                    AppendMenuW(h_menu, caps_flags, IDM_CAPS_CTRL, caps_text.as_ptr());

                    // separator
                    AppendMenuW(h_menu, MF_SEPARATOR, 0, null());

                    // add "Restart" item
                    // grayed out while the registry change is in progress,
                    // because the worker thread would be killed in the middle of the work
                    let restart_text = encode_utf16("Restart");
                    let mut restart_flags = MF_STRING;
                    if CAPS_JOB_RUNNING.load(Ordering::SeqCst) {
                        restart_flags |= MF_GRAYED;
                    }
                    AppendMenuW(h_menu, restart_flags, IDM_RESTART, restart_text.as_ptr());

                    // add Exit menu item
                    let menu_text = encode_utf16("Exit");
                    AppendMenuW(h_menu, MF_STRING, IDM_EXIT, menu_text.as_ptr());

                    // get current mouse position
                    let mut pos = std::mem::zeroed();
                    GetCursorPos(&mut pos);

                    // to close the menu when clicking outside
                    SetForegroundWindow(hwnd);

                    // show pop-up menu and wait for selection
                    TrackPopupMenu(
                        h_menu,
                        TPM_LEFTALIGN | TPM_BOTTOMALIGN,
                        pos.x,
                        pos.y,
                        0,
                        hwnd,
                        null_mut(),
                    );
                    // required for tray popup menus: without this, the menu may not close
                    // correctly on the next click (see KB135788)
                    PostMessageW(hwnd, WM_NULL, 0, 0);

                    DestroyMenu(h_menu);
                }
            }
            0
        }
        WM_COMMAND => {
            // the low word of wParam is the command id (the high word is the notification code)
            match w_param & 0xFFFF {
                // terminate the message loop if Exit is selected
                IDM_EXIT => unsafe { PostQuitMessage(0); },

                // toggle Alt -> IME switching
                IDM_ALT_IME => {
                    let current = ALT_IME_ENABLED.load(Ordering::Relaxed);
                    ALT_IME_ENABLED.store(!current, Ordering::Relaxed);
                    // do not leave a half-processed Alt press behind
                    LALT_COMB.store(false, Ordering::Relaxed);
                    RALT_COMB.store(false, Ordering::Relaxed);
                    LALT_PRESSED.store(false, Ordering::Relaxed);
                    RALT_PRESSED.store(false, Ordering::Relaxed);
                    clear_keys();
                },

                // toggle CapsLock -> Ctrl (writes the registry in a worker thread)
                IDM_CAPS_CTRL => start_caps_ctrl_job(hwnd, !is_caps_ctrl_registered()),

                // restart: start a new process, then quit this one
                IDM_RESTART => {
                    if CAPS_JOB_RUNNING.load(Ordering::SeqCst) {
                        return 0;
                    }
                    match spawn_restart() {
                        Ok(()) => unsafe { PostQuitMessage(0); },
                        Err(e) => {
                            // keep running if the new process could not be started
                            let text_w = encode_utf16(&format!("Failed to restart. ({})", e));
                            let title_w = encode_utf16("ime-alt");
                            unsafe {
                                MessageBoxW(hwnd, text_w.as_ptr(), title_w.as_ptr(), MB_OK | MB_ICONERROR | MB_SETFOREGROUND);
                            }
                        }
                    }
                },

                _ => {}
            }
            0
        }
        // the registry worker thread has finished: tell the user what happened
        WM_CAPS_DONE => {
            let code = w_param as u32;
            let enabled = l_param != 0;

            let (text, icon) = if code == 0 {
                let state = if enabled { "enabled" } else { "disabled" };
                (
                    format!(
                        "CapsLock -> Ctrl has been {}.\nSign out or restart Windows to apply the change.",
                        state
                    ),
                    MB_ICONINFORMATION,
                )
            } else if code == ERROR_CANCELLED {
                // the user declined the UAC prompt
                (
                    "Administrator permission was not granted.\nThe setting was not changed.".to_string(),
                    MB_ICONWARNING,
                )
            } else {
                (
                    format!("Failed to change the setting. (error code: {})", code),
                    MB_ICONERROR,
                )
            };

            let text_w = encode_utf16(&text);
            let title_w = encode_utf16("ime-alt");
            unsafe {
                MessageBoxW(hwnd, text_w.as_ptr(), title_w.as_ptr(), MB_OK | icon | MB_SETFOREGROUND);
            }

            // clear the flag after the message box is closed,
            // so that the menu item stays disabled while the box is shown
            CAPS_JOB_RUNNING.store(false, Ordering::SeqCst);
            0
        }
        // the session was locked / unlocked: key up events may have been lost
        // (e.g. Ctrl+Alt+Del), so forget the recorded keys
        WM_WTSSESSION_CHANGE => {
            if w_param == WTS_SESSION_LOCK || w_param == WTS_SESSION_UNLOCK {
                clear_keys();
            }
            0
        }
        // Explorer was restarted and the taskbar was recreated: the tray icon is gone, add it again
        m if m != 0 && m == WM_TASKBAR_CREATED.load(Ordering::Relaxed) => {
            unsafe { add_tray_icon(hwnd, TRAY_ICON.load(Ordering::Relaxed) as HICON); }
            0
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0); }
            0
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, w_param, l_param) },
    }
}


unsafe extern "system" fn keyboard_proc(
    n_code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    if n_code >= 0 {

        let kbd_struct = unsafe { *(l_param as *const KBDLLHOOKSTRUCT) };

        // immediately pass injected inputs from SendInput to the next hook without processing
        // LLKHF_INJECTED : specifies whether the event was injected programmatically
        //                  (e.g., via the SendInput function)
        if (kbd_struct.flags & LLKHF_INJECTED) != 0 {
            return unsafe { CallNextHookEx(HOOK_HANDLE.load(Ordering::Relaxed), n_code, w_param, l_param) };
        }

        let vk = kbd_struct.vkCode;
        let is_down = w_param == WM_KEYDOWN as usize || w_param == WM_SYSKEYDOWN as usize;
        let is_up = w_param == WM_KEYUP as usize || w_param == WM_SYSKEYUP as usize;

        // record the state of the keys other than Alt from real events.
        // this is done even while Alt -> IME is turned off, so the record stays in sync
        if vk != VK_LMENU && vk != VK_RMENU {
            if is_down {
                set_key(vk, true);
            } else if is_up {
                set_key(vk, false);
            }
        }

        // Alt -> IME switching is turned off from the tray menu: do nothing
        if !ALT_IME_ENABLED.load(Ordering::Relaxed) {
            return unsafe { CallNextHookEx(HOOK_HANDLE.load(Ordering::Relaxed), n_code, w_param, l_param) };
        }

        // key down
        // WM_KEYDOWN    : the virtual-key code of the nonsystem key
        //                 A nonsystem key is a key that is pressed when the ALT key is not pressed
        // WM_SYSKEYDOWN : the virtual-key code of the key being pressed
        //                 F10 key (which activates the menu bar) or holds down the ALT key and then presses another key
        if is_down {

            if vk == VK_LMENU {
                // left alt key
                if !LALT_PRESSED.load(Ordering::Relaxed) {
                    // reset combination of this side
                    // if another key is already held, it is a combination from the start
                    let comb = is_other_key_pressed();
                    LALT_COMB.store(comb, Ordering::Relaxed);
                    // mark left alt key pressed
                    LALT_PRESSED.store(true, Ordering::Relaxed);
                    // another key is held: pass the Alt keydown through instead of consuming it
                    if comb {
                        return unsafe { CallNextHookEx(HOOK_HANDLE.load(Ordering::Relaxed), n_code, w_param, l_param) };
                    }
                }
                // consume an event
                return 1;

            } else if vk == VK_RMENU {
                // right alt key
                if !RALT_PRESSED.load(Ordering::Relaxed) {
                    // reset combination of this side
                    // if another key is already held, it is a combination from the start
                    let comb = is_other_key_pressed();
                    RALT_COMB.store(comb, Ordering::Relaxed);
                    // mark right alt key pressed
                    RALT_PRESSED.store(true, Ordering::Relaxed);
                    // another key is held: pass the Alt keydown through instead of consuming it
                    if comb {
                        return unsafe { CallNextHookEx(HOOK_HANDLE.load(Ordering::Relaxed), n_code, w_param, l_param) };
                    }
                }
                // consume an event
                return 1;

            } else {
                // if another key is pressed while Alt is held down,
                // it is considered a combination (of every Alt key being held)
                let lalt = LALT_PRESSED.load(Ordering::Relaxed);
                let ralt = RALT_PRESSED.load(Ordering::Relaxed);
                if lalt || ralt {
                    if lalt { LALT_COMB.store(true, Ordering::Relaxed); }
                    if ralt { RALT_COMB.store(true, Ordering::Relaxed); }

                    // for a key combination, immediately re-fire Alt keydown
                    // to trigger the native shortcut (e.g., Alt+Tab)
                    let active_alt = if lalt {
                        VK_LMENU as u16
                    } else {
                        VK_RMENU as u16
                    };
                    unsafe {
                        let mut inputs: [INPUT; 1] = std::mem::zeroed();
                        inputs[0].r#type = INPUT_KEYBOARD;
                        inputs[0].Anonymous.ki = KEYBDINPUT {
                            wVk: active_alt,
                            wScan: 0,
                            dwFlags: 0,
                            time: 0,
                            dwExtraInfo: 0,
                        };
                        SendInput(1, inputs.as_mut_ptr(), std::mem::size_of::<INPUT>() as i32);
                    }
                }
            }
        }

        // key up
        // WM_KEYUP    : the virtual-key code of the nonsystem key
        //               a nonsystem key is a key that is pressed when the ALT key is not pressed
        // WM_SYSKEYUP : the user releases a key that was pressed while the ALT key was held down
        if is_up {

            if vk == VK_LMENU || vk == VK_RMENU {

                // whether the keydown of THIS side was passed through to the OS.
                // if it was, the keyup must be passed through too, otherwise the OS keeps the Alt held
                let was_combination = if vk == VK_LMENU {
                    LALT_COMB.load(Ordering::Relaxed)
                } else {
                    RALT_COMB.load(Ordering::Relaxed)
                };

                // clear state
                if vk == VK_LMENU {
                    LALT_PRESSED.store(false, Ordering::Relaxed);
                    LALT_COMB.store(false, Ordering::Relaxed);
                }
                if vk == VK_RMENU {
                    RALT_PRESSED.store(false, Ordering::Relaxed);
                    RALT_COMB.store(false, Ordering::Relaxed);
                }

                // toggle the IME only on a single press of the alt key
                if !was_combination {
                    unsafe {
                        if vk == VK_LMENU {
                            send_key_press(VK_IME_OFF);
                        } else {
                            send_key_press(VK_IME_ON);
                        }
                    }
                    // consume an event
                    return 1;
                }
            }
        }
    }

    unsafe { CallNextHookEx(HOOK_HANDLE.load(Ordering::Relaxed), n_code, w_param, l_param) }
}

// record a key down / up event (Alt keys are not recorded)
fn set_key(vk: u32, down: bool) {
    let idx = (vk as usize / 64) & 3;
    let bit = 1u64 << (vk % 64);
    if down {
        KEYS_DOWN[idx].fetch_or(bit, Ordering::Relaxed);
    } else {
        KEYS_DOWN[idx].fetch_and(!bit, Ordering::Relaxed);
    }
}

// forget all recorded keys (used when the state may be out of sync)
fn clear_keys() {
    for k in KEYS_DOWN.iter() {
        k.store(0, Ordering::Relaxed);
    }
}

// returns true if any key other than Alt is currently held down
fn is_other_key_pressed() -> bool {
    KEYS_DOWN.iter().any(|k| k.load(Ordering::Relaxed) != 0)
}

unsafe fn send_key_press(vk_code: u16) {
    unsafe {
        let mut inputs: [INPUT; 2] = std::mem::zeroed();

        // [0] key down
        inputs[0].r#type = INPUT_KEYBOARD;
        inputs[0].Anonymous.ki = KEYBDINPUT {
            wVk: vk_code,
            wScan: 0,
            dwFlags: 0, // KEYEVENTF_KEYDOWN
            time: 0,
            dwExtraInfo: 0,
        };

        // [1] key up
        inputs[1].r#type = INPUT_KEYBOARD;
        inputs[1].Anonymous.ki = KEYBDINPUT {
            wVk: vk_code,
            wScan: 0,
            dwFlags: KEYEVENTF_KEYUP,
            time: 0,
            dwExtraInfo: 0,
        };

        // send input event (key down -> up)
        SendInput(2, inputs.as_mut_ptr(), std::mem::size_of::<INPUT>() as i32);
    }
}

// start this exe again with "--wait-pid <current pid>"
fn spawn_restart() -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    std::process::Command::new(exe)
        .arg(ARG_WAIT_PID)
        .arg(std::process::id().to_string())
        .spawn()?;
    Ok(())
}

// wait until the given process exits (up to 10 seconds)
fn wait_for_process_exit(pid: u32) {
    unsafe {
        let h_process = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
        if h_process.is_null() {
            // the process has already exited
            return;
        }
        WaitForSingleObject(h_process, 10_000);
        CloseHandle(h_process);
    }
}

// ---------------------------------------------------------------------------
// CapsLock -> Ctrl (registry "Scancode Map")
// ---------------------------------------------------------------------------

// Scancode Map layout (all little endian):
//   header    : 8 bytes, zero
//   count     : u32, number of entries + 1 (the terminator)
//   entries   : 4 bytes each = (new scan code: u16, original scan code: u16)
//   terminator: 4 bytes, zero
//
// e.g. CapsLock (0x3A) -> left Ctrl (0x1D):
//   00 00 00 00 00 00 00 00 02 00 00 00 1D 00 3A 00 00 00 00 00

// parse a Scancode Map value into (new, original) scan code pairs
fn parse_scancode_map(data: &[u8]) -> Vec<(u16, u16)> {
    if data.len() < 12 {
        return Vec::new();
    }
    let count = u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;
    let mut entries = Vec::new();
    for i in 0..count.saturating_sub(1) {
        let off = 12 + i * 4;
        if off + 4 > data.len() {
            break;
        }
        let new_sc = u16::from_le_bytes([data[off], data[off + 1]]);
        let old_sc = u16::from_le_bytes([data[off + 2], data[off + 3]]);
        entries.push((new_sc, old_sc));
    }
    entries
}

// build a Scancode Map value from (new, original) scan code pairs
fn build_scancode_map(entries: &[(u16, u16)]) -> Vec<u8> {
    let mut data = vec![0u8; 8]; // header
    data.extend_from_slice(&(entries.len() as u32 + 1).to_le_bytes());
    for &(new_sc, old_sc) in entries {
        data.extend_from_slice(&new_sc.to_le_bytes());
        data.extend_from_slice(&old_sc.to_le_bytes());
    }
    data.extend_from_slice(&[0u8; 4]); // terminator
    data
}

// read the raw Scancode Map value (reading does not need administrator rights)
// Ok(None) : the value does not exist
// Err(code): Win32 error code
fn read_scancode_map() -> Result<Option<Vec<u8>>, u32> {
    unsafe {
        let key_name = encode_utf16(KEYBOARD_LAYOUT_KEY);
        let mut hkey: HKEY = null_mut();
        let rc = RegOpenKeyExW(HKEY_LOCAL_MACHINE, key_name.as_ptr(), 0, KEY_QUERY_VALUE, &mut hkey);
        if rc != ERROR_SUCCESS {
            return Err(rc);
        }

        let value_name = encode_utf16(SCANCODE_MAP_VALUE);
        let mut size: u32 = 0;
        let mut value_type: u32 = 0;

        // first call: get the required size
        let rc = RegQueryValueExW(
            hkey, value_name.as_ptr(), null(), &mut value_type, null_mut(), &mut size,
        );
        let result = if rc == ERROR_FILE_NOT_FOUND {
            Ok(None)
        } else if rc != ERROR_SUCCESS {
            Err(rc)
        } else if value_type != REG_BINARY {
            // not a binary value: do not try to parse it as a Scancode Map
            Ok(None)
        } else {
            // second call: read the data
            let mut buf = vec![0u8; size as usize];
            let rc = RegQueryValueExW(
                hkey, value_name.as_ptr(), null(), null_mut(), buf.as_mut_ptr(), &mut size,
            );
            if rc == ERROR_SUCCESS {
                buf.truncate(size as usize);
                Ok(Some(buf))
            } else {
                Err(rc)
            }
        };

        RegCloseKey(hkey);
        result
    }
}

// true if the registry currently maps CapsLock -> left Ctrl
// (it is the registry state; the mapping is active after a restart / sign-out)
fn is_caps_ctrl_registered() -> bool {
    match read_scancode_map() {
        Ok(Some(data)) => parse_scancode_map(&data).contains(&(SC_LCTRL, SC_CAPSLOCK)),
        _ => false,
    }
}

// add / remove the CapsLock -> Ctrl entry, keeping all the other mappings as they are
// Err(ERROR_ACCESS_DENIED) means administrator rights are required
fn write_scancode_map(enable: bool) -> Result<(), u32> {
    let mut entries = match read_scancode_map()? {
        Some(data) => parse_scancode_map(&data),
        None => Vec::new(),
    };

    if enable {
        // replace an existing mapping of CapsLock, if any
        entries.retain(|&(_, old_sc)| old_sc != SC_CAPSLOCK);
        entries.push((SC_LCTRL, SC_CAPSLOCK));
    } else {
        entries.retain(|&e| e != (SC_LCTRL, SC_CAPSLOCK));
    }

    unsafe {
        let key_name = encode_utf16(KEYBOARD_LAYOUT_KEY);
        let mut hkey: HKEY = null_mut();
        let rc = RegOpenKeyExW(HKEY_LOCAL_MACHINE, key_name.as_ptr(), 0, KEY_SET_VALUE, &mut hkey);
        if rc != ERROR_SUCCESS {
            return Err(rc);
        }

        let value_name = encode_utf16(SCANCODE_MAP_VALUE);
        let rc = if entries.is_empty() {
            // no mapping left: remove the value itself
            let rc = RegDeleteValueW(hkey, value_name.as_ptr());
            if rc == ERROR_FILE_NOT_FOUND { ERROR_SUCCESS } else { rc }
        } else {
            let data = build_scancode_map(&entries);
            RegSetValueExW(hkey, value_name.as_ptr(), 0, REG_BINARY, data.as_ptr(), data.len() as u32)
        };

        RegCloseKey(hkey);
        if rc == ERROR_SUCCESS { Ok(()) } else { Err(rc) }
    }
}

// GetLastError() that never returns 0,
// because 0 is used as "success" in WM_CAPS_DONE and in the exit code of the helper process
unsafe fn last_error() -> u32 {
    let e = unsafe { GetLastError() };
    if e == 0 { ERROR_GEN_FAILURE } else { e }
}

// re-launch this exe with administrator rights (UAC prompt) to apply the change,
// and wait for it to finish
// Err(ERROR_CANCELLED) means the user declined the UAC prompt
fn run_elevated_set(enable: bool) -> Result<(), u32> {
    unsafe {
        let mut path = [0u16; 1024];
        let len = GetModuleFileNameW(null_mut(), path.as_mut_ptr(), path.len() as u32);
        // 0: failure / buffer length: the path was truncated
        if len == 0 || len as usize >= path.len() {
            return Err(last_error());
        }

        let verb = encode_utf16("runas");
        let params = encode_utf16(&format!("{} {}", ARG_SET_CAPS_CTRL, if enable { "on" } else { "off" }));

        let mut sei: SHELLEXECUTEINFOW = std::mem::zeroed();
        sei.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        sei.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC; // run synchronously: this thread has no COM apartment
        sei.lpVerb = verb.as_ptr();
        sei.lpFile = path.as_ptr();
        sei.lpParameters = params.as_ptr();
        sei.nShow = SW_HIDE as i32;
        if ShellExecuteExW(&mut sei) == 0 {
            return Err(last_error());
        }

        // the exit code of the helper is the Win32 error code (0 = success)
        let h_process = sei.hProcess;
        if h_process.is_null() {
            return Err(last_error());
        }
        WaitForSingleObject(h_process, INFINITE);
        let mut code: u32 = 1;
        GetExitCodeProcess(h_process, &mut code);
        CloseHandle(h_process);

        if code == 0 { Ok(()) } else { Err(code) }
    }
}

// change the registry in a worker thread, so that the message loop (and the keyboard hook)
// is never blocked while the UAC prompt is shown
// when finished, WM_CAPS_DONE is posted to the window
fn start_caps_ctrl_job(hwnd: HWND, enable: bool) {
    // ignore the request while another change is in progress
    if CAPS_JOB_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }

    // a raw window handle cannot be moved to another thread as is
    let hwnd_value = hwnd as usize;

    std::thread::spawn(move || {
        // try with the current rights first, ask for elevation only when it is needed
        let result = match write_scancode_map(enable) {
            Err(ERROR_ACCESS_DENIED) => run_elevated_set(enable),
            other => other,
        };
        let code = result.err().unwrap_or(0);

        unsafe {
            // if posting fails, WM_CAPS_DONE never arrives: clear the flag here so the menu item is not stuck
            if PostMessageW(hwnd_value as HWND, WM_CAPS_DONE, code as WPARAM, enable as LPARAM) == 0 {
                CAPS_JOB_RUNNING.store(false, Ordering::SeqCst);
            }
        }
    });
}

// add the tray icon (also used to restore it after Explorer restarts)
unsafe fn add_tray_icon(hwnd: HWND, h_icon: HICON) -> bool {
    unsafe {
        let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 1;
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = WM_MY_TRAYICON;
        nid.hIcon = h_icon;
        let tip = encode_utf16("ime-alt");
        let copy_len = tip.len().min(nid.szTip.len() - 1);
        nid.szTip[..copy_len].copy_from_slice(&tip[..copy_len]);
        Shell_NotifyIconW(NIM_ADD, &nid) != 0
    }
}

// remove the tray icon (NIM_DELETE only needs cbSize, hWnd and uID)
unsafe fn remove_tray_icon(hwnd: HWND) {
    unsafe {
        let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 1;
        Shell_NotifyIconW(NIM_DELETE, &nid);
    }
}

// utility for encoding string slice to UTF-16
fn encode_utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
