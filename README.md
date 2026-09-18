# ime-alt

<p align="center">
  <img src="https://github.com/naotsugu/ime-alt/blob/main/docs/128x128.png" alt="あlt">
</p>

A small tool that provides Mac-like IME switching on US-ASCII keyboards.

## Features

Switch between IME ON / OFF using the Alt keys, similar to how you can switch input modes on a Mac:

* Left Alt key (left of the space bar) ➔ IME OFF
* Right Alt key (right of the space bar) ➔ IME ON

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

## Uninstallation

To uninstall `ime-alt`, exit the application and delete the downloaded `ime-alt.exe`.

If you have added `ime-alt` to the Windows Startup folder, also delete its shortcut from the Startup folder.

## Notice on Anti-Virus False Positives

The application executable may occasionally be flagged as a Trojan or malware by some anti-virus (AV) software, particularly those utilizing machine learning heuristics. Such false positives are known to occur somewhat frequently with Rust projects.

While it might be possible to mitigate this by modifying optimization flags or changing dependency crates to alter the resulting binary pattern, we have decided not to pursue these workarounds in this repository. This issue is limited to a small number of AV programs, and attempting to constantly bypass their detection often turns into a never-ending game of cat-and-mouse.

If your AV software flags `ime-alt.exe` as malicious, it would be highly appreciated if you could report it as a false positive through your AV vendor's official reporting form.
