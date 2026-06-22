fn main() {
    // Link against the Interception import library
    println!("cargo:rustc-link-search=native=E:\\Program\\Interception\\library\\x64");
    println!("cargo:rustc-link-lib=interception");
}
