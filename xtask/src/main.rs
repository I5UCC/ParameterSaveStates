//! `cargo xtask package [--openvr-lib PATH] [--no-build]`
//!
//! Builds the release binary and assembles `dist/<platform>/` plus an
//! archive (`.zip` on Windows, `.tar.gz` on Linux).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

const EXE: &str = if cfg!(windows) {
    "ParameterSaveStates.exe"
} else {
    "ParameterSaveStates"
};
const OPENVR_LIB: &str = if cfg!(windows) {
    "openvr_api.dll"
} else {
    "libopenvr_api.so"
};
const PLATFORM: &str = if cfg!(windows) { "windows" } else { "linux" };

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives in the workspace")
        .to_path_buf()
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("package") => package(&args[1..]),
        _ => {
            eprintln!("usage: cargo xtask package [--openvr-lib PATH] [--no-build]");
            std::process::exit(2);
        }
    }
}

fn package(args: &[String]) -> Result<()> {
    let mut openvr_lib: Option<PathBuf> = None;
    let mut build = true;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--openvr-lib" => {
                openvr_lib = Some(PathBuf::from(
                    it.next().context("--openvr-lib needs a path")?,
                ))
            }
            "--no-build" => build = false,
            other => bail!("unknown argument '{other}'"),
        }
    }

    let root = workspace_root();
    if build {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let status = Command::new(cargo)
            .current_dir(&root)
            .args(["build", "--release", "--package", "psstates-app"])
            .status()
            .context("failed to run cargo")?;
        if !status.success() {
            bail!("cargo build failed");
        }
    }

    let exe = root.join("target").join("release").join(EXE);
    if !exe.is_file() {
        bail!("{} not found", exe.display());
    }

    let dist = root.join("dist");
    let stage = dist.join(PLATFORM);
    if stage.exists() {
        fs::remove_dir_all(&stage)?;
    }
    fs::create_dir_all(&stage)?;

    let copy = |src: PathBuf, name: &str| -> Result<()> {
        fs::copy(&src, stage.join(name))
            .with_context(|| format!("copying {}", src.display()))
            .map(|_| ())
    };
    copy(exe, EXE)?;
    copy(
        root.join("packaging").join("app.vrmanifest"),
        "app.vrmanifest",
    )?;
    copy(root.join("assets").join("icon.png"), "icon.png")?;
    copy(root.parent().unwrap().join("LICENSE"), "LICENSE")?;
    match openvr_lib {
        Some(lib) => copy(lib, OPENVR_LIB)?,
        None => {
            eprintln!("warning: no --openvr-lib given, the SteamVR overlay will be unavailable")
        }
    }
    if cfg!(not(windows)) {
        let linux = root.join("packaging").join("linux");
        copy(
            linux.join("parametersavestates.desktop"),
            "parametersavestates.desktop",
        )?;
        copy(linux.join("install.sh"), "install.sh")?;
    }

    let archive = if cfg!(windows) {
        let path = dist.join(format!("ParameterSaveStates-{PLATFORM}.zip"));
        write_zip(&stage, &path)?;
        path
    } else {
        let path = dist.join(format!(
            "ParameterSaveStates-{PLATFORM}-{}.tar.gz",
            std::env::consts::ARCH
        ));
        write_tar_gz(&stage, &path)?;
        path
    };
    println!("created {}", archive.display());
    Ok(())
}

fn files_in(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<_, _>>()?;
    files.sort();
    Ok(files)
}

fn is_executable(name: &str) -> bool {
    name == EXE || name.ends_with(".sh") || name.ends_with(".so")
}

fn write_zip(stage: &Path, dest: &Path) -> Result<()> {
    let mut zip = zip::ZipWriter::new(fs::File::create(dest)?);
    for path in files_in(stage)? {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        zip.start_file(name, options)?;
        zip.write_all(&fs::read(&path)?)?;
    }
    zip.finish()?;
    Ok(())
}

fn write_tar_gz(stage: &Path, dest: &Path) -> Result<()> {
    let gz = flate2::write::GzEncoder::new(fs::File::create(dest)?, flate2::Compression::best());
    let mut tar = tar::Builder::new(gz);
    for path in files_in(stage)? {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let data = fs::read(&path)?;
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(if is_executable(&name) { 0o755 } else { 0o644 });
        header.set_mtime(0);
        header.set_cksum();
        tar.append_data(
            &mut header,
            format!("ParameterSaveStates/{name}"),
            data.as_slice(),
        )?;
    }
    tar.into_inner()?.finish()?;
    Ok(())
}
