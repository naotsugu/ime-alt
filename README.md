# ime-alt

<p align="center">
  <img src="https://github.com/naotsugu/ime-alt/blob/main/docs/128x128.png" alt="あlt">
</p>

A small tool that provides Mac-like IME switching on Windows US-ASCII keyboards.

## Features

Switch between IME ON / OFF using the Alt keys, similar to how you can switch input modes on a Mac:

- Left Alt key (left of the space bar) ➔ IME OFF
- Right Alt key (right of the space bar) ➔ IME ON
- CapsLock key ➔ Ctrl key (optional, see [Remapping CapsLock to Ctrl](#remapping-capslock-to-ctrl))

## Installation

1. Download the latest binary from the [Releases](https://github.com/naotsugu/ime-alt/releases).
2. Extract the downloaded ZIP file to any folder.
3. Run `ime-alt.exe`.

There is no installer. Simply extract the archive to a folder and run `ime-alt.exe`.

## Startup

To automatically start `ime-alt` when Windows starts, add a shortcut to `ime-alt.exe` to the Windows Startup folder.

1. Choose a folder where you want to keep `ime-alt.exe`.
2. Right-click `ime-alt.exe` and select **Create shortcut**.
3. Press `Win + R` to open the **Run** dialog, enter `shell:startup`.
4. Move the shortcut you created into the Startup folder.

> [!NOTE]
> If you move `ime-alt.exe` to another folder, you will need to recreate the shortcut in the Startup folder.

## Usage

1. Run `ime-alt.exe` to start the application in the background.
2. To exit the application, right-click its system tray icon and select **Exit**.

## Remapping CapsLock to Ctrl

`ime-alt` can also remap the CapsLock key to Ctrl.
This feature is optional and disabled by default.

To enable it, right-click the `ime-alt` icon in the system tray and select **CapsLock to Ctrl**.

> [!IMPORTANT]
> The remapping is done by changing the Scancode Map in the Windows registry
> (`HKEY_LOCAL_MACHINE\SYSTEM\CurrentControlSet\Control\Keyboard Layout`), not by hooking keyboard events.
> Therefore:
>
> - **Administrator access** is required to change the setting.
> - You need to **sign out** (and sign back in) for the change to take effect.
>
> <sub>Implementing the remapping with a keyboard hook can be unreliable, because key-up events are sometimes not delivered. Changing the Scancode Map in the registry is the most stable approach, since the remapping is handled by Windows itself.</sub>

## Uninstallation

To uninstall `ime-alt`, exit the application and delete the downloaded `ime-alt.exe`.

If you have added `ime-alt` to the Windows Startup folder, also delete its shortcut from the Startup folder.

## Notice on Anti-Virus False Positives

The application executable may occasionally be flagged as a Trojan or malware by some anti-virus (AV) software, particularly those utilizing machine learning heuristics. Such false positives are known to occur somewhat frequently with Rust projects.

While it might be possible to mitigate this by modifying optimization flags or changing dependency crates to alter the resulting binary pattern, we have decided not to pursue these workarounds in this repository. This issue is limited to a small number of AV programs, and attempting to constantly bypass their detection often turns into a never-ending game of cat-and-mouse.

If your AV software flags `ime-alt.exe` as malicious, it would be highly appreciated if you could report it as a false positive through your AV vendor's official reporting form.
