use std::{
    env,
    path::PathBuf,
};

fn main() {
    if cfg!(target_os = "windows") {
        // These libraries are prefixed with "lib" on Windows
        println!("cargo:rustc-link-lib=libprotobuf");
        println!("cargo:rustc-link-lib=libcrypto");
        println!("cargo:rustc-link-lib=libssl");
    } else {
        println!("cargo:rustc-link-lib=protobuf");
        println!("cargo:rustc-link-lib=crypto");
        println!("cargo:rustc-link-lib=ssl");
    }

    if cfg!(target_os = "windows") && cfg!(target_env = "msvc") {
        println!("cargo:rustc-link-lib=abseil_dll");
    } else {
        println!("cargo:rustc-link-lib=absl_log_internal_check_op");
        println!("cargo:rustc-link-lib=absl_log_internal_message");
    }

    let bindings = bindgen::Builder::default()
        .clang_arg("-Ithirdparty/GameNetworkingSockets/include/")
        .clang_arg("-Ithirdparty/GameNetworkingSockets/src/public/")
        .clang_arg("-Ithirdparty/GameNetworkingSockets/src/common/")
        .clang_arg("-Ithirdparty/GameNetworkingSockets/src/common/")
        .clang_arg("-DSTEAMNETWORKINGSOCKETS_STANDALONELIB")
        .header("thirdparty/GameNetworkingSockets/include/steam/steamnetworkingsockets_flat.h")
        .header("thirdparty/GameNetworkingSockets/include/steam/steamnetworkingsockets.h")
        .derive_debug(true)
        .derive_default(true)
        .derive_copy(true)
        .derive_partialord(true)
        .derive_ord(true)
        .derive_partialeq(true)
        .derive_eq(true)
        .derive_hash(true)
        .layout_tests(false)
        .default_enum_style(bindgen::EnumVariation::Rust {
            non_exhaustive: false,
        })
        .clang_arg("-xc++")
        .clang_arg("-std=c++20")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("Unable to generate bindings");

    let out_path = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");

    let mut cmake = cmake::Config::new("thirdparty/GameNetworkingSockets");

    let target_features = env::var("CARGO_CFG_TARGET_FEATURE").unwrap_or_default();
    if cfg!(target_os = "windows") && cfg!(target_env = "msvc")
            && target_features.contains("crt-static") {
        cmake.define("MSVC_CRT_STATIC", "ON");
    }

    let dst = cmake
        .profile("Release")
        .define("BUILD_STATIC_LIB", "ON")
        .define("BUILD_SHARED_LIB", "OFF")
        .build();

    println!("cargo:rustc-link-search=native={}", dst.join("lib").display());
    // The static library is suffixed with _s
    println!("cargo:rustc-link-lib=static=GameNetworkingSockets_s");
}
