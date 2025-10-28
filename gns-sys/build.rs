use std::{fs, path::PathBuf, process::Command};
use std::path::Path;

fn link(lib: impl AsRef<str>) {
    println!("cargo:rustc-link-lib={}", lib.as_ref());
}

fn link_search(build_subpath: impl AsRef<Path>) {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    println!("cargo:rustc-link-search={}", out_dir.join(build_subpath).display());
}

fn link_protobuf_default() {
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
    link("static=protobuf");
}

fn link_protobuf() {
    let mut config = pkg_config::Config::new();
    // if std::env::var("CARGO_CFG_TARGET_OS").unwrap() != "macos" {
    //     config.statik(true);
    // }
    let result = config
        .atleast_version("2.6.1")
        .probe("protobuf");
    match result {
        Err(pkg_config::Error::EnvNoPkgConfig(_)) => {
            println!(
                "cargo::warning=pkg-config was not found in PATH, using default lib link flags\
                 for protobuf"
            );
            link_protobuf_default();
        },
        Err(pkg_config::Error::ProbeFailure { name, command, output }) => {
            println!(
                "cargo::warning=library '{}' was not found by pkg-config; using default lib\
                 link flags\n{}",
                name.clone(),
                pkg_config::Error::ProbeFailure { name, command, output },
            );
            link_protobuf_default();
        },
        Err(e) => Err(e).unwrap(),
        Ok(_) => {},
    };
}

fn link_openssl_default() {
    link("static=crypto");
    link("static=ssl");
}

fn link_openssl() {
    let mut config = pkg_config::Config::new();
    // if std::env::var("CARGO_CFG_TARGET_OS").unwrap() != "macos" {
    //     config.statik(true);
    // }
    let result = config
        .atleast_version("1.1.1")
        .probe("openssl");
    match result {
        Err(pkg_config::Error::EnvNoPkgConfig(_)) => {
            println!(
                "cargo::warning=pkg-config was not found in PATH, using default lib link flags\
                 for openssl"
            );
            link_openssl_default();
        },
        Err(pkg_config::Error::ProbeFailure { name, command, output }) => {
            println!(
                "cargo::warning=library '{}' was not found by pkg-config; using default lib\
                 link flags\n{}",
                name.clone(),
                pkg_config::Error::ProbeFailure { name, command, output },
            );
            link_openssl_default();
        },
        Err(e) => Err(e).unwrap(),
        Ok(_) => {},
    }
}

// Copied from 'cc'; https://docs.rs/cc/latest/src/cc/lib.rs.html#3073
fn link_stdlib() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap();
    let target_vendor = std::env::var("CARGO_CFG_TARGET_VENDOR").unwrap();

    if &target_os == "windows" && &target_env == "msvc" {
        // No stdlib linking needed for MSVC
    } else if &target_vendor == "apple"
        || &target_os == "freebsd"
        || &target_os == "openbsd"
        || &target_os == "aix"
        || (&target_os == "linux" && &target_env == "ohos")
        || &target_os == "wasi"
    {
        link("c++");
    } else if &target_os == "android" {
        link("c++_shared");
    } else {
        link("stdc++");
    }
}

fn assert_cmd(cmd: &mut Command) {
    let status = cmd.status().unwrap();
    if !status.success() {
        panic!("Failed to exec cmd ({status}): {cmd:?}");
    }
}

fn git_clone(repo_url: &str, dst: &Path, commit: Option<&str>) {
    let exists = if dst.exists() {
        Command::new("git")
            .arg("-C").arg(dst)
            .arg("status")
            .status().unwrap()
            .success()
    } else {
        false
    };
    if !exists {
        // Repo not created yet, clone it
        assert_cmd(Command::new("git")
            .args(["clone", repo_url])
            .arg(dst.as_os_str()));
    }
    if let Some(commit) = commit {
        assert_cmd(Command::new("git")
            .arg("-C").arg(dst)
            .args(["checkout", commit]));
    }
    assert_cmd(Command::new("git")
        .arg("-C").arg(dst)
        .args(["submodule", "update", "--init", "--recursive"]));
}

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap();

    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());

    let gns_src_dir = manifest_dir.join("thirdparty").join("GameNetworkingSockets");

    /* start added */
    // Path to your shim header
    let shim_header = manifest_dir.join("c_shim").join("string_view_cstr_compat.h");

    // Where to put it inside the submodule so it’s visible to all source files
    // For example, copy it into the main include directory of GNS
    let dest_header = gns_src_dir.join("include").join("string_view_cstr_compat.h");

    // Create parent directories if they don’t exist
    fs::create_dir_all(dest_header.parent().unwrap()).unwrap();

    // Copy the file
    fs::copy(&shim_header, &dest_header).unwrap();
    /* end added */
    
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());

    println!("cargo::rerun-if-changed={}", manifest_dir.join("src").display());

    let gns_src_dir = manifest_dir.join("thirdparty").join("GameNetworkingSockets");
    println!("cargo::rerun-if-changed={}", gns_src_dir.join("src").display());
    println!("cargo::rerun-if-changed={}", gns_src_dir.join("include").display());
    println!("cargo::rerun-if-changed={}", gns_src_dir.join("cmake").display());
    println!("cargo::rerun-if-changed={}", gns_src_dir.join("CMakeLists.txt").display());

    let bindings = bindgen::Builder::default()
        .clang_arg(format!("-I{}", gns_src_dir.join("src").join("include").display()))
        .clang_arg(format!("-I{}", gns_src_dir.join("src").join("public").display()))
        .clang_arg(format!("-I{}", gns_src_dir.join("src").join("common").display()))
        .clang_arg("-DSTEAMNETWORKINGSOCKETS_STANDALONELIB")
        .header(gns_src_dir.join("include").join("steam").join("steamnetworkingsockets_flat.h").to_string_lossy())
        .header(gns_src_dir.join("include").join("steam").join("steamnetworkingsockets.h").to_string_lossy())
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

    bindings
        .write_to_file(out_dir.join("bindings.rs"))
        .expect("Couldn't write bindings!");

    if std::env::var("DOCS_RS").is_ok() {
        // We're building docs on docs.rs, and don't actually need to compile. Instead, just
        // generate bindings and let docs build from that.
        return
    }

    link_search("build/src");

    link("GameNetworkingSockets_s");

    let gns_src_dir = if &target_os == "windows" && &target_env == "msvc" {
        println!("cargo::rerun-if-changed={}", gns_src_dir.join("vcpkg.json").display());

        // TODO: We can't make changes outside of OUT_DIR, but we need to clone/install vcpkg,
        //  and _only_ on Windows. Upstream GameNetworkingSockets will only find vcpkg if it is
        //  cloned to the root of it's src/ dir; we may want to submit a patch that will let it
        //  find vcpkg elsewhere, to avoid cloning the src/ dir to OUT_DIR.
        let new_dir = out_dir.join("GNS");
        if new_dir.exists() {
            std::fs::remove_dir_all(&new_dir).unwrap();
        }
        dircpy::copy_dir(&gns_src_dir, &new_dir).unwrap();
        new_dir
    } else {
        gns_src_dir
    };

    let mut c = cmake::Config::new(&gns_src_dir);

    if &target_os == "windows" && &target_env == "msvc" {
        let vcpkg_root = gns_src_dir.join("vcpkg");
        let vcpkg_installed_root = out_dir.join("vcpkg").join("installed");

        println!("cargo::rerun-if-env-changed=GNS_VCPKG_BUILDTREES_ROOT");
        println!("cargo::rerun-if-env-changed=GNS_VCPKG_BUILDTREES_ROOT_NO_CHECK");

        let vcpkg_buildtrees_root = match std::env::var("GNS_VCPKG_BUILDTREES_ROOT") {
            Ok(v) => PathBuf::from(v),
            Err(_) => out_dir.join("vcpkg").join("buildtrees"),
        };
        let vcpkg_buildtrees_root_len = vcpkg_buildtrees_root.to_string_lossy().chars().count();
        if std::env::var("GNS_VCPKG_BUILDTREES_ROOT_NO_CHECK").unwrap_or("".to_owned()) != "true"
            && vcpkg_buildtrees_root_len > 100
        {
            panic!(
                "vcpkg 'buildtrees' root path ('{}') is too long ({} > 100)\
                \n\
                \nvcpkg 'buildtrees' root can use very long paths, which can exceed the\
                \ndefault Windows MAX_PATH of 256, and more importantly the CMake limit of 250.\
                \nA shorter path can be used by setting `GNS_VCPKG_BUILDTREES_ROOT` to a custom\
                \nlocation.\
                \n\
                \nAlternatively, this check can be bypassed by setting\
                \n`GNS_VCPKG_BUILDTREES_ROOT_NO_CHECK=true`, but this will likely result in build\
                \nfailures if you don't know exactly what you are doing.",
                vcpkg_buildtrees_root.display(),
                vcpkg_buildtrees_root_len,
            );
        }

        git_clone(
            "https://github.com/microsoft/vcpkg",
            &vcpkg_root,
            None,
        );
        Command::new(vcpkg_root.join("bootstrap-vcpkg.bat"))
            .status()
            .unwrap();
        let buildtrees_root_arg = format!("--x-buildtrees-root={}", vcpkg_buildtrees_root.display());
        assert_cmd(Command::new(vcpkg_root.join("vcpkg"))
            .arg("install")
            .arg(format!("--x-manifest-root={}", gns_src_dir.display()))
            .arg("--triplet=x64-windows-static-md-release")
            .arg(format!("--x-install-root={}", vcpkg_installed_root.display()))
            .arg(&buildtrees_root_arg));

        let protobuf = vcpkg_rs_mf::Config::new()
            .vcpkg_root(vcpkg_root.clone())
            .vcpkg_installed_root(vcpkg_installed_root.clone())
            .cargo_metadata(false)
            .copy_dlls(false)
            .target_triplet("x64-windows-static-md-release")
            .find_package("protobuf")
            .unwrap();

        for line in protobuf.cargo_metadata {
            // vcpkg crate doesn't have any method to specify the link metadata as static, so
            // manually do that here
            let line = line.replace(
                "cargo:rustc-link-lib=",
                "cargo:rustc-link-lib=static=",
            );
            println!("{}", line);
        }
        
        let profile = std::env::var("PROFILE").unwrap();
        if profile == "release" {
            link_search("build/src/Release");
        } else {
            link_search("build/src/Debug");
        }

        c.define("USE_CRYPTO", "BCrypt");
        c.define("VCPKG_TARGET_TRIPLET", "x64-windows-static-md-release");
        c.define("VCPKG_BUILD_TYPE", profile.clone());
        c.define("VCPKG_INSTALLED_DIR", &vcpkg_installed_root);
        c.define("VCPKG_INSTALL_OPTIONS", &buildtrees_root_arg);
    } else {
        link_protobuf();
        link_openssl();
    }
    link_stdlib();

    c.static_crt(false);
    // c.define("CMAKE_OSX_ARCHITECTURES", "arm64");
    // c.define("CMAKE_POSITION_INDEPENDENT_CODE", "ON");
    // c.cxxflag("-std=c++17");
    // let shim_path = manifest_dir.join("c_shim").join("string_view_cstr_compat.h");
    // c.define("CMAKE_CXX_FLAGS", format!("-include {}", shim_path.display()));
    let shim_path = gns_src_dir.join("include").join("string_view_cstr_compat.h");
    c.define("CMAKE_CXX_FLAGS", format!("-include {}", shim_path.display()));
    c.define("BUILD_STATIC_LIB", "ON");
    c.define("BUILD_SHARED_LIB", "OFF");
    c.define("OPENSSL_USE_STATIC_LIB", "OFF");
    c.define("Protobuf_USE_STATIC_LIBS", "OFF");
    c.build();
}
