fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc")
    {
        // Measured debug render frames (including GPUI components) need more
        // headroom than the default 1 MiB main stack. Reserve 8 MiB for koyori;
        // Windows commits stack pages on demand as the stack grows.
        println!("cargo:rustc-link-arg-bin=koyori=/STACK:8388608");
    }
}
