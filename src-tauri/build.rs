fn main() {
    tauri_build::build();
    embed_test_manifest();
}

/// `tauri_build` embeds the Windows manifest (common controls v6) into the app binary only. A test
/// binary that hosts Tauri's mock runtime (`tests/updater_feed.rs`) imports `TaskDialogIndirect`
/// through the same libraries and exits with `STATUS_ENTRYPOINT_NOT_FOUND` before `main` without
/// it, so the test targets get the same dependency declared here. Nothing changes for the app.
fn embed_test_manifest() {
    let windows_msvc = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "msvc");
    if !windows_msvc {
        return;
    }
    let manifest = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
</assembly>
"#;
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"))
        .join("test-common-controls.manifest");
    std::fs::write(&out, manifest).expect("write the test manifest");
    println!("cargo:rustc-link-arg-tests=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg-tests=/MANIFESTINPUT:{}",
        out.display()
    );
}
