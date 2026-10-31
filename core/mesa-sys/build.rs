//! Builds libmesa from third_party/libmesa as libmesa's build.zig does,
//! and generates its bindings with bindgen.
//!
//! The flags are build.zig's, at libmesa 2cbb076. A release build
//! matches zig's ReleaseFast (-O2 -DNDEBUG), a debug build its Debug
//! (-O0) less the UBSan runtime. urcrypt and SQLite are -O2 always, as
//! build.zig has them. For wasm32-wasip1 the sources are copied and
//! wasm/libmesa-wasm32.diff is applied to the copy, as wasm/build.sh
//! does, and the compiler is the wasi-sdk's clang at WASI_SDK_PATH with
//! zig's wasm CPU (lime1).
//!
//! It also writes suite.env to OUT_DIR: the compiler and flags, so
//! suites.sh builds libmesa's own tests exactly as this built libmesa.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// build.zig's C flags, before the OS define.
const FLAGS: &[&str] = &[
    "-std=gnu23",
    "-Wall",
    "-Werror",
    "-Wimplicit-fallthrough",
    "-Wno-deprecated-non-prototype",
    "-Wno-gnu",
    "-Wno-unused-variable",
    "-Wno-unused-function",
    "-Wno-unused-but-set-variable",
    "-fno-omit-frame-pointer",
    "-fms-extensions",
    "-DU3_OS_ENDIAN_little=1",
    "-DU3_OS_PROF=1",
];

/// UBSan as zig 0.15.2's Debug has it. The first three flags are the
/// ones its --verbose-cc shows. The fourth is ours: zig's own UBSan
/// runtime stops at the first fault, and -fno-sanitize-recover makes
/// clang's do the same.
const UBSAN: [&str; 4] = [
    "-fsanitize=undefined",
    "-fno-sanitize=vptr",
    "-fno-sanitize=function",
    "-fno-sanitize-recover=undefined",
];

/// libmesa's core sources, as build.zig's addCore lists them.
const CORE: &[&str] = &[
    "util.c",
    "ship.c",
    "noun.c",
    "crypto.c",
    "sha.c",
    "ident.c",
    "json.c",
    "azimuth.c",
    "knot.c",
    "aes_siv.c",
    "store_mem.c",
    "store_sqlite.c",
    "gall.c",
    "eng/eng_util.c",
    "eng/eng_space.c",
    "eng/eng_serve.c",
    "eng/eng_peer.c",
    "eng/eng_fetch.c",
    "eng/eng_flow.c",
    "eng/eng_sys.c",
    "eng/eng_ames.c",
    "eng/engine.c",
    "wire/bitset.c",
    "wire/lss.c",
    "wire/pact.c",
];

/// The public headers the bindings cover. host.h is libmesa-host's
/// (libuv), which mesa-sys does not build.
const HEADERS: &[&str] = &[
    "noun.h",
    "log.h",
    "ident.h",
    "azimuth.h",
    "store.h",
    "engine.h",
    "gall.h",
];

struct Target {
    wasm: bool,
    arch: String,
    os: String,
    sysroot: Option<PathBuf>,
    release: bool,
}

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let repo = manifest.join("../..");
    let upstream = repo.join("third_party/libmesa");
    let diff = repo.join("wasm/libmesa-wasm32.diff");
    let deps = env::var_os("MESA_SYS_DEPS")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest.join("deps"));

    for p in [
        &upstream.join("src"),
        &upstream.join("include"),
        &upstream.join("vendor"),
        &diff,
    ] {
        println!("cargo:rerun-if-changed={}", p.display());
    }
    println!("cargo:rerun-if-changed={}", deps.display());
    for v in [
        "MESA_SYS_DEPS",
        "WASI_SDK_PATH",
        "CC",
        "CFLAGS",
        "MESA_SYS_UBSAN",
    ] {
        println!("cargo:rerun-if-env-changed={v}");
    }

    if !upstream.join("src/noun.c").exists() {
        panic!("third_party/libmesa is empty: run git submodule update --init");
    }
    // suites.sh's zig run and wasm/build.sh build the submodule's HEAD
    // (git archive); this build reads its files. A local change would
    // make them build different sources, so it stops the build.
    let dirty = Command::new("git")
        .arg("-C")
        .arg(&upstream)
        .args(["status", "--porcelain", "--untracked-files=no"])
        .output();
    match dirty {
        Ok(o) if o.status.success() => assert!(
            o.stdout.is_empty(),
            "third_party/libmesa has local changes: build it clean, at its pinned commit"
        ),
        // a source tree with no git, such as a crate package, is as shipped
        _ => println!("cargo:warning=could not check third_party/libmesa is clean"),
    }
    let urcrypt = deps.join("urcrypt");
    let sqlite = deps.join("sqlite");
    if !urcrypt.join("urcrypt/urcrypt.h").exists() || !sqlite.join("sqlite3.c").exists() {
        panic!(
            "urcrypt or SQLite is missing from {}: run core/mesa-sys/fetch-deps.sh",
            deps.display()
        );
    }

    let t = target();

    // the wasm build compiles a patched copy, so the submodule stays clean
    let root = if t.wasm {
        let copy = out.join("libmesa");
        if copy.exists() {
            fs::remove_dir_all(&copy).unwrap();
        }
        for d in ["include", "src", "vendor"] {
            copy_dir(&upstream.join(d), &copy.join(d));
        }
        fs::copy(upstream.join("build.zig"), copy.join("build.zig")).unwrap();
        let ok = Command::new("patch")
            .args(["-s", "-p1", "-d"])
            .arg(&copy)
            .arg("-i")
            .arg(&diff)
            .status()
            .expect("run patch");
        assert!(ok.success(), "wasm/libmesa-wasm32.diff did not apply");
        copy
    } else {
        upstream.clone()
    };

    let os_define = match (t.wasm, t.os.as_str()) {
        // libmesa-wasm32.diff: wasi builds as linux
        (true, _) | (false, "linux") => "-DU3_OS_linux=1",
        (false, "macos") => "-DU3_OS_osx=1",
        (false, "windows") => "-DU3_OS_windows=1",
        (false, os) => panic!("libmesa does not build for {os}"),
    };

    let includes: Vec<PathBuf> = vec![
        sqlite.clone(),
        root.join("include"),
        root.join("src"),
        root.join("src/wire"),
        root.join("src/eng"),
        root.join("vendor"),
        root.join("vendor/murmur3"),
        root.join("vendor/verstable"),
        urcrypt.join("urcrypt"),
        urcrypt.join("blake3"),
        urcrypt.join("ed25519/src"),
    ];

    let mut flags: Vec<String> = FLAGS.iter().map(|s| s.to_string()).collect();
    flags.push(os_define.into());

    // libmesa: listed first, so the linker reads it before the libraries
    // it calls into
    let mut core = base(&t, true);
    core.opt_level(if t.release { 2 } else { 0 });
    for i in &includes {
        core.include(i);
    }
    for f in &flags {
        core.flag(f);
    }
    if t.release {
        core.define("NDEBUG", None);
    }
    // zig's Debug compiles libmesa (not urcrypt or SQLite) with UBSan.
    // MESA_SYS_UBSAN=1 does the same, for suites.sh: the Rust tests do
    // not link the sanitizer's runtime.
    if env::var_os("MESA_SYS_UBSAN").is_some_and(|v| v == "1") {
        for f in &UBSAN {
            core.flag(f);
        }
    }
    core.file(root.join("vendor/c3/defs.c"));
    for f in CORE {
        core.file(root.join("src").join(f));
    }
    // murmur3 has flags of its own: -O3 -fno-sanitize=all
    let mut murmur = base(&t, false);
    murmur.opt_level(3).flag("-fno-sanitize=all");
    if t.release {
        murmur.define("NDEBUG", None);
    }
    murmur.file(root.join("vendor/murmur3/murmur3.c"));
    for o in murmur.compile_intermediates() {
        core.object(o);
    }
    core.compile("mesa");

    // urcrypt's subset, as build.zig's libcrypt
    let mut crypt = base(&t, false);
    crypt.opt_level(2);
    for f in [
        "-fno-omit-frame-pointer",
        "-fno-sanitize=all",
        "-Wno-unused-result",
    ] {
        crypt.flag(f);
    }
    if t.release {
        crypt.define("NDEBUG", None);
    }
    for d in [
        "urcrypt",
        "blake3",
        "ed25519/src",
        "ge-additions",
        "monocypher",
    ] {
        crypt.include(urcrypt.join(d));
    }
    for f in [
        "blake3.c",
        "chacha.c",
        "ed25519.c",
        "ge_additions.c",
        "util.c",
    ] {
        crypt.file(urcrypt.join("urcrypt").join(f));
    }
    for f in ["blake3.c", "blake3_dispatch.c", "blake3_portable.c"] {
        crypt.file(urcrypt.join("blake3").join(f));
    }
    if t.arch == "x86_64" && t.os != "windows" && !t.wasm {
        for f in [
            "blake3_sse2_x86-64_unix.S",
            "blake3_sse41_x86-64_unix.S",
            "blake3_avx2_x86-64_unix.S",
            "blake3_avx512_x86-64_unix.S",
        ] {
            crypt.file(urcrypt.join("blake3").join(f));
        }
    } else if t.arch == "aarch64" {
        // build.zig compiles this one file with its own flags
        let mut neon = base(&t, false);
        neon.opt_level(2).define("BLAKE3_USE_NEON", "1");
        if t.release {
            neon.define("NDEBUG", None);
        }
        neon.file(urcrypt.join("blake3/blake3_neon.c"));
        for o in neon.compile_intermediates() {
            crypt.object(o);
        }
    } else {
        for m in [
            "BLAKE3_NO_SSE2",
            "BLAKE3_NO_SSE41",
            "BLAKE3_NO_AVX2",
            "BLAKE3_NO_AVX512",
        ] {
            crypt.define(m, "1");
        }
    }
    for f in [
        "add_scalar.c",
        "fe.c",
        "ge.c",
        "keypair.c",
        "key_exchange.c",
        "sc.c",
        "seed.c",
        "sha512.c",
        "sign.c",
        "verify.c",
    ] {
        crypt.file(urcrypt.join("ed25519/src").join(f));
    }
    crypt.file(urcrypt.join("ge-additions/ge-additions.c"));
    crypt.file(urcrypt.join("monocypher/monocypher.c"));
    crypt.compile("mesa-urcrypt");

    // SQLite, libmesa's default store
    let mut sql = base(&t, false);
    sql.opt_level(2).flag("-fno-sanitize=all");
    if t.release {
        sql.define("NDEBUG", None);
    }
    for d in [
        "SQLITE_DQS=0",
        "SQLITE_DEFAULT_MEMSTATUS=0",
        "SQLITE_OMIT_LOAD_EXTENSION",
        "SQLITE_OMIT_DEPRECATED",
        "SQLITE_LIKE_DOESNT_MATCH_BLOBS",
    ] {
        let (k, v) = d.split_once('=').map_or((d, None), |(k, v)| (k, Some(v)));
        sql.define(k, v);
    }
    sql.file(sqlite.join("sqlite3.c"));
    sql.compile("mesa-sqlite");

    if !t.wasm && t.os != "windows" {
        println!("cargo:rustc-link-lib=m");
        println!("cargo:rustc-link-lib=pthread");
    }

    bindings(&t, &root, &out);
    suite_env(&t, &core, &root, &out);

    // for crates that link against libmesa's headers
    println!("cargo:include={}", root.join("include").display());
    println!("cargo:root={}", root.display());
}

fn target() -> Target {
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    let wasm = arch == "wasm32" && os == "wasi";
    let release = env::var("OPT_LEVEL").is_ok_and(|o| o != "0");
    let sysroot = wasm.then(|| wasi_sdk().join("share/wasi-sysroot"));
    Target {
        wasm,
        arch,
        os,
        sysroot,
        release,
    }
}

fn wasi_sdk() -> PathBuf {
    let p = env::var_os("WASI_SDK_PATH")
        .map(PathBuf::from)
        .expect("a wasm32-wasip1 build needs WASI_SDK_PATH, the wasi-sdk's directory");
    assert!(
        p.join("bin/clang").exists(),
        "no clang in WASI_SDK_PATH {}",
        p.display()
    );
    p
}

/// A cc::Build with the compiler and target flags. For libmesa's own
/// sources (`own`) cc adds only -Wall, which build.zig has too, so its
/// -Werror holds. For urcrypt and SQLite, which build.zig compiles with
/// no warning flags, cc adds -w: a warning there is upstream's. Neither
/// changes the code generated.
fn base(t: &Target, own: bool) -> cc::Build {
    let mut b = cc::Build::new();
    b.warnings(own).extra_warnings(false);
    if t.wasm {
        let sdk = wasi_sdk();
        b.compiler(sdk.join("bin/clang"));
        b.archiver(sdk.join("bin/llvm-ar"));
        b.flag(format!(
            "--sysroot={}",
            t.sysroot.as_ref().unwrap().display()
        ));
        // zig's wasm32 CPU, lime1: bulk-memory-opt (memcpy and memset)
        // but not the rest of bulk memory, and no reference types or SIMD
        b.flag("-mcpu=lime1");
    } else if t.arch == "x86_64" {
        // baseline x86-64, whatever the build machine: what ships runs
        // on any x86-64. suites.sh compares with zig at the same CPU.
        b.flag("-march=x86-64");
    }
    if !t.wasm
        && env::var_os("CC").is_none()
        && env::var_os(format!(
            "CC_{}",
            env::var("TARGET").unwrap().replace('-', "_")
        ))
        .is_none()
    {
        // build.zig compiles with clang (zig cc)
        b.compiler("clang");
    }
    b
}

/// The type of every integer macro in the headers. bindgen alone gives
/// MESA_NONE, which is ((mesa_noun)UINT64_MAX), as `i32 = -1`, a wrong
/// constant that would compile. So each one is named here with its C
/// type, and a macro not named stops the build until it is.
#[derive(Debug)]
struct MacroTypes;

impl bindgen::callbacks::ParseCallbacks for MacroTypes {
    fn int_macro(&self, name: &str, value: i64) -> Option<bindgen::callbacks::IntKind> {
        use bindgen::callbacks::IntKind;
        match name {
            // UINT64_MAX, which clang hands over as -1
            "MESA_NONE" | "MESA_RIFT_UNKNOWN" => {
                assert_eq!(value, -1, "{name} is no longer UINT64_MAX");
                Some(IntKind::U64)
            }
            // include guards, and the system headers' macros
            _ if !name.starts_with("MESA_") || name.ends_with("_H") => None,
            _ => panic!("give the integer macro {name} ({value}) its C type in build.rs"),
        }
    }
}

fn bindings(t: &Target, root: &Path, out: &Path) {
    let mut wrapper = String::new();
    for h in HEADERS {
        wrapper.push_str(&format!("#include \"mesa/{h}\"\n"));
    }
    let header = out.join("wrapper.h");
    fs::write(&header, wrapper).unwrap();

    let mut b = bindgen::Builder::default()
        .header(header.to_str().unwrap())
        .clang_arg(format!("-I{}", root.join("include").display()))
        .use_core()
        .allowlist_function("mesa_.*")
        .allowlist_type("mesa_.*")
        .allowlist_var("MESA_.*")
        // MESA_NONE is a cast, ((mesa_noun)UINT64_MAX): clang evaluates it
        .clang_macro_fallback()
        .parse_callbacks(Box::new(MacroTypes))
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()));
    if t.wasm {
        b = b
            .clang_arg("--target=wasm32-wasip1")
            // clang's wasm32 hides symbols by default, and bindgen skips
            // every hidden function
            .clang_arg("-fvisibility=default")
            .clang_arg(format!(
                "--sysroot={}",
                t.sysroot.as_ref().unwrap().display()
            ));
    }
    b.generate()
        .expect("bindgen on libmesa's headers")
        .write_to_file(out.join("bindings.rs"))
        .unwrap();
}

/// suite.env: how this build compiled libmesa, for suites.sh.
fn suite_env(t: &Target, core: &cc::Build, root: &Path, out: &Path) {
    // the compiler's arguments are already every flag, define and
    // include libmesa was compiled with
    let cc = core.get_compiler();
    let cflags: Vec<String> = cc
        .args()
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let q = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
    let env = format!(
        "MESA_CC={}\nMESA_CFLAGS={}\nMESA_LIBDIR={}\nMESA_ROOT={}\nMESA_WASM={}\nMESA_RELEASE={}\n",
        q(&cc.path().to_string_lossy()),
        q(&cflags.join(" ")),
        q(&out.to_string_lossy()),
        q(&root.to_string_lossy()),
        t.wasm as u8,
        t.release as u8,
    );
    fs::write(out.join("suite.env"), env).unwrap();
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let p = e.path();
        if e.file_type().unwrap().is_dir() {
            copy_dir(&p, &to.join(e.file_name()));
        } else {
            fs::copy(&p, to.join(e.file_name())).unwrap();
        }
    }
}
