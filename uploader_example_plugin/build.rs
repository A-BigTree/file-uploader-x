use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let out_dir = std::env::var("OUT_DIR").unwrap();

    let target_dir = PathBuf::from(&out_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();

    eprintln!("target_dir = {}", target_dir.display());

    for file in ["meta.json", "config.json", "plugin.id"] {
        let src = PathBuf::from(&manifest_dir).join(file);
        let dst = target_dir.join(file);
        fs::copy(&src, &dst)
            .unwrap_or_else(|e| panic!("复制失败 {:?} → {:?}: {}", src, dst, e));
        println!("cargo:rerun-if-changed={}", file);
    }

    println!("cargo:rerun-if-changed=build.rs");
}
