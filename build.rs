fn main() {
    #[cfg(target_os = "windows")]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("docs/ime-alt.ico");
        res.set("FileDescription", "Left/right Alt for IME on/off.");
        res.set("FileVersion", "0.1.0.0");
        res.set("LegalCopyright", "Copyright (C) mammb.com. All rights reserved.");
        res.set("ProductName", "ime-alt");
        res.set("OriginalFilename", "ime-alt.exe");
        res.set("ProductVersion", "0.1.0.0");
        res.compile().unwrap();
    }
}
