use std::{env, path::PathBuf, process::Command};
fn main() {
    println!("cargo:rerun-if-changed=native/dashboard.m");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let object = out.join("dashboard.o");
    assert!(
        Command::new("clang")
            .args([
                "-fobjc-arc",
                "-O2",
                "-mmacosx-version-min=12.0",
                "-c",
                "native/dashboard.m",
                "-o"
            ])
            .arg(&object)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        Command::new("ar")
            .arg("rcs")
            .arg(out.join("libdashboard.a"))
            .arg(object)
            .status()
            .unwrap()
            .success()
    );
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=dashboard");
    println!("cargo:rustc-link-lib=framework=AppKit");
    println!("cargo:rustc-link-lib=framework=QuartzCore");
}
