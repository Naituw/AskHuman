#[path = "src/gsettings_schema.rs"]
mod gsettings_schema;

fn main() {
    println!("cargo:rerun-if-changed=src/gsettings_schema.rs");
    println!("cargo:rerun-if-env-changed=GSETTINGS_SCHEMAS_PATH");
    let schema_dirs = if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        gsettings_schema::schema_data_dirs(std::env::var_os("GSETTINGS_SCHEMAS_PATH"))
            .unwrap_or_else(|error| panic!("invalid GSETTINGS_SCHEMAS_PATH: {error}"))
    } else {
        String::new()
    };
    println!("cargo:rustc-env=ASKHUMAN_COMPILED_GSETTINGS_DATA_DIRS={schema_dirs}");

    // WebView2 invokes commands while a deep native callback stack is active. Reserve enough
    // virtual stack for the Windows UI thread so a future large command cannot regress into a
    // process-wide stack overflow. Pages are committed on demand, not eagerly allocated.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rustc-link-arg=/STACK:8388608");
    }

    // macOS 原生 QuickLook 预览面板（QLPreviewPanel）位于 Quartz 框架（QuickLookUI）。
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-link-lib=framework=Quartz");
        // 裸二进制无 .app bundle，把 Info.plist 以 __TEXT,__info_plist 段嵌入 Mach-O，
        // 让 TCC 能读到麦克风/语音识别的用法串（否则访问麦克风会被系统直接终止）。
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let plist = std::path::Path::new(&manifest_dir).join("Info.plist");
        println!("cargo:rerun-if-changed={}", plist.display());
        println!(
            "cargo:rustc-link-arg=-Wl,-sectcreate,__TEXT,__info_plist,{}",
            plist.display()
        );

        // Compile the system speech and attachment preview bridges into the existing static library.
        build_swift_bridges(&manifest_dir);
    }
    tauri_build::build()
}

/// Compile the Swift bridges for the Cargo target architecture and emit link arguments.
fn build_swift_bridges(manifest_dir: &str) {
    use std::path::Path;
    use std::process::Command;

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let swift_dir = Path::new(manifest_dir).join("swift");
    println!("cargo:rerun-if-changed={}", swift_dir.display());

    // 收集 swift 源文件。
    let mut sources: Vec<String> = Vec::new();
    let mut collect = |dir: &Path| {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().and_then(|s| s.to_str()) == Some("swift") {
                    println!("cargo:rerun-if-changed={}", p.display());
                    sources.push(p.to_string_lossy().into_owned());
                }
            }
        }
    };
    collect(&swift_dir);
    // Prototype-only verification helpers (window capture, burst recording, forced-activation test
    // hooks) are compiled solely for the collapse prototype; the product binary never contains them.
    if std::env::var_os("CARGO_FEATURE_COLLAPSE_PROTOTYPE").is_some() {
        collect(&swift_dir.join("prototype"));
    }
    if sources.is_empty() {
        panic!(
            "build_swift_bridges: 未找到 swift 源文件于 {}",
            swift_dir.display()
        );
    }
    sources.sort();

    // cargo 架构 → Apple triple 架构名。
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86_64") => "x86_64",
        other => panic!("build_swift_bridges: 不支持的架构 {:?}", other),
    };
    // 部署目标必须用「app 的最低系统版本」而非 26.0：
    // 用 26 SDK 编译，但 SpeechAnalyzer 等 26 专有 API 经 `if #available(macOS 26,*)` 弱链接，
    // 从而二进制仍可在 < macOS 26 启动（届时隐藏语音功能），不会被抬高 minos 而拒绝加载。
    let deploy = "11.0";
    let triple = format!("{arch}-apple-macosx{deploy}");

    let sdk_path = run_capture("xcrun", &["--sdk", "macosx", "--show-sdk-path"]);
    let swiftc = run_capture("xcrun", &["--sdk", "macosx", "-f", "swiftc"]);

    // Swift 运行时/标准库静态归档所在目录：<toolchain>/usr/lib/swift/macosx
    // 由 swiftc 路径 .../usr/bin/swiftc 推导出 .../usr/lib/swift/macosx
    let toolchain_usr = Path::new(swiftc.trim())
        .parent() // bin
        .and_then(|p| p.parent()) // usr
        .expect("无法定位 Swift 工具链 usr 目录")
        .to_path_buf();
    let swift_lib_dir = toolchain_usr.join("lib/swift/macosx");

    let lib_path = Path::new(&out_dir).join("libahspeech.a");
    let module_cache = swift_module_cache_dir(Path::new(&out_dir));
    std::fs::create_dir_all(&module_cache).expect("无法创建 Swift module cache");

    // 编译为静态库。-static -emit-library 产出 .a；autolink 指令(框架/swiftCore)内嵌到对象中。
    let mut cmd = Command::new(swiftc.trim());
    cmd.args([
        "-target",
        &triple,
        "-sdk",
        sdk_path.trim(),
        // 沿用 demo 的宽松并发检查（桥/引擎用 @unchecked Sendable + Task 捕获）。
        "-swift-version",
        "5",
        "-O",
        "-wmo",
        "-parse-as-library",
        "-module-name",
        "ahspeech",
        // Keep Clang/Swift modules under `target/` instead of ~/.cache/clang: builds launched from
        // sandboxed agents may not write the latter. See `swift_module_cache_dir` for the layout.
        "-module-cache-path",
        module_cache
            .to_str()
            .expect("Swift module cache 路径不是 UTF-8"),
        "-emit-library",
        "-static",
        "-o",
    ]);
    cmd.arg(&lib_path);
    for s in &sources {
        cmd.arg(s);
    }
    let status = cmd.status().expect("执行 swiftc 失败");
    if !status.success() {
        panic!("swiftc 编译 Swift 语音桥失败 (target={triple})");
    }

    // 链接：force_load 保证 @objc 类被注册不被裁剪；提供 Swift 运行时 search path；
    // rpath 让运行期从系统 /usr/lib/swift 解析 Swift 运行时(单文件分发)。
    println!("cargo:rustc-link-search=native={out_dir}");
    println!("cargo:rustc-link-search=native={}", swift_lib_dir.display());
    println!(
        "cargo:rustc-link-arg=-Wl,-force_load,{}",
        lib_path.display()
    );
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
}

/// Directory for the Clang/Swift module cache (`.pcm` files, about 240 MiB).
///
/// `OUT_DIR` is `<target>[/<triple>]/<profile>/build/<package>-<hash>/out`. Cargo creates a fresh
/// one for every profile and again whenever the build script's inputs change (dependencies,
/// features, toolchain), so a cache kept there is rebuilt cold (about 25 s) each time and
/// duplicated per profile. Use a single `swift-module-cache` next to the profile directories
/// instead: still under `target/` (writable for sandboxed agents, removed by a full `cargo clean`),
/// shared by every profile of that target directory, and untouched by `cargo clean -p`/`--profile`.
///
/// Clang module files embed their absolute cache path and refuse to load once the directory is
/// copied or moved, so anything that clones a `target/` somewhere else must drop this directory
/// first (scripts/seed-worktree-target.sh does); the next build recreates it.
fn swift_module_cache_dir(out_dir: &std::path::Path) -> std::path::PathBuf {
    // Ancestors of OUT_DIR: out, <package>-<hash>, build, <profile>, <target>[/<triple>].
    let mut ancestors = out_dir.ancestors();
    let build_dir = ancestors.nth(2);
    let target_root = ancestors.nth(1);
    match (build_dir, target_root) {
        (Some(build), Some(root))
            if build.file_name().and_then(|name| name.to_str()) == Some("build") =>
        {
            root.join("swift-module-cache")
        }
        // Unexpected layout (custom build directory, future Cargo change): stay private to this build.
        _ => out_dir.join("swift-module-cache"),
    }
}

fn run_capture(cmd: &str, args: &[&str]) -> String {
    let out = std::process::Command::new(cmd)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("执行 {cmd} {args:?} 失败: {e}"));
    if !out.status.success() {
        panic!(
            "{cmd} {args:?} 返回非零: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}
