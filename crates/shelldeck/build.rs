fn main() {
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../packaging/icons/shelldeck.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=failed to embed Windows resources: {e}");
        }
    }
}
