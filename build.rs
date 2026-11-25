// SPDX-License-Identifier: MPL-2.0
use std::{env, fs, path::PathBuf};

// Our five icons deliberately use a small, checked SVG path subset. Compile
// these source assets once; runtime rendering needs neither XML nor an SVG DLL.
fn compile_icons() -> PathBuf {
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let names = ["clear", "copy", "chevron-down", "chevron-up", "backspace"];
    let mut header = String::from(
        "#pragma once\nstruct SvgCommand { char verb; float v[6]; };\nstruct SvgIcon { const SvgCommand* commands; unsigned count; };\n",
    );
    for (id, name) in names.iter().enumerate() {
        let file = format!("assets/icons/{name}.svg");
        println!("cargo:rerun-if-changed={file}");
        let svg = fs::read_to_string(&file).expect("SVG icon source");
        for attr in [
            "viewBox=\"0 0 24 24\"",
            "fill=\"none\"",
            "stroke=\"currentColor\"",
            "stroke-width=\"1.8\"",
            "stroke-linecap=\"round\"",
            "stroke-linejoin=\"round\"",
        ] {
            assert!(svg.contains(attr), "unsupported icon style: {file}");
        }
        header.push_str(&format!("static constexpr SvgCommand icon{id}[] = {{\n"));
        let paths: Vec<_> = svg.split("<path d=\"").skip(1).collect();
        assert_eq!(paths.len(), 1, "icons must have one path element");
        let mut parts = paths[0].split('"').next().unwrap().split_whitespace();
        while let Some(verb) = parts.next() {
            let count = match verb {
                "M" | "L" => 2,
                "C" => 6,
                "Z" => 0,
                _ => panic!("unsupported SVG path command: {verb}"),
            };
            let mut values = [0f32; 6];
            for value in values.iter_mut().take(count) {
                *value = parts
                    .next()
                    .expect("SVG coordinate")
                    .parse()
                    .expect("numeric SVG coordinate");
                assert!(
                    value.is_finite() && (0.0..=24.0).contains(value),
                    "icon outside viewBox"
                );
            }
            let coords = values
                .iter()
                .map(|n| format!("{n:.4}f"))
                .collect::<Vec<_>>()
                .join(",");
            header.push_str(&format!("{{'{verb}',{{{coords}}}}},\n"));
        }
        header.push_str("};\n");
    }
    header.push_str("static constexpr SvgIcon svgIcons[] = {\n");
    for id in 0..names.len() {
        header.push_str(&format!(
            "{{icon{id},sizeof(icon{id})/sizeof(SvgCommand)}},\n"
        ));
    }
    header.push_str("};\n");
    fs::write(out.join("svg_icons.h"), header).expect("compiled SVG header");
    out
}
fn main() {
    let icon_include = compile_icons();
    let icon =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("assets/branding/sylva.ico");
    let rc = icon_include.join("sylva.rc");
    let res = icon_include.join("sylva.res");
    fs::write(
        &rc,
        format!(
            "101 ICON \"{}\"\n",
            icon.display().to_string().replace('\\', "/")
        ),
    )
    .unwrap();
    let target = env::var("TARGET").unwrap();
    let compiler = cc::windows_registry::find_tool(&target, "cl.exe").expect("MSVC tools");
    let rc_exe = compiler
        .get_envs()
        .find(|(k, _)| k.eq_ignore_ascii_case("PATH"))
        .and_then(|(_, paths)| {
            env::split_paths(paths)
                .map(|p| p.join("rc.exe"))
                .find(|p| p.exists())
        })
        .expect("Windows SDK resource compiler");
    let status = std::process::Command::new(rc_exe)
        .args(["/nologo", "/fo"])
        .arg(&res)
        .arg(&rc)
        .status()
        .expect("compile application icon");
    assert!(status.success(), "resource compilation failed");
    println!("cargo:rustc-link-arg={}", res.display());
    println!("cargo:rerun-if-changed=assets/branding/sylva.ico");
    cc::Build::new()
        .cpp(true)
        .file("native/backend.cpp")
        .file("native/key_audio.cpp")
        .include("vendor")
        .include(icon_include)
        .flag("/std:c++17")
        .flag("/utf-8")
        .flag("/EHsc")
        .compile("native_backend");
    println!("cargo:rustc-link-search=native=vendor");
    for lib in [
        "openvr_api",
        "xaudio2",
        "d3d11",
        "dxgi",
        "d2d1",
        "dwrite",
        "user32",
        "gdi32",
        "ole32",
        "shell32",
        "imm32",
        "windowscodecs",
        "psapi",
    ] {
        println!("cargo:rustc-link-lib={lib}");
    }
    println!("cargo:rerun-if-changed=native/backend.cpp");
    println!("cargo:rerun-if-changed=native/key_audio.cpp");
    println!("cargo:rerun-if-changed=native/lifecycle.h");
    println!("cargo:rerun-if-changed=native/panel_geometry.h");
    println!("cargo:rerun-if-changed=native/key_feedback.h");
    println!("cargo:rerun-if-changed=native/graphics_metrics.h");
    println!("cargo:rerun-if-changed=native/frame_budget.h");
    println!("cargo:rerun-if-changed=native/software_panel.h");
    println!("cargo:rerun-if-changed=native/render_damage.h");
    println!("cargo:rerun-if-changed=native/raw_swap.h");
    println!("cargo:rerun-if-changed=vendor/openvr.h");
}
