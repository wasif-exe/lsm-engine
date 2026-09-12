fn main() {
    // Register "loom" as a valid custom configuration flag to suppress compiler warnings
    println!("cargo:rustc-check-cfg=cfg(loom)");
}