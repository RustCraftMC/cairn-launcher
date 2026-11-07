fn main() {
    println!("cargo:rerun-if-changed=../../assets/logo.ico");
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("../../assets/logo.ico");
        if res.compile().is_ok() && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu") {
            let object = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("resource.o");
            println!("cargo:rustc-link-arg={}", object.display());
        }
    }
}
