#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // /SUBSYSTEM:WINDOWS hide console

use std::ffi::c_void;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::HBRUSH;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, KEYBDINPUT, INPUT_KEYBOARD, KEYEVENTF_KEYUP,
};
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NOTIFYICONDATAW, NIM_ADD, NIM_DELETE, NIF_ICON, NIF_MESSAGE, NIF_TIP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DispatchMessageW,
    GetCursorPos, GetMessageW, PostQuitMessage, RegisterClassW, LoadImageW, SetForegroundWindow,
    SetWindowsHookExW, TrackPopupMenu, TranslateMessage, UnhookWindowsHookEx, AppendMenuW,
    HHOOK, HCURSOR, HICON, KBDLLHOOKSTRUCT, MSG, WNDCLASSW,
    TPM_BOTTOMALIGN, TPM_LEFTALIGN, WH_KEYBOARD_LL, WM_COMMAND, WM_DESTROY, WM_KEYDOWN, WM_KEYUP,
    WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_USER, LLKHF_INJECTED, IMAGE_ICON, LR_DEFAULTCOLOR,
    LR_DEFAULTSIZE, WM_INPUT, MF_STRING, MF_CHECKED, MF_UNCHECKED,
};

// virtual key code
const VK_LMENU:    u32 = 0xA4; // left alt
const VK_RMENU:    u32 = 0xA5; // right alt
const VK_IME_OFF:  u16 = 0x1A; // ime off
const VK_IME_ON:   u16 = 0x16; // ime on
const VK_LCONTROL: u16 = 0xA2; // left ctrl (emitted in place of CapsLock)

// can code of the CapsLock key (also the "Eisu / CapsLock" key on Japanese keyboards).
// the virtual-key code of this key can differ between key down and key up
// (e.g. VK_CAPITAL vs VK_OEM_ATTN depending on the Shift state), so the key
// is identified by its scan code instead of vkCode.
const SC_CAPSLOCK: u32 = 0x3A;

// raw Input constants (defined locally to stay independent of alias types in windows-sys)
const RID_INPUT:        u32 = 0x1000_0003; // GetRawInputData: read the RAWINPUT data
const RIM_TYPEKEYBOARD: u32 = 1;           // RAWINPUTHEADER.dwType: keyboard
const RI_KEY_BREAK:     u16 = 0x0001;      // RAWKEYBOARD.Flags: key up
const RIDEV_INPUTSINK:  u32 = 0x0000_0100; // receive input even when not in the foreground

// custom window message for system tray notifications
const WM_MY_TRAYICON: u32 = WM_USER + 1;
// system tray menu item id (exit)
const IDM_EXIT: usize = 1001;
// system tray menu item id (toggle CapsLock -> Ctrl)
const IDM_CAPS_TOGGLE: usize = 1002;

// alt state
static mut IS_COMBINATION: bool = false;
static mut LALT_PRESSED: bool = false;
static mut RALT_PRESSED: bool = false;

// CapsLock -> Ctrl state
static mut CAPS_REMAP_ENABLED: bool = true; // feature on/off (toggled from the tray menu)
static mut CAPS_PRESSED: bool = false;      // true while the emulated Ctrl is held down

static mut HOOK_HANDLE: HHOOK = null_mut();

fn main() {
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

        // register for Raw Input (keyboard).
        // Raw Input is not affected by what the low-level hook swallows,
        // so it is used as a reliable source of the CapsLock "key up" event.
        let rid = RAWINPUTDEVICE {
            usUsagePage: 0x01, // generic desktop controls
            usUsage: 0x06,     // keyboard
            dwFlags: RIDEV_INPUTSINK,
            hwndTarget: hwnd,
        };
        if RegisterRawInputDevices(&rid, 1, size_of::<RAWINPUTDEVICE>() as u32) == 0 {
            // not fatal: the hook alone still handles CapsLock key up in most cases
            eprintln!("failed to register raw input device");
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

        // never leave the emulated Ctrl key stuck on exit
        caps_up();

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

                    // add "CapsLock -> Ctrl" item (checked while enabled)
                    let caps_text = encode_utf16("CapsLock -> Ctrl");
                    let caps_flags = MF_STRING
                        | if CAPS_REMAP_ENABLED { MF_CHECKED } else { MF_UNCHECKED };
                    AppendMenuW(h_menu, caps_flags, IDM_CAPS_TOGGLE, caps_text.as_ptr());

                    // add Exit menu item
                    let exit_text = encode_utf16("Exit");
                    AppendMenuW(h_menu, MF_STRING, IDM_EXIT, exit_text.as_ptr());

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

        WM_INPUT => {
            // Raw Input: used to detect the CapsLock key up reliably
            unsafe { handle_raw_input(l_param); }
            // DefWindowProc must be called for WM_INPUT so the system can clean up
            unsafe { DefWindowProcW(hwnd, msg, w_param, l_param) }
        }

        WM_COMMAND => {
            match w_param {
                // terminate the message loop if Exit is selected
                IDM_EXIT => unsafe { PostQuitMessage(0); },
                // toggle CapsLock -> Ctrl
                IDM_CAPS_TOGGLE => unsafe {
                    CAPS_REMAP_ENABLED = !CAPS_REMAP_ENABLED;
                    if !CAPS_REMAP_ENABLED {
                        // release Ctrl if it is currently held by the emulation
                        caps_up();
                    }
                },
                _ => {}
            }
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

        let vk = kbd_struct.vkCode;

        // WM_KEYDOWN    : the virtual-key code of the nonsystem key
        //                 A nonsystem key is a key that is pressed when the ALT key is not pressed
        // WM_SYSKEYDOWN : the virtual-key code of the key being pressed
        //                 F10 key (which activates the menu bar) or holds down the ALT key and then presses another key
        // WM_KEYUP      : the virtual-key code of the nonsystem key
        //                 a nonsystem key is a key that is pressed when the ALT key is not pressed
        // WM_SYSKEYUP   : the user releases a key that was pressed while the ALT key was held down
        let is_down = w_param == WM_KEYDOWN as usize || w_param == WM_SYSKEYDOWN as usize;
        let is_up   = w_param == WM_KEYUP as usize || w_param == WM_SYSKEYUP as usize;

        // CapsLock -> Ctrl
        // The key is identified by its scan code, not by vkCode (see SC_CAPSLOCK).
        // Both down and up are always consumed so that the CapsLock state never toggles.
        if unsafe { CAPS_REMAP_ENABLED } && kbd_struct.scanCode == SC_CAPSLOCK {
            if is_down {
                unsafe { caps_down(); }
            } else if is_up {
                unsafe { caps_up(); }
            }
            // consume an event
            return 1;
        }

        // key down
        if is_down {

            if vk == VK_LMENU { // left alt key
                // if Ctrl (emulated CapsLock) is held, Alt is passed through
                // to the system as part of a combination instead of being consumed
                if unsafe { alt_down(true) } {
                    return unsafe { CallNextHookEx(HOOK_HANDLE, n_code, w_param, l_param) };
                }
                // consume an event
                return 1;

            } else if vk == VK_RMENU { // right alt key
                if unsafe { alt_down(false) } {
                    return unsafe { CallNextHookEx(HOOK_HANDLE, n_code, w_param, l_param) };
                }
                // consume an event
                return 1;

            } else {
                // if another key is pressed while Alt is held down,
                // it is considered a combination
                unsafe { mark_combination_and_refire_alt(); }
            }
        }

        // key up
        if is_up {

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

// Called when a key other than Alt goes down (including the emulated Ctrl).
// Marks the current Alt press as a combination and re-fires Alt keydown
// to trigger the native shortcut (e.g., Alt+Tab).
unsafe fn mark_combination_and_refire_alt() {
    unsafe {
        if LALT_PRESSED || RALT_PRESSED {
            IS_COMBINATION = true;

            let active_alt = if LALT_PRESSED {
                VK_LMENU as u16
            } else {
                VK_RMENU as u16
            };
            send_key_event(active_alt, false);
        }
    }
}

// Alt keydown bookkeeping.
// Returns `true` if the event must be passed to the system (not consumed).
//
// - auto-repeat of an already pressed Alt: consumed, state untouched
// - first press while Ctrl (emulated CapsLock) is held: the press is a combination
//   from the start, so the IME toggle is suppressed on release and the Alt keydown
//   is passed through, giving the system a proper Ctrl+Alt
// - first press otherwise: consumed, the decision is deferred to the key up
unsafe fn alt_down(is_left: bool) -> bool {
    unsafe {
        let already_pressed = if is_left { LALT_PRESSED } else { RALT_PRESSED };
        if already_pressed {
            return false;
        }

        // reset combination (it is a combination right away if Ctrl is held)
        IS_COMBINATION = CAPS_PRESSED;

        // mark the alt key pressed
        if is_left {
            LALT_PRESSED = true;
        } else {
            RALT_PRESSED = true;
        }

        CAPS_PRESSED
    }
}

// CapsLock physically pressed: start emulating Ctrl.
unsafe fn caps_down() {
    unsafe {
        // ignore auto-repeat of the held key
        if CAPS_PRESSED {
            return;
        }
        CAPS_PRESSED = true;

        // treat it like any other key pressed while Alt is held down
        mark_combination_and_refire_alt();

        send_key_event(VK_LCONTROL, false);
    }
}

// CapsLock released: stop emulating Ctrl.
// Idempotent: it is called from both the low-level hook and Raw Input,
// whichever observes the key up first wins and the other one is a no-op.
unsafe fn caps_up() {
    unsafe {
        if !CAPS_PRESSED {
            return;
        }
        CAPS_PRESSED = false;

        send_key_event(VK_LCONTROL, true);
    }
}

// WM_INPUT handler: watch only for the CapsLock key up.
unsafe fn handle_raw_input(l_param: LPARAM) {
    unsafe {
        let mut raw: RAWINPUT = std::mem::zeroed();
        let mut size = size_of::<RAWINPUT>() as u32;

        let ret = GetRawInputData(
            l_param as HRAWINPUT,
            RID_INPUT,
            &mut raw as *mut RAWINPUT as *mut c_void,
            &mut size,
            size_of::<RAWINPUTHEADER>() as u32,
        );
        // GetRawInputData returns (UINT)-1 on error
        if ret == u32::MAX || raw.header.dwType != RIM_TYPEKEYBOARD {
            return;
        }

        let kb = raw.data.keyboard;
        if kb.MakeCode as u32 == SC_CAPSLOCK && (kb.Flags & RI_KEY_BREAK) != 0 {
            caps_up();
        }
    }
}

// send a single key event (down or up)
unsafe fn send_key_event(vk_code: u16, key_up: bool) {
    unsafe {
        let mut input: INPUT = std::mem::zeroed();
        input.r#type = INPUT_KEYBOARD;
        input.Anonymous.ki = KEYBDINPUT {
            wVk: vk_code,
            wScan: 0,
            dwFlags: if key_up { KEYEVENTF_KEYUP } else { 0 },
            time: 0,
            dwExtraInfo: 0,
        };
        SendInput(1, &input, size_of::<INPUT>() as i32);
    }
}

// send a key event (down and up)
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

// utility for encoding string slice to UTF-16 
fn encode_utf16(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
