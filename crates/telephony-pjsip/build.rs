use std::env;
use std::path::{Path, PathBuf};

const ADAPTER_SOURCE: &str = "../../native/pjsip-adapter/tsumuji_phone_pjsip.c";
const ADAPTER_INCLUDE: &str = "../../native/pjsip-adapter";

fn main() {
    println!("cargo:rerun-if-changed={ADAPTER_SOURCE}");
    println!("cargo:rerun-if-changed=../../native/pjsip-adapter/tsumuji_phone_pjsip.h");
    println!("cargo:rerun-if-env-changed=PJPROJECT_ROOT");

    if env::var_os("CARGO_FEATURE_NATIVE_PJSIP").is_none() {
        return;
    }

    match env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("windows") => build_windows(),
        Ok("linux") => build_unix(),
        Ok(target) => panic!("native-pjsip is not configured for target OS {target}"),
        Err(error) => panic!("Cargo did not provide CARGO_CFG_TARGET_OS: {error}"),
    }
}

fn adapter_build() -> cc::Build {
    let mut build = cc::Build::new();
    build
        .file(ADAPTER_SOURCE)
        .include(ADAPTER_INCLUDE)
        .warnings(true);
    build
}

fn build_unix() {
    let library = pkg_config::Config::new()
        .statik(true)
        .probe("libpjproject")
        .expect("native-pjsip requires libpjproject.pc; run scripts/build-pjsip-linux.sh first");

    let mut build = adapter_build();
    for include in &library.include_paths {
        build.include(include);
    }
    for (name, value) in &library.defines {
        build.define(name, value.as_deref());
    }
    build.compile("tsumuji_phone_pjsip");
}

fn build_windows() {
    let architecture = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    assert_eq!(
        architecture, "x86_64",
        "native-pjsip currently supports only x86_64-pc-windows-msvc on Windows"
    );
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    assert_eq!(
        target_env, "msvc",
        "native-pjsip requires the MSVC Rust target on Windows"
    );

    let project_root =
        env::var_os("PJPROJECT_ROOT").map_or_else(default_pjproject_root, PathBuf::from);
    let library_dir = project_root.join("lib");
    let library_name = "libpjproject-x86_64-x64-vc14-Release";
    let library_file = library_dir.join(format!("{library_name}.lib"));
    assert!(
        library_file.is_file(),
        "PJSIP library was not found at {}; run scripts/build-pjsip-windows.ps1 first",
        library_file.display()
    );

    let mut build = adapter_build();
    for include in [
        "pjlib/include",
        "pjlib-util/include",
        "pjnath/include",
        "pjmedia/include",
        "pjsip/include",
    ] {
        build.include(project_root.join(include));
    }
    build
        .define("WIN64", "1")
        .define("PJ_WIN64", "1")
        .define("PJ_M_X86_64", "1")
        .compile("tsumuji_phone_pjsip");

    println!("cargo:rustc-link-search=native={}", library_dir.display());
    println!("cargo:rustc-link-lib=static={library_name}");
    for library in [
        "iphlpapi", "dsound", "dxguid", "netapi32", "mswsock", "ws2_32", "odbc32", "odbccp32",
        "ole32", "user32", "gdi32", "advapi32", "winmm", "secur32",
    ] {
        println!("cargo:rustc-link-lib={library}");
    }
}

fn default_pjproject_root() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.join("../../vendor/pjproject")
}
