use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, SetWindowsHookExW, UnhookWindowsHookEx,
    KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_SYSKEYDOWN,
    WM_KEYUP, WM_SYSKEYUP, HHOOK, LLKHF_INJECTED,
};

// virtual key code
const VK_LMENU: u32 = 0xA4;   // left alt
const VK_RMENU: u32 = 0xA5;   // right alt
const VK_IME_OFF: u16 = 0x1A; // ime off
const VK_IME_ON: u16 = 0x16;  // ime on

static mut HOOK_HANDLE: HHOOK = null_mut();

static mut IS_COMBINATION: bool = false;
static mut LALT_PRESSED: bool = false;
static mut RALT_PRESSED: bool = false;

fn main() {

    unsafe {
        let h_instance = GetModuleHandleW(null_mut());
        let hook = SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(keyboard_proc),
            h_instance,
            0,
        );

        if hook.is_null() {
            eprintln!("failed to register the hook");
            return;
        }
        HOOK_HANDLE = hook;

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            // message loop
        }

        UnhookWindowsHookEx(HOOK_HANDLE);
    }
}


unsafe extern "system" fn keyboard_proc(
    n_code: i32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    if n_code >= 0 {
        let kbd_struct = *(l_param as *const KBDLLHOOKSTRUCT);
        
        // immediately pass injected inputs from SendInput to the next hook without processing
        if (kbd_struct.flags & LLKHF_INJECTED) != 0 {
            return CallNextHookEx(HOOK_HANDLE, n_code, w_param, l_param);
        }

        let vk = kbd_struct.vkCode;

        // key down
        // WM_KEYDOWN    : the virtual-key code of the nonsystem key
        //                 ALT key is not pressed
        // WM_SYSKEYDOWN : the virtual-key code of the key being pressed
        //                 F10 key (which activates the menu bar) or holds down the ALT key and then presses another key
        if w_param == WM_KEYDOWN as usize ||
           w_param == WM_SYSKEYDOWN as usize {

            if vk == VK_LMENU /* Left Alt key */ {
                if !LALT_PRESSED {
                    // reset combination
                    IS_COMBINATION = false;
                    LALT_PRESSED = true;
                }
                // block alt down event
                return 1;
            } else if vk == VK_RMENU /* Right Alt key */ {
                if !RALT_PRESSED {
                    // reset combination
                    IS_COMBINATION = false;
                    RALT_PRESSED = true;
                }
                // block alt down event
                return 1;
            } else {
                // if another key is pressed while Alt is held down, it is considered a combination
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

        // key up
        if w_param == WM_KEYUP as usize ||
           w_param == WM_SYSKEYUP as usize {

            if vk == VK_LMENU || vk == VK_RMENU {
                let was_combination = IS_COMBINATION;
                
                // clear stat
                if vk == VK_LMENU { LALT_PRESSED = false; }
                if vk == VK_RMENU { RALT_PRESSED = false; }

                // switch the IME only on a single press of the alt key
                if !was_combination {
                    if vk == VK_LMENU {
                        send_ime_key(VK_IME_OFF);
                    } else {
                        send_ime_key(VK_IME_ON);
                    }
                    // consum event
                    return 1;
                }
            }
        }
    }

    CallNextHookEx(HOOK_HANDLE, n_code, w_param, l_param)
}

unsafe fn send_ime_key(vk_code: u16) {

    let mut inputs: [INPUT; 2] = std::mem::zeroed();

    // key down
    inputs[0].r#type = INPUT_KEYBOARD;
    inputs[0].Anonymous.ki = KEYBDINPUT {
        wVk: vk_code,
        wScan: 0,
        dwFlags: 0,
        time: 0,
        dwExtraInfo: 0,
    };

    // key up
    inputs[1].r#type = INPUT_KEYBOARD;
    inputs[1].Anonymous.ki = KEYBDINPUT {
        wVk: vk_code,
        wScan: 0,
        dwFlags: KEYEVENTF_KEYUP,
        time: 0,
        dwExtraInfo: 0,
    };

    // send input event
    SendInput(2, inputs.as_mut_ptr(), std::mem::size_of::<INPUT>() as i32);
}
