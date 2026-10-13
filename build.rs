// Embeds the application icon and version info into the Windows .exe.
fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "FrameMap Studio");
        res.set("FileDescription", "FrameMap Studio - conceptual framework builder");
        res.set("CompanyName", "Muruhukye Jacob");
        res.set("LegalCopyright", "MIT License");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed icon: {e}");
        }
    }
    println!("cargo:rerun-if-changed=assets/icon.ico");
}
