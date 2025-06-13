use std::{path::PathBuf, process::Command};

fn main() {
    let out_dir = std::env::var("OUT_DIR").unwrap();

    let link = |x: &str| {
        println!("cargo:rustc-link-lib={}", x);
    };

    let link_search = |x: &str| {
        println!("cargo:rustc-link-search={}/{}", out_dir, x);
    };

    link_search("build/src");

    link("static=utf8_range");
    link("static=utf8_validity");
    link("static=absl_failure_signal_handler");
    link("static=absl_log_internal_fnmatch");
    link("static=absl_raw_hash_set");
    link("static=absl_bad_any_cast_impl");
    link("static=absl_flags_commandlineflag");
    link("static=absl_log_internal_format");
    link("static=absl_raw_logging_internal");
    link("static=absl_bad_optional_access");
    link("static=absl_flags_commandlineflag_internal");
    link("static=absl_log_internal_globals");
    link("static=absl_bad_variant_access");
    link("static=absl_flags_config");
    link("static=absl_log_internal_log_sink_set");
    link("static=absl_scoped_set_env");
    link("static=absl_base");
    link("static=absl_flags_internal");
    link("static=absl_log_internal_message");
    link("static=absl_spinlock_wait");
    link("static=absl_city");
    link("static=absl_flags_marshalling");
    link("static=absl_log_internal_nullguard");
    link("static=absl_stacktrace");
    link("static=absl_civil_time");
    link("static=absl_flags_parse");
    link("static=absl_log_internal_proto");
    link("static=absl_status");
    link("static=absl_cord");
    link("static=absl_flags_private_handle_accessor");
    link("static=absl_log_severity");
    link("static=absl_cord_internal");
    link("static=absl_flags_program_name");
    link("static=absl_log_sink");
    link("static=absl_statusor");
    link("static=absl_cordz_functions");
    link("static=absl_flags_reflection");
    link("static=absl_low_level_hash");
    link("static=absl_strerror");
    link("static=absl_cordz_handle");
    link("static=absl_flags_usage");
    link("static=absl_malloc_internal");
    link("static=absl_str_format_internal");
    link("static=absl_cordz_info");
    link("static=absl_flags_usage_internal");
    link("static=absl_periodic_sampler");
    link("static=absl_strings");
    link("static=absl_cordz_sample_token");
    link("static=absl_graphcycles_internal");
    link("static=absl_poison");
    link("static=absl_strings_internal");
    link("static=absl_crc32c");
    link("static=absl_hash");
    link("static=absl_random_distributions");
    link("static=absl_string_view");
    link("static=absl_crc_cord_state");
    link("static=absl_hashtablez_sampler");
    link("static=absl_random_internal_distribution_test_util");
    link("static=absl_symbolize");
    link("static=absl_crc_cpu_detect");
    link("static=absl_int128");
    link("static=absl_random_internal_platform");
    link("static=absl_synchronization");
    link("static=absl_crc_internal");
    link("static=absl_kernel_timeout_internal");
    link("static=absl_random_internal_pool_urbg");
    link("static=absl_throw_delegate");
    link("static=absl_debugging_internal");
    link("static=absl_leak_check");
    link("static=absl_random_internal_randen");
    link("static=absl_time");
    link("static=absl_decode_rust_punycode");
    link("static=absl_log_entry");
    link("static=absl_random_internal_randen_hwaes");
    link("static=absl_time_zone");
    link("static=absl_demangle_internal");
    link("static=absl_log_flags");
    link("static=absl_random_internal_randen_hwaes_impl");
    link("static=absl_utf8_for_code_point");
    link("static=absl_demangle_rust");
    link("static=absl_log_globals");
    link("static=absl_random_internal_randen_slow");
    link("static=absl_vlog_config_internal");
    link("static=absl_die_if_null");
    link("static=absl_log_initialize");
    link("static=absl_random_internal_seed_material");
    link("static=absl_examine_stack");
    link("static=absl_log_internal_check_op");
    link("static=absl_random_seed_gen_exception");
    link("static=absl_exponential_biased");
    link("static=absl_log_internal_conditions");
    link("static=absl_random_seed_sequences");
    link("GameNetworkingSockets_s");

    let mut c = cmake::Config::new("thirdparty/GameNetworkingSockets");

    if cfg!(target_os = "windows") {
        if Command::new("git")
            .args(&[
                "clone",
                "https://github.com/microsoft/vcpkg",
                "thirdparty/GameNetworkingSockets/vcpkg",
            ])
            .status()
            .is_ok()
        {
            Command::new("thirdparty/GameNetworkingSockets/vcpkg/bootstrap-vcpkg.bat")
                .status()
                .unwrap();
        }
        Command::new("thirdparty/GameNetworkingSockets/vcpkg/vcpkg")
            .args(&[
                "install",
                "--x-manifest-root=thirdparty/GameNetworkingSockets",
                "--triplet=x64-windows-static-md-release",
            ])
            .status()
            .unwrap();

        let profile = std::env::var("PROFILE").unwrap();
        if profile == "release" {
            link_search("build/src/Release");
        } else {
            link_search("build/src/Debug");
        }
        link_search("build/vcpkg_installed/x64-windows-static-md-release/lib");

        link("static=libprotobuf");
        link("static=absl_log_internal_structured_proto");

        c.define("USE_CRYPTO", "BCrypt");
        c.define("VCPKG_TARGET_TRIPLET", "x64-windows-static-md-release");
        c.define("VCPKG_BUILD_TYPE", profile.clone());
    } else {
        link("static=protobuf");
        link("static=crypto");
        link("static=ssl");
        link("stdc++");
    }

    c.static_crt(false);
    c.define("BUILD_STATIC_LIB", "ON");
    c.define("BUILD_SHARED_LIB", "OFF");
    c.define("OPENSSL_USE_STATIC_LIB", "ON");
    c.define("Protobuf_USE_STATIC_LIBS", "ON");
    c.build();

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
        .use_core()
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
