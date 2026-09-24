#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // /SUBSYSTEM:WINDOWS hide console

use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ACCESS_DENIED, ERROR_CANCELLED, ERROR_FILE_NOT_FOUND,
    ERROR_GEN_FAILURE, ERROR_INVALID_PARAMETER, ERROR_SUCCESS, HWND, LPARAM, LRESULT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::HBRUSH;
use windows_sys::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_BINARY,
};
use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject, INFINITE};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, KEYBDINPUT, INPUT_KEYBOARD, KEYEVENTF_KEYUP,
};
use windows_sys::Win32::UI::Shell::{
    ShellExecuteExW, Shell_NotifyIconW,
    NIM_ADD, NIM_DELETE, NIF_ICON, NIF_MESSAGE, NIF_TIP, SEE_MASK_NOCLOSEPROCESS,
    NOTIFYICONDATAW, SHELLEXECUTEINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CallNextHookEx, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DispatchMessageW, GetCursorPos, GetMessageW, MessageBoxW, PostMessageW, PostQuitMessage,
    RegisterClassW, LoadImageW,
    SetForegroundWindow, SetWindowsHookExW, TrackPopupMenu, TranslateMessage, UnhookWindowsHookEx,
    HHOOK, HCURSOR, HICON, KBDLLHOOKSTRUCT, MSG, WNDCLASSW,
    TPM_BOTTOMALIGN, TPM_LEFTALIGN, WH_KEYBOARD_LL, WM_COMMAND, WM_DESTROY, WM_KEYDOWN, WM_KEYUP,
    WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_USER, LLKHF_INJECTED, IMAGE_ICON,
    LR_DEFAULTCOLOR, LR_DEFAULTSIZE,
    MB_ICONERROR, MB_ICONINFORMATION, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND,
    MF_CHECKED, MF_GRAYED, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, SW_HIDE,
};

// virtual key code
const VK_LMENU:   u32 = 0xA4; // left alt
const VK_RMENU:   u32 = 0xA5; // right alt
const VK_MENU:    i32 = 0x12; // alt (either side)
const VK_IME_OFF: u16 = 0x1A; // ime off
const VK_IME_ON:  u16 = 0x16; // ime on

// custom window message for system tray notifications
const WM_MY_TRAYICON: u32 = WM_USER + 1;
// custom window message posted by the registry worker thread when it finishes
// (wParam: Win32 error code, 0 = success / lParam: 1 = enabled, 0 = disabled)
const WM_CAPS_DONE: u32 = WM_USER + 2;
// system tray menu item id
const IDM_EXIT: usize = 1001;
const IDM_ALT_IME: usize = 1002;   // toggle Alt -> IME
const IDM_CAPS_CTRL: usize = 1003; // toggle CapsLock -> Ctrl

// registry: CapsLock -> Ctrl is done with the "Scancode Map" value
const KEYBOARD_LAYOUT_KEY: &str = r"SYSTEM\CurrentControlSet\Control\Keyboard Layout";
const SCANCODE_MAP_VALUE: &str = "Scancode Map";
const SC_CAPSLOCK: u16 = 0x3A;
const SC_LCTRL: u16 = 0x1D;

// command line of the elevated helper process: "--set-caps-ctrl on|off"
const ARG_SET_CAPS_CTRL: &str = "--set-caps-ctrl";

// NOTE: the following `static mut` values are only accessed from the main thread
// (the low-level keyboard hook callback and the window procedure both run on the thread
// that runs the message loop), so there is no data race.
static mut HOOK_HANDLE: HHOOK = null_mut();

static mut IS_COMBINATION: bool = false;
static mut LALT_PRESSED: bool = false;
static mut RALT_PRESSED: bool = false;

// Alt -> IME switching on/off (toggled from the tray menu, enabled at startup)
static mut ALT_IME_ENABLED: bool = true;

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

    unsafe {

        let h_instance = GetModuleHandleW(null_mut());

        // create window class (dummy)
        let class_name = encode_utf16("ImeAltClass");
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
        ) as windows_sys::Win32::UI::WindowsAndMessaging::HICON;
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
        Shell_NotifyIconW(NIM_ADD, &nid);

        // register keybord hook
        let hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), h_instance, 0);
        if hook.is_null() {
            eprintln!("failed to register the hook");
            Shell_NotifyIconW(NIM_DELETE, &nid);
            return;
        }
        HOOK_HANDLE = hook;

        // message loop
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        // clear event hook
        UnhookWindowsHookEx(HOOK_HANDLE);
        Shell_NotifyIconW(NIM_DELETE, &nid);
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
                        | if ALT_IME_ENABLED { MF_CHECKED } else { MF_UNCHECKED };
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

                    DestroyMenu(h_menu);
                }
            }
            0
        }
        WM_COMMAND => {
            match w_param {
                // terminate the message loop if Exit is selected
                IDM_EXIT => unsafe { PostQuitMessage(0); },

                // toggle Alt -> IME switching
                IDM_ALT_IME => unsafe {
                    ALT_IME_ENABLED = !ALT_IME_ENABLED;
                    // do not leave a half-processed Alt press behind
                    IS_COMBINATION = false;
                    LALT_PRESSED = false;
                    RALT_PRESSED = false;
                },

                // toggle CapsLock -> Ctrl (writes the registry in a worker thread)
                IDM_CAPS_CTRL => start_caps_ctrl_job(hwnd, !is_caps_ctrl_registered()),

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
            return unsafe { CallNextHookEx(HOOK_HANDLE, n_code, w_param, l_param) };
        }

        // Alt -> IME switching is turned off from the tray menu: do nothing
        if unsafe { !ALT_IME_ENABLED } {
            return unsafe { CallNextHookEx(HOOK_HANDLE, n_code, w_param, l_param) };
        }

        let vk = kbd_struct.vkCode;

        // key down
        // WM_KEYDOWN    : the virtual-key code of the nonsystem key
        //                 A nonsystem key is a key that is pressed when the ALT key is not pressed
        // WM_SYSKEYDOWN : the virtual-key code of the key being pressed
        //                 F10 key (which activates the menu bar) or holds down the ALT key and then presses another key
        if w_param == WM_KEYDOWN as usize ||
            w_param == WM_SYSKEYDOWN as usize {

            if vk == VK_LMENU {
                // left alt key
                unsafe {
                    if !LALT_PRESSED {
                        // reset combination
                        // if another key is already held, it is a combination from the start
                        IS_COMBINATION = is_other_key_pressed();
                        // mark left alt key pressed
                        LALT_PRESSED = true;
                        // another key is held: pass the Alt keydown through instead of consuming it
                        if IS_COMBINATION {
                            return CallNextHookEx(HOOK_HANDLE, n_code, w_param, l_param);
                        }
                    }
                }
                // consume an event
                return 1;

            } else if vk == VK_RMENU {
                // right alt key
                unsafe {
                    if !RALT_PRESSED {
                        // reset combination
                        // if another key is already held, it is a combination from the start
                        IS_COMBINATION = is_other_key_pressed();
                        // mark right alt key pressed
                        RALT_PRESSED = true;
                        // another key is held: pass the Alt keydown through instead of consuming it
                        if IS_COMBINATION {
                            return CallNextHookEx(HOOK_HANDLE, n_code, w_param, l_param);
                        }
                    }
                }
                // consume an event
                return 1;

            } else {
                // if another key is pressed while Alt is held down,
                // it is considered a combination
                unsafe {
                    if LALT_PRESSED || RALT_PRESSED {
                        IS_COMBINATION = true;

                        // for a key combination, immediately re-fire Alt keydown
                        // to trigger the native shortcut (e.g., Alt+Tab)
                        let active_alt = if LALT_PRESSED {
                            VK_LMENU as u16
                        } else {
                            VK_RMENU as u16
                        };
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
        if w_param == WM_KEYUP as usize ||
            w_param == WM_SYSKEYUP as usize {

            if vk == VK_LMENU || vk == VK_RMENU {

                let was_combination = unsafe { IS_COMBINATION };

                // clear stat
                unsafe {
                    if vk == VK_LMENU { LALT_PRESSED = false; }
                    if vk == VK_RMENU { RALT_PRESSED = false; }
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

    unsafe { CallNextHookEx(HOOK_HANDLE, n_code, w_param, l_param) }
}


// returns true if any key other than Alt is currently held down
// (the most significant bit of GetAsyncKeyState means "currently down")
fn is_other_key_pressed() -> bool {
    // 0x01..=0x07 are mouse buttons, so start from 0x08 (the first keyboard key)
    (0x08..=0xFE_i32).any(|vk| {
        // skip the Alt keys (generic, left, right)
        if vk == VK_MENU || vk == VK_LMENU as i32 || vk == VK_RMENU as i32 {
            return false;
        }
        (unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000) != 0
    })
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

        // first call: get the required size
        let rc = RegQueryValueExW(
            hkey, value_name.as_ptr(), null(), null_mut(), null_mut(), &mut size,
        );
        let result = if rc == ERROR_FILE_NOT_FOUND {
            Ok(None)
        } else if rc != ERROR_SUCCESS {
            Err(rc)
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
        sei.fMask = SEE_MASK_NOCLOSEPROCESS;
        sei.lpVerb = verb.as_ptr();
        sei.lpFile = path.as_ptr();
        sei.lpParameters = params.as_ptr();
        sei.nShow = SW_HIDE as i32;
        if ShellExecuteExW(&mut sei) == 0 {
            return Err(last_error());
        }

        // the exit code of the helper is the Win32 error code (0 = success)
        let h_process = sei.hProcess;
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


// utility for encoding string slice to UTF-16
fn encode_utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
