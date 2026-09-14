fn main() {
    #[cfg(target_os = "windows")]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("docs/ime-alt.ico");
        res.set("FileDescription", "ime-alt - Left/right Alt for IME on/off.");
        res.set("LegalCopyright", "Copyright (C) mammb.com. All rights reserved.");
        res.set("ProductName", "ime-alt");
        res.set("OriginalFilename", "ime-alt.exe");
        res.compile().unwrap();
    }
}
