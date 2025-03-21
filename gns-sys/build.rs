use std::path::PathBuf;

fn main() {
    println!("cargo:rustc-link-lib=protobuf");
    println!("cargo:rustc-link-lib=crypto");
    println!("cargo:rustc-link-lib=ssl");
    println!("cargo:rustc-link-lib=absl_log_internal_check_op");
    println!("cargo:rustc-link-lib=absl_log_internal_message");
    println!("cargo:rustc-link-lib=GameNetworkingSockets_s");
    println!("cargo:rustc-link-lib=stdc++");
    println!(
        "cargo:rustc-link-search={}/lib64",
        std::env::var("OUT_DIR").unwrap()
    );

    cmake::Config::new("thirdparty/GameNetworkingSockets")
        .define("BUILD_STATIC_LIB", "1")
        .build();

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
}
