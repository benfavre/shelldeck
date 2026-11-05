fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../packaging/icons/shelldeck.ico");
        res.set("ProductName", "Inklura Desktop");
        res.set("FileDescription", "Inklura Desktop");
        if let Err(e) = res.compile() {
            println!("cargo:warning=failed to embed Windows resources: {e}");
        }
    }
}
