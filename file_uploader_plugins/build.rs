use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let out_dir = env::var("OUT_DIR").unwrap();

    // OUT_DIR = target/debug/build/plugin_auth_v1-xxx/out
    //                                                 ↑ out_dir
    //                                   plugin_auth_v1-xxx  .parent()×1
    //                             build                      .parent()×2
    //                       debug                            .parent()×3  ← 目标
    let target_dir = PathBuf::from(&out_dir)
        .parent()
        .unwrap() // plugin_auth_v1-xxx
        .parent()
        .unwrap() // build
        .parent()
        .unwrap() // target/debug  ← .dylib 所在目录
        .to_path_buf();

    // 验证路径
    eprintln!("target_dir = {}", target_dir.display());

    let src = PathBuf::from(&manifest_dir).join("pre_upload_plugins.json");
    let dst = target_dir.join("pre_upload_plugins.json");

    fs::copy(&src, &dst).unwrap_or_else(|e| panic!("复制失败 {:?} → {:?}: {}", src, dst, e));

    let src = PathBuf::from(&manifest_dir).join("post_upload_plugins.json");
    let dst = target_dir.join("post_upload_plugins.json");

    fs::copy(&src, &dst).unwrap_or_else(|e| panic!("复制失败 {:?} → {:?}: {}", src, dst, e));

    let src = PathBuf::from(&manifest_dir).join("upload_plugins.json");
    let dst = target_dir.join("upload_plugins.json");

    fs::copy(&src, &dst).unwrap_or_else(|e| panic!("复制失败 {:?} → {:?}: {}", src, dst, e));

    println!("cargo:rerun-if-changed=pre_upload_plugins.json");
    println!("cargo:rerun-if-changed=build.rs");
}
