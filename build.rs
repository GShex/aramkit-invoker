fn main() {
    println!("cargo:rerun-if-changed=assets/aramkit-invoker.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/aramkit-invoker.ico")
            .compile()
            .expect("failed to embed the application icon");
    }
}
