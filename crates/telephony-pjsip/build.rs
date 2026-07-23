fn main() {
    println!("cargo:rerun-if-changed=../../native/pjsip-adapter/hikari_phone_pjsip.c");
    println!("cargo:rerun-if-changed=../../native/pjsip-adapter/hikari_phone_pjsip.h");

    if std::env::var_os("CARGO_FEATURE_NATIVE_PJSIP").is_none() {
        return;
    }

    let library = pkg_config::Config::new()
        .statik(true)
        .probe("libpjproject")
        .expect("native-pjsip requires libpjproject.pc; run scripts/build-pjsip-linux.sh first");

    let mut build = cc::Build::new();
    build
        .file("../../native/pjsip-adapter/hikari_phone_pjsip.c")
        .include("../../native/pjsip-adapter")
        .warnings(true);
    for include in &library.include_paths {
        build.include(include);
    }
    for (name, value) in &library.defines {
        build.define(name, value.as_deref());
    }
    build.compile("hikari_phone_pjsip");
}
