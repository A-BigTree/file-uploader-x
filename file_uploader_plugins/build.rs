use std::fs;
use std::path::Path;

fn copy_dir_recursive(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if path.is_dir() {
            copy_dir_recursive(&path, &dst_path);
        } else {
            fs::copy(&path, &dst_path)
                .unwrap_or_else(|e| panic!("复制失败 {:?} → {:?}: {}", path, dst_path, e));
        }
    }
}

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let out_dir = std::env::var("OUT_DIR").unwrap();

    // OUT_DIR 的 parent×3 = target/<profile>
    let target_dir = std::path::PathBuf::from(&out_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();

    eprintln!("target_dir = {}", target_dir.display());

    let src = std::path::PathBuf::from(&manifest_dir).join("resources");
    let dst = target_dir.join("resources");

    println!("cargo:rustc-env=FILE_UPLOADER_RESOURCES_DIR={}", dst.display());

    if src.exists() {
        copy_dir_recursive(&src, &dst);
    } else {
        eprintln!("warn: resources dir not found at {}", src.display());
    }

    println!("cargo:rerun-if-changed=resources");
    println!("cargo:rerun-if-changed=build.rs");
}
