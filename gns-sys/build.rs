use std::{
    path::{Path, PathBuf},
    process::Command,
};

fn main() {
    println!("cargo:rustc-link-lib=protobuf");
    println!("cargo:rustc-link-lib=crypto");
    println!("cargo:rustc-link-lib=ssl");
    println!("cargo:rustc-link-lib=absl_log_internal_check_op");
    println!("cargo:rustc-link-lib=absl_log_internal_message");

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

    Command::new("protoc").args(&[
      "--proto_path=thirdparty/GameNetworkingSockets/src/common",
      "--cpp_out=thirdparty/GameNetworkingSockets/src/common/",
      "thirdparty/GameNetworkingSockets/src/common/steamnetworkingsockets_messages.proto",
      "thirdparty/GameNetworkingSockets/src/common/steamnetworkingsockets_messages_certs.proto",
      "thirdparty/GameNetworkingSockets/src/common/steamnetworkingsockets_messages_udp.proto"
    ]).current_dir(Path::new("./"))
    .status().unwrap();

    let mut cc = cc::Build::new();

    if cfg!(target_os = "linux") {
        cc.define("linux", None);
    } else if cfg!(target_os = "windows") {
        cc.define("_WIN32", None);
        cc.define("_WINDOWS", None);
    }

    cc.cpp(true)
        .define("STEAMNETWORKINGSOCKETS_STATIC_LINK", None)
        .define("VALVE_CRYPTO_OPENSSL", None)
        .define("VALVE_CRYPTO_ENABLE_25519", None)
        .define("VALVE_CRYPTO_25519_OPENSSLEVP", None)
        .include("thirdparty/GameNetworkingSockets/include/")
        .include("thirdparty/GameNetworkingSockets/src/public/")
        .include("thirdparty/GameNetworkingSockets/src/common/")
        .files([
            "thirdparty/GameNetworkingSockets/src/common/crypto.cpp",
            "thirdparty/GameNetworkingSockets/src/common/crypto_textencode.cpp",
            "thirdparty/GameNetworkingSockets/src/common/keypair.cpp",
            "thirdparty/GameNetworkingSockets/src/common/crypto_openssl.cpp",
            "thirdparty/GameNetworkingSockets/src/common/crypto_25519_openssl.cpp",
            "thirdparty/GameNetworkingSockets/src/common/crypto_digest_opensslevp.cpp",
            "thirdparty/GameNetworkingSockets/src/common/crypto_symmetric_opensslevp.cpp",
            "thirdparty/GameNetworkingSockets/src/common/opensslwrapper.cpp",
        ])
        .files([
            "thirdparty/GameNetworkingSockets/src/common/steamnetworkingsockets_messages.pb.cc",
            "thirdparty/GameNetworkingSockets/src/common/steamnetworkingsockets_messages_certs.pb.cc",
            "thirdparty/GameNetworkingSockets/src/common/steamnetworkingsockets_messages_udp.pb.cc",

            "thirdparty/GameNetworkingSockets/src/common/steamid.cpp",
	          "thirdparty/GameNetworkingSockets/src/steamnetworkingsockets/steamnetworkingsockets_certs.cpp",
	          "thirdparty/GameNetworkingSockets/src/steamnetworkingsockets/steamnetworkingsockets_certstore.cpp",
	          "thirdparty/GameNetworkingSockets/src/steamnetworkingsockets/steamnetworkingsockets_shared.cpp",
            "thirdparty/GameNetworkingSockets/src/tier0/dbg.cpp",
            "thirdparty/GameNetworkingSockets/src/tier0/platformtime.cpp",
            "thirdparty/GameNetworkingSockets/src/tier1/netadr.cpp",
            "thirdparty/GameNetworkingSockets/src/tier1/utlbuffer.cpp",
            "thirdparty/GameNetworkingSockets/src/tier1/utlmemory.cpp",
            "thirdparty/GameNetworkingSockets/src/tier1/ipv6text.c",
            "thirdparty/GameNetworkingSockets/src/vstdlib/strtools.cpp",

	          "thirdparty/GameNetworkingSockets/src/steamnetworkingsockets/steamnetworkingsockets_stats.cpp",
	          "thirdparty/GameNetworkingSockets/src/steamnetworkingsockets/steamnetworkingsockets_thinker.cpp",
	          "thirdparty/GameNetworkingSockets/src/steamnetworkingsockets/clientlib/csteamnetworkingsockets.cpp",
	          "thirdparty/GameNetworkingSockets/src/steamnetworkingsockets/clientlib/csteamnetworkingmessages.cpp",
	          "thirdparty/GameNetworkingSockets/src/steamnetworkingsockets/clientlib/steamnetworkingsockets_flat.cpp",
	          "thirdparty/GameNetworkingSockets/src/steamnetworkingsockets/clientlib/steamnetworkingsockets_connections.cpp",
	          "thirdparty/GameNetworkingSockets/src/steamnetworkingsockets/clientlib/steamnetworkingsockets_lowlevel.cpp",
	          "thirdparty/GameNetworkingSockets/src/steamnetworkingsockets/clientlib/steamnetworkingsockets_p2p.cpp",
	          "thirdparty/GameNetworkingSockets/src/steamnetworkingsockets/clientlib/steamnetworkingsockets_stun.cpp",
	          "thirdparty/GameNetworkingSockets/src/steamnetworkingsockets/clientlib/steamnetworkingsockets_p2p_ice.cpp",
	          "thirdparty/GameNetworkingSockets/src/steamnetworkingsockets/clientlib/steamnetworkingsockets_snp.cpp",
	          "thirdparty/GameNetworkingSockets/src/steamnetworkingsockets/clientlib/steamnetworkingsockets_udp.cpp",

        ])
        .compiler("clang++")
        .flag("-std=c++20")
        .flag("-fvisibility=hidden")
        .flag("-fno-strict-aliasing")
        .flag("-Wall")
        .flag("-Wno-unknown-pragmas")
        .flag("-Wno-sign-compare")
        .flag("-Wno-unused-local-typedef")
        .flag("-Wno-unused-const-variable")
        .flag("-Wno-unused-parameter")
        .flag("-Wno-nested-anon-types")
        .flag("-O")
        .static_flag(true)
        .compile("GameNetworkingSockets");
}
