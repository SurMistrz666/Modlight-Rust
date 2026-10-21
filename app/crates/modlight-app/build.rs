//! Windows: what Explorer and the system read from Modlight.exe itself, which upstream's
//! project sets in its `.csproj` and `app.manifest`: the icon (upstream's
//! `Modlight_Icon.ico`, every frame as it is), the version information, and the
//! manifest (Windows 10 and 11, per-monitor DPI awareness v2, long paths, version 6 of
//! the common controls for the system dialogs, no elevation). Other systems get nothing
//! here; the macOS bundle is made by `scripts/build-app.sh`.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/upstream/icons/Modlight_Icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    #[cfg(windows)]
    windows::embed();
}

#[cfg(windows)]
mod windows {
    const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity type="win32" name="Modlight" version="{VERSION}"/>
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/>
    </dependentAssembly>
  </dependency>
  <compatibility xmlns="urn:schemas-microsoft-com:compatibility.v1">
    <application>
      <supportedOS Id="{8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a}"/>
    </application>
  </compatibility>
  <application xmlns="urn:schemas-microsoft-com:asm.v3">
    <windowsSettings>
      <dpiAware xmlns="http://schemas.microsoft.com/SMI/2005/WindowsSettings">true/pm</dpiAware>
      <dpiAwareness xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">PerMonitorV2</dpiAwareness>
      <longPathAware xmlns="http://schemas.microsoft.com/SMI/2016/WindowsSettings">true</longPathAware>
    </windowsSettings>
  </application>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>
"#;

    pub fn embed() {
        let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
        // The manifest's version is four numbers.
        let mut parts: Vec<u16> = version
            .split(['.', '-', '+'])
            .take(3)
            .map(|p| p.parse().unwrap_or(0))
            .collect();
        parts.resize(4, 0);
        let four = parts
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(".");
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("../../assets/upstream/icons/Modlight_Icon.ico")
            .set_manifest(&MANIFEST.replace("{VERSION}", &four))
            .set("ProductName", "Modlight")
            .set("FileDescription", "Modlight")
            .set("InternalName", "Modlight")
            .set("OriginalFilename", "Modlight.exe")
            .set("LegalCopyright", "GPL-3.0-or-later. Based on ShareX.")
            .set("ProductVersion", &version)
            .set("FileVersion", &version);
        if let Err(error) = resource.compile() {
            // Without the resource compiler the program still builds and runs; it only
            // lacks its icon and manifest. Say so loudly instead of failing the build.
            println!("cargo:warning=Modlight.exe resources were not embedded: {error}");
        }
    }
}
