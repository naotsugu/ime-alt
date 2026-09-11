#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // /SUBSYSTEM:WINDOWS hide console

use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::HBRUSH;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
};
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, NIF_ICON, NIF_MESSAGE, NIF_TIP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DispatchMessageW, GetCursorPos, GetMessageW, LoadIconW, PostQuitMessage,
    RegisterClassW, SetForegroundWindow, SetWindowsHookExW, TrackPopupMenu,
    TranslateMessage, UnhookWindowsHookEx, HHOOK, HCURSOR, HICON, KBDLLHOOKSTRUCT, MSG,
    TPM_BOTTOMALIGN, TPM_LEFTALIGN, WH_KEYBOARD_LL, WM_COMMAND, WM_DESTROY,
    WM_KEYDOWN, WM_KEYUP, WM_RBUTTONUP, WM_SYSKEYDOWN,
    WM_SYSKEYUP, WM_USER, WNDCLASSW, IDI_APPLICATION, LLKHF_INJECTED,
};


// virtual key code
const VK_LMENU:   u32 = 0xA4; // left alt
const VK_RMENU:   u32 = 0xA5; // right alt
const VK_IME_OFF: u16 = 0x1A; // ime off
const VK_IME_ON:  u16 = 0x16; // ime on

// custom window message for system tray notifications
const WM_MY_TRAYICON: u32 = WM_USER + 1;
// system tray menu item id
const IDM_EXIT: usize = 1001;

static mut HOOK_HANDLE: HHOOK = null_mut();

static mut IS_COMBINATION: bool = false;
static mut LALT_PRESSED: bool = false;
static mut RALT_PRESSED: bool = false;

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

        // register system tray icon
        let h_icon = LoadIconW(null_mut(), IDI_APPLICATION);
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
                    let menu_text = encode_utf16("Exit");
                    
                    // add Exit menu item
                    windows_sys::Win32::UI::WindowsAndMessaging::AppendMenuW(
                        h_menu,
                        windows_sys::Win32::UI::WindowsAndMessaging::MF_STRING,
                        IDM_EXIT,
                        menu_text.as_ptr(),
                    );

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
            // terminate the message loop if Exit is selected
            if w_param == IDM_EXIT {
                unsafe { PostQuitMessage(0); }
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
        if (kbd_struct.flags & LLKHF_INJECTED) != 0 {
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

            if vk == VK_LMENU /* left alt key */ {
                unsafe {
                    if !LALT_PRESSED {
                        // reset combination
                        IS_COMBINATION = false;
                        LALT_PRESSED = true;
                    }
                }
                // consume an event
                return 1;

            } else if vk == VK_RMENU /* right alt key */ {
                unsafe {
                    if !RALT_PRESSED {
                        // reset combination
                        IS_COMBINATION = false;
                        // mark right alt key pressed
                        RALT_PRESSED = true;
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

                        // for a key combination, immediately re-fire Alt KEYDOWN
                        // to trigger the native shortcut (e.g., Alt+Tab)
                        let active_alt = if LALT_PRESSED { VK_LMENU as u16 } else { VK_RMENU as u16 };
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
        //               
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
