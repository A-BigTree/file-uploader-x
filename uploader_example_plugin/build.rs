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

    // 必需资源：缺失即构建失败
    for file in ["meta.json", "config.json", "plugin.id"] {
        let src = PathBuf::from(&manifest_dir).join(file);
        let dst = target_dir.join(file);
        fs::copy(&src, &dst).unwrap_or_else(|e| panic!("复制失败 {:?} → {:?}: {}", src, dst, e));
        println!("cargo:rerun-if-changed={}", file);
    }

    // 可选资源：不存在则跳过（README 为可选，仅作路径引用）
    for file in ["README.md"] {
        let src = PathBuf::from(&manifest_dir).join(file);
        println!("cargo:rerun-if-changed={}", file);
        if !src.is_file() {
            eprintln!("info: optional resource not found, skip {}", src.display());
            continue;
        }
        let dst = target_dir.join(file);
        if let Err(e) = fs::copy(&src, &dst) {
            eprintln!("warn: 复制可选资源失败 {:?} → {:?}: {}", src, dst, e);
        }
    }

    println!("cargo:rerun-if-changed=build.rs");
}
