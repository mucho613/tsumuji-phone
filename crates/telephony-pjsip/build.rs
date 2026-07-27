use std::env;
use std::path::{Path, PathBuf};

const ADAPTER_SOURCE: &str = "../../native/pjsip-adapter/tsumuji_phone_pjsip.c";
const ADAPTER_INCLUDE: &str = "../../native/pjsip-adapter";

fn main() {
    println!("cargo:rerun-if-changed={ADAPTER_SOURCE}");
    println!("cargo:rerun-if-changed=../../native/pjsip-adapter/tsumuji_phone_pjsip.h");
    println!("cargo:rerun-if-env-changed=PJPROJECT_ROOT");
    println!("cargo:rerun-if-env-changed=PJPROJECT_ANDROID_ROOT");
    println!("cargo:rerun-if-env-changed=ANDROID_NDK_ROOT");
    println!("cargo:rerun-if-env-changed=ANDROID_NDK_HOME");
    println!("cargo:rerun-if-env-changed=NDK_HOME");

    if env::var_os("CARGO_FEATURE_NATIVE_PJSIP").is_none() {
        return;
    }

    match env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("windows") => build_windows(),
        Ok("linux") => build_unix(),
        Ok("android") => build_android(),
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

fn build_android() {
    let architecture = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let (abi, library_suffix) = match architecture.as_str() {
        "aarch64" => ("arm64-v8a", "aarch64-unknown-linux-android"),
        "x86_64" => ("x86_64", "x86_64-pc-linux-android"),
        target => panic!("native-pjsip is not configured for Android architecture {target}"),
    };
    let project_root = env::var_os("PJPROJECT_ANDROID_ROOT").map_or_else(
        || {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/pjproject-android")
                .join(abi)
        },
        PathBuf::from,
    );
    let library_dir = project_root.join("lib");
    let runtime_dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/android-jni-libs")
        .join(abi);
    let expected_library = library_dir.join(format!("libpjsua-{library_suffix}.a"));
    assert!(
        expected_library.is_file(),
        "Android PJSIP library was not found at {}; run scripts/build-pjsip-android.ps1 first",
        expected_library.display()
    );
    assert!(
        runtime_dir.join("liboboe.so").is_file(),
        "Android runtime libraries were not found under {}; run scripts/build-pjsip-android.ps1 first",
        runtime_dir.display()
    );

    let mut build = adapter_build();
    configure_android_compiler(&mut build, architecture.as_str());
    build
        .include(project_root.join("include"))
        .define("PJ_AUTOCONF", "1")
        .define("PJ_IS_BIG_ENDIAN", "0")
        .define("PJ_IS_LITTLE_ENDIAN", "1")
        .compile("tsumuji_phone_pjsip");

    println!("cargo:rustc-link-search=native={}", library_dir.display());
    println!("cargo:rustc-link-search=native={}", runtime_dir.display());
    for library in [
        "pjsua",
        "pjsip-ua",
        "pjsip-simple",
        "pjsip",
        "pjmedia-codec",
        "pjmedia-videodev",
        "pjmedia-audiodev",
        "pjmedia",
        "pjnath",
        "pjlib-util",
        "srtp",
        "resample",
        "speex",
        "webrtc",
        "pj",
    ] {
        println!("cargo:rustc-link-lib=static={library}-{library_suffix}");
    }
    for library in [
        "oboe",
        "c++_shared",
        "mediandk",
        "OpenSLES",
        "log",
        "android",
        "atomic",
        "m",
        "dl",
    ] {
        println!("cargo:rustc-link-lib=dylib={library}");
    }
}

fn configure_android_compiler(build: &mut cc::Build, architecture: &str) {
    if env::consts::OS != "windows" {
        return;
    }
    let ndk_root = android_ndk_root();
    let toolchain = ndk_root.join("toolchains/llvm/prebuilt/windows-x86_64");
    let target = match architecture {
        "aarch64" => "aarch64-linux-android31",
        "x86_64" => "x86_64-linux-android31",
        _ => unreachable!("Android architecture was validated before compiler setup"),
    };
    build
        .compiler(toolchain.join("bin/clang.exe"))
        .archiver(toolchain.join("bin/llvm-ar.exe"))
        .flag(format!("--target={target}"))
        .flag(format!("--sysroot={}", toolchain.join("sysroot").display()))
        .define("ANDROID", None);
}

fn android_ndk_root() -> PathBuf {
    for variable in ["ANDROID_NDK_ROOT", "ANDROID_NDK_HOME", "NDK_HOME"] {
        if let Some(path) = env::var_os(variable) {
            let root = PathBuf::from(path);
            if root.join("toolchains/llvm").is_dir() {
                return root;
            }
        }
    }
    let sdk_root = env::var_os("ANDROID_HOME")
        .or_else(|| env::var_os("ANDROID_SDK_ROOT"))
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .map(|path| path.join("Android/Sdk"))
        })
        .expect("Android SDK location is unavailable");
    let ndk_parent = sdk_root.join("ndk");
    let mut candidates = std::fs::read_dir(&ndk_parent)
        .unwrap_or_else(|error| {
            panic!(
                "Android NDK directory could not be read at {}: {error}",
                ndk_parent.display()
            )
        })
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.join("toolchains/llvm").is_dir())
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.pop().unwrap_or_else(|| {
        panic!(
            "Android NDK was not found under {}; install it from Android Studio",
            ndk_parent.display()
        )
    })
}

fn default_pjproject_root() -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.join("../../vendor/pjproject")
}
