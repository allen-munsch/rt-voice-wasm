use std::env;
use std::path::PathBuf;

fn main() {
    let target = env::var("TARGET").unwrap();
    let is_wasm = target == "wasm32-unknown-unknown";

    if is_wasm {
        println!("cargo:rustc-cfg=wasm_target");
        return;
    }

    let whisper_root = PathBuf::from("third_party/whisper.cpp");
    let ggml = whisper_root.join("ggml").join("src");

    let common_includes = [
        whisper_root.join("ggml").join("include"),
        whisper_root.join("include"),
        whisper_root.join("src"),
        ggml.clone(),
        ggml.join("ggml-cpu"),
    ];
    let common_defines = [
        ("GGML_USE_CPU", None),
        ("WHISPER_VERSION", Some("\"1.9.1\"")),
        ("GGML_SCHED_MAX_COPIES", Some("2")),
        ("GGML_VERSION", Some("\"1.9.1\"")),
        ("GGML_COMMIT", Some("\"unknown\"")),
        ("GGML_USE_CPU_REPACK", None),
    ];

    // --- C files (ggml-base + ggml-cpu) ---
    let mut c_build = cc::Build::new();
    c_build
        .flag_if_supported("-std=c11")
        .flag_if_supported("-pthread")
        .flag_if_supported("-march=native")
        .flag_if_supported("-ffast-math")
        .flag_if_supported("-fno-finite-math-only")
        .define("_GNU_SOURCE", None);
    for (name, val) in &common_defines {
        c_build.define(name, *val);
    }
    for inc in &common_includes {
        c_build.include(inc);
    }
    c_build
        .warnings(false)
        .file(ggml.join("ggml.c"))
        .file(ggml.join("ggml-alloc.c"))
        .file(ggml.join("ggml-quants.c"))
        .file(ggml.join("ggml-cpu").join("ggml-cpu.c"))
        .file(ggml.join("ggml-cpu").join("quants.c"))
        .file(
            ggml.join("ggml-cpu")
                .join("arch")
                .join("x86")
                .join("quants.c"),
        )
        .compile("whisper_c");

    // --- C++ files (whisper.cpp + ggml-base + ggml-cpu) ---
    let mut cpp_build = cc::Build::new();
    cpp_build
        .cpp(true)
        .flag_if_supported("-std=c++17")
        .flag_if_supported("-pthread")
        .flag_if_supported("-march=native")
        .flag_if_supported("-ffast-math")
        .flag_if_supported("-fno-finite-math-only");
    for (name, val) in &common_defines {
        cpp_build.define(name, *val);
    }
    for inc in &common_includes {
        cpp_build.include(inc);
    }
    cpp_build
        .warnings(false)
        .file(whisper_root.join("src").join("whisper.cpp"))
        .file(ggml.join("ggml.cpp"))
        .file(ggml.join("ggml-backend.cpp"))
        .file(ggml.join("ggml-backend-meta.cpp"))
        .file(ggml.join("ggml-backend-reg.cpp"))
        .file(ggml.join("ggml-opt.cpp"))
        .file(ggml.join("ggml-threading.cpp"))
        .file(ggml.join("gguf.cpp"))
        .file(ggml.join("ggml-cpu").join("ggml-cpu.cpp"))
        .file(ggml.join("ggml-cpu").join("repack.cpp"))
        .file(ggml.join("ggml-cpu").join("traits.cpp"))
        .file(ggml.join("ggml-cpu").join("vec.cpp"))
        .file(ggml.join("ggml-cpu").join("ops.cpp"))
        .file(ggml.join("ggml-cpu").join("binary-ops.cpp"))
        .file(ggml.join("ggml-cpu").join("unary-ops.cpp"))
        .file(ggml.join("ggml-cpu").join("amx").join("amx.cpp"))
        .file(ggml.join("ggml-cpu").join("amx").join("mmq.cpp"))
        .file(ggml.join("ggml-cpu").join("hbm.cpp"))
        .file(
            ggml.join("ggml-cpu")
                .join("arch")
                .join("x86")
                .join("repack.cpp"),
        )
        .compile("whisper_cpp");

    // --- Generate bindings ---
    let bindings = bindgen::Builder::default()
        .header(
            whisper_root
                .join("include")
                .join("whisper.h")
                .to_str()
                .unwrap(),
        )
        .clang_args(&[
            format!("-I{}", whisper_root.join("include").display()),
            format!("-I{}", whisper_root.join("ggml").join("include").display()),
        ])
        .allowlist_function("whisper_init_from_file_with_params")
        .allowlist_function("whisper_free")
        .allowlist_function("whisper_full_default_params")
        .allowlist_function("whisper_full_default_params_by_ref")
        .allowlist_function("whisper_free_params")
        .allowlist_function("whisper_full")
        .allowlist_function("whisper_full_n_segments")
        .allowlist_function("whisper_full_get_segment_text")
        .allowlist_function("whisper_full_get_segment_t0")
        .allowlist_function("whisper_full_get_segment_t1")
        .allowlist_function("whisper_context_default_params")
        .allowlist_function("whisper_print_timings")
        .allowlist_function("whisper_reset_timings")
        .allowlist_type("whisper_context_params")
        .allowlist_type("whisper_full_params")
        .generate()
        .expect("bindgen failed");

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap()).join("whisper_bindings.rs");
    bindings
        .write_to_file(&out_path)
        .expect("failed to write bindings");

    println!("cargo:rerun-if-changed=third_party/whisper.cpp/");
    println!("cargo:rustc-link-search=native=/usr/lib/gcc/x86_64-linux-gnu/13");
    println!("cargo:rustc-link-lib=static=stdc++");

    // Parakeet engine (shared library, only parakeet_capi_* symbols exported)
    let parakeet_lib_dir = PathBuf::from("build/parakeet");
    println!("cargo:rustc-link-search=native={}", parakeet_lib_dir.display());
    println!("cargo:rustc-link-lib=dylib=parakeet");
    println!("cargo:rustc-link-lib=dylib=ggml");
    println!("cargo:rustc-link-lib=dylib=ggml-base");
    println!("cargo:rustc-link-lib=dylib=ggml-cpu");
    // Set rpath so the binary finds the .so files at runtime
    println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN/../build/parakeet");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", parakeet_lib_dir.canonicalize().unwrap_or(parakeet_lib_dir).display());

    // Moonshine engine — loaded at runtime via dlopen (avoids link-time
    // conflicts with whisper.cpp's statically-linked ggml and ONNX Runtime).
    // The shared library is at build/moonshine/libmoonshine.so.
}
