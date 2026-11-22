use std::{
    error::Error,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};

use clap::Parser;
use clap_derive::{Parser, ValueEnum};
use tera::{Context, Tera};

/// Simple program to greet a person
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Dimension to use
    #[arg(short, long)]
    dim: Dimension,

    /// Features to enable
    #[arg(short, long)]
    feature_set: FeatureSet,
}

#[derive(ValueEnum, Debug, Clone, Copy)]
pub enum Dimension {
    Dim2,
    Dim3,
}

#[derive(ValueEnum, Default, Debug, Clone, Copy)]
pub enum FeatureSet {
    #[default]
    NonDeterministic,
    Deterministic,
    Simd,
    /// Multi-threaded (rayon over Web Workers) + SIMD. Needs nightly Rust (`-Z build-std`) at build
    /// time and a cross-origin-isolated page at runtime. Opt-in: not in `prepare_all_projects.sh`.
    Threads,
}

/// Values to use when creating the new build folder.
pub struct BuildValues {
    /// Only the number of dimensions, as sometimes it will be prefixed by "dim" and sometimes post-fixed by "d".
    pub dim: String,
    /// real name of the additional features to enable in the project
    pub feature_set: Vec<String>,
    pub target_dir: PathBuf,
    pub template_dir: PathBuf,
    pub additional_rust_flags: String,
    pub additional_wasm_opt_flags: Vec<String>,
    pub js_package_name: String,
    /// Extra arguments for `wasm-pack build` (anything after `--` goes to cargo).
    pub wasm_pack_args: String,
    /// Whether this is the multi-threaded variant (wasm-bindgen-rayon + a thread-pool `init()`).
    pub threads: bool,
}

impl BuildValues {
    pub fn new(args: Args) -> Self {
        let dim = match args.dim {
            Dimension::Dim2 => "2",
            Dimension::Dim3 => "3",
        };
        let feature_set = match args.feature_set {
            FeatureSet::NonDeterministic => vec![],
            FeatureSet::Deterministic => vec!["enhanced-determinism"],
            // SIMD is always on in rapier itself; the wasm build only differs by
            // the `+simd128` target feature and wasm-opt flags set below.
            FeatureSet::Simd => vec![],
            // `unsync-callbacks`: JS hooks and event handlers are not Send/Sync. Worker threads then
            // get a no-op hook, and pairs with user hooks are processed on the calling thread only.
            FeatureSet::Threads => vec!["parallel", "unsync-callbacks"],
        };
        let js_package_name = match args.feature_set {
            FeatureSet::NonDeterministic => format!("rapier{dim}d"),
            FeatureSet::Deterministic => format!("rapier{dim}d-deterministic"),
            FeatureSet::Simd => format!("rapier{dim}d-simd"),
            FeatureSet::Threads => format!("rapier{dim}d-threads"),
        };

        let root: PathBuf = env!("CARGO_MANIFEST_DIR").into();

        Self {
            dim: dim.to_string(),
            feature_set: feature_set.iter().map(|f| f.to_string()).collect(),
            template_dir: root.join("templates/").clone(),
            target_dir: root.parent().unwrap().join(&js_package_name).into(),
            additional_rust_flags: match args.feature_set {
                FeatureSet::Simd => "RUSTFLAGS='-C target-feature=+simd128'".to_string(),
                // Atomics + shared memory for threads. The release profile's `strip = true` would drop
                // the `target_features` section wasm-bindgen reads to emit threaded glue, so only
                // debuginfo is stripped here. LTO is enabled for this build.
                FeatureSet::Threads => concat!(
                    "RUSTUP_TOOLCHAIN=${RUSTUP_TOOLCHAIN:-nightly} ",
                    "CARGO_PROFILE_RELEASE_STRIP=debuginfo CARGO_PROFILE_RELEASE_LTO=fat ",
                    "RUSTFLAGS='-C target-feature=+atomics,+bulk-memory,+mutable-globals,+simd128 ",
                    "-C link-arg=--shared-memory -C link-arg=--max-memory=4294967296 ",
                    "-C link-arg=--import-memory -C link-arg=--export=__wasm_init_tls ",
                    "-C link-arg=--export=__tls_size -C link-arg=--export=__tls_align ",
                    "-C link-arg=--export=__tls_base'"
                )
                .to_string(),
                _ => "".to_string(),
            },
            additional_wasm_opt_flags: match args.feature_set {
                FeatureSet::Simd => vec!["--enable-simd".to_string()],
                FeatureSet::Threads => ["--enable-simd", "--enable-threads", "--enable-mutable-globals"]
                    .map(String::from)
                    .to_vec(),
                _ => vec![],
            },
            wasm_pack_args: match args.feature_set {
                // wasm-bindgen-rayon's workers re-import the module by URL: needs the `web` target. The crate
                // path (`.`) must be explicit, or wasm-pack 0.12 takes `-Z` after `--` as the path.
                FeatureSet::Threads => "--target web . -- -Z build-std=panic_abort,std".to_string(),
                _ => String::new(),
            },
            threads: matches!(args.feature_set, FeatureSet::Threads),
            js_package_name,
        }
    }
}

fn main() {
    let args = Args::parse();
    dbg!(&args);

    let build_values = BuildValues::new(args);
    copy_top_level_files_in_directory(&build_values.template_dir, &build_values.target_dir)
        .expect("Failed to copy directory");
    process_templates(&build_values).expect("Failed to process templates");
}

fn copy_top_level_files_in_directory(
    src: impl AsRef<Path>,
    dest: impl AsRef<std::ffi::OsStr>,
) -> std::io::Result<()> {
    let src = src.as_ref();
    let dest = Path::new(&dest);
    if dest.exists() {
        fs::remove_dir_all(&dest)?;
    }
    fs::create_dir_all(&dest)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let dest_path = dest.join(path.file_name().unwrap());

        if !path.is_dir() {
            fs::copy(&path, &dest_path)?;
        }
    }
    Ok(())
}

/// Process all tera templates in the target directory:
/// - Remove the extension
/// - Render the templates to
///
fn process_templates(build_values: &BuildValues) -> std::io::Result<()> {
    let target_dir = build_values.target_dir.clone();

    let mut context = Context::new();
    context.insert("dimension", &build_values.dim);
    context.insert("additional_features", &build_values.feature_set);
    context.insert("additional_rust_flags", &build_values.additional_rust_flags);
    context.insert(
        "additional_wasm_opt_flags",
        &build_values.additional_wasm_opt_flags,
    );
    context.insert("js_package_name", &build_values.js_package_name);
    context.insert("wasm_pack_args", &build_values.wasm_pack_args);
    context.insert("threads", &build_values.threads);

    let tera = match Tera::new(target_dir.join("**/*.tera").to_str().unwrap()) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Parsing error(s): {}", e);
            ::std::process::exit(1);
        }
    };
    dbg!(tera.templates.keys(), &context);

    for entry in fs::read_dir(target_dir)? {
        let entry = entry?;
        let path = entry.path();
        // For tera templates, remove extension.
        if path.extension() == Some(std::ffi::OsStr::new("tera")) {
            let mut i = path.iter();

            // Get path from target directory
            for _ in i
                .by_ref()
                .take_while(|c| *c != build_values.target_dir.file_name().unwrap())
            {}
            let path_template = i.as_path();
            match tera.render(path_template.to_str().unwrap(), &context) {
                Ok(s) => {
                    let old_path = path.clone();
                    let new_path = path.with_extension("");
                    let mut file = File::create(path.join(new_path))?;
                    file.write_all(s.as_bytes())?;
                    std::fs::remove_file(old_path)?;
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
                    let mut cause = e.source();
                    while let Some(e) = cause {
                        eprintln!("Reason: {}", e);
                        cause = e.source();
                    }
                }
            };
        }
    }

    Ok(())
}
