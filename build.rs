fn main() {
    // Interception import library path.
    // Set INTERCEPTION_LIB_DIR to override, e.g.:
    //   $env:INTERCEPTION_LIB_DIR="D:\libs\interception"
    let lib_dir = std::env::var("INTERCEPTION_LIB_DIR")
        .unwrap_or_else(|_| r"E:\Program\Interception\library\x64".into());
    println!("cargo:rustc-link-search=native={}", lib_dir);
    println!("cargo:rustc-link-lib=interception");
}
