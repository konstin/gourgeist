use anyhow::{bail, format_err, Context};
use camino::{Utf8Path, Utf8PathBuf};
use clap::Parser;
use configparser::ini::Ini;
use dirs::{cache_dir, data_dir};
use fs_err as fs;
use fs_err::os::unix::fs::symlink;
use fs_err::File;
use serde::{Deserialize, Serialize};
use std::io;
use std::io::{BufReader, BufWriter, Write};
use std::path::Path;
use std::process::{Command, ExitCode, Stdio};
use std::time::Instant;
use tracing::{debug, error, info, warn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

/// The bash activate scripts with the venv dependent paths patches out
const ACTIVATE_TEMPLATES: &[(&str, &str)] = &[
    ("activate", include_str!("activator/activate")),
    ("activate.csh", include_str!("activator/activate.csh")),
    ("activate.fish", include_str!("activator/activate.fish")),
    ("activate.nu", include_str!("activator/activate.nu")),
    ("activate.ps1", include_str!("activator/activate.ps1")),
    (
        "activate_this.py",
        include_str!("activator/activate_this.py"),
    ),
];
const QUERY_PYTHON: &str = include_str!("query_python.py");
const VIRTUALENV_PATCH: &str = include_str!("_virtualenv.py");

#[derive(Parser, Debug)]
struct Cli {
    path: Option<Utf8PathBuf>,
    #[clap(short, long)]
    python: Option<Utf8PathBuf>,
    #[clap(long)]
    bare: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct InterpreterInfo {
    base_exec_prefix: String,
    base_prefix: String,
    major: u8,
    minor: u8,
    python_version: String,
}

/// [`symlink`] wrapper
fn symlink_with_context(
    src: impl AsRef<Utf8Path>,
    dst: impl AsRef<Utf8Path>,
) -> anyhow::Result<()> {
    symlink(src.as_ref(), dst.as_ref()).with_context(|| {
        format!(
            "Failed to create symlink. original: {}, link: {}",
            src.as_ref(),
            dst.as_ref().join("python")
        )
    })
}

/// Very basic `.cfg` file format writer.
fn write_cfg(f: &mut impl Write, data: &[(&str, String); 8]) -> io::Result<()> {
    for (key, value) in data {
        writeln!(f, "{} = {}", key, value)?;
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct CacheEntry {
    interpreter: Utf8PathBuf,
    modified: u128,
    interpreter_info: InterpreterInfo,
}

fn get_interpreter_info(interpreter: &Utf8PathBuf) -> anyhow::Result<InterpreterInfo> {
    let cache_dir = cache_dir()
        .and_then(|path| Utf8PathBuf::from_path_buf(path).ok())
        .context("Couldn't detect cache dir")?
        .join(env!("CARGO_PKG_NAME"))
        .join("interpreter_info");

    let index = seahash::hash(interpreter.as_str().as_bytes());
    let cache_file = cache_dir.join(index.to_string()).with_extension("json");

    let modified = fs::metadata(interpreter)?
        .modified()?
        .elapsed()?
        .as_millis();

    if cache_file.exists() {
        let cache_entry: Result<CacheEntry, String> = File::open(&cache_file)
            .map_err(|err| err.to_string())
            .and_then(|cache_reader| {
                serde_json::from_reader(BufReader::new(cache_reader)).map_err(|err| err.to_string())
            });
        match cache_entry {
            Ok(cache_entry) => {
                debug!("Using cache entry {cache_file}");
                if modified == cache_entry.modified && interpreter == &cache_entry.interpreter {
                    return Ok(cache_entry.interpreter_info);
                }
            }
            Err(cache_err) => {
                debug!("Removing broken cache entry {cache_file} ({cache_err})");
                if let Err(remove_err) = fs::remove_file(&cache_file) {
                    warn!("Failed to remove broken cache file at {cache_file}: {remove_err} (original error: {cache_err})")
                }
            }
        }
    }

    let interpreter_info = query_interpreter(interpreter)?;
    fs::create_dir_all(&cache_dir).context("Failed to create cache dir")?;
    let cache_entry = CacheEntry {
        interpreter: interpreter.clone(),
        modified,
        interpreter_info: interpreter_info.clone(),
    };
    let mut cache_writer = File::create(&cache_file).context("Failed to create cache file")?;
    serde_json::to_writer(&mut cache_writer, &cache_entry).context("Failed to write cache file")?;

    Ok(interpreter_info)
}

/// Runs a python script that returns the relevant info about the interpreter as json
fn query_interpreter(interpreter: &Utf8PathBuf) -> anyhow::Result<InterpreterInfo> {
    let mut child = Command::new(interpreter)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(QUERY_PYTHON.as_bytes())
            .context("Failed to pass script to python")?;
    }
    let output = child.wait_with_output()?;
    let stdout = String::from_utf8(output.stdout).unwrap_or_else(|err| {
        // At this point, there was most likely an error caused by a non-utf8 character, so we're in
        // an ugly case but still very much want to give the user a chance
        error!(
            "The stdout of the failed call of the call to {} contains non-utf8 characters",
            interpreter
        );
        String::from_utf8_lossy(err.as_bytes()).to_string()
    });
    let stderr = String::from_utf8(output.stderr).unwrap_or_else(|err| {
        error!(
            "The stderr of the failed call of the call to {} contains non-utf8 characters",
            interpreter
        );
        String::from_utf8_lossy(err.as_bytes()).to_string()
    });
    // stderr isn't technically a criterion for success, but i don't know of any cases where there
    // should be stderr output and if there is, we want to know
    if !output.status.success() || !stderr.trim().is_empty() {
        bail!(
            "Querying python at {} failed with status {}:\n--- stdout:\n{}\n--- stderr:\n{}",
            interpreter,
            output.status,
            stdout.trim(),
            stderr.trim()
        )
    }
    let data = serde_json::from_str::<InterpreterInfo>(&stdout).with_context(||
        format!(
            "Querying python at {} did not return the expected data:\n--- stdout:\n{}\n--- stderr:\n{}",
            interpreter,
            stdout.trim(),
            stderr.trim()
        )
    )?;
    Ok(data)
}

/// https://stackoverflow.com/a/65192210/3549270
fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> io::Result<()> {
    fs::create_dir_all(&dst)?;
    for entry in fs::read_dir(src.as_ref())? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(entry.path(), dst.as_ref().join(entry.file_name()))?;
        } else {
            fs::copy(entry.path(), dst.as_ref().join(entry.file_name()))?;
        }
    }
    Ok(())
}

/// Template for the console scripts in the `bin` directory
fn unix_launcher_script(python: &Utf8Path, import_from: &str, function: &str) -> String {
    format!(
        r#"#!{python}
    # -*- coding: utf-8 -*-
import re
import sys
from {import_from} import {function}
if __name__ == '__main__':
    sys.argv[0] = re.sub(r'(-script\.pyw|\.exe)?$', '', sys.argv[0])
    sys.exit({function}())
"#,
        python = python,
        import_from = import_from,
        function = function
    )
}

/// Write all the files that belong to a venv
fn create_venv(
    location: &Utf8PathBuf,
    base_python: &Utf8PathBuf,
    info: InterpreterInfo,
    bare: bool,
) -> anyhow::Result<()> {
    if location.exists() {
        fs::remove_dir_all(location)?;
    }
    fs::create_dir_all(location)?;
    let bin_dir = {
        #[cfg(unix)]
        {
            location.join("bin")
        }
        #[cfg(windows)]
        {
            location.join("bin")
        }
        #[cfg(not(any(unix, windows)))]
        {
            compile_error!("only unix (like mac and linux) and windows are supported")
        }
    };

    fs::create_dir(&bin_dir)?;
    let venv_python = bin_dir.join("python");
    symlink_with_context(base_python, &venv_python)?;
    symlink_with_context("python", bin_dir.join(format!("python{}", info.major)))?;
    symlink_with_context(
        "python",
        bin_dir.join(format!("python{}.{}", info.major, info.minor)),
    )?;
    for (name, template) in ACTIVATE_TEMPLATES {
        let activator = template
            .replace("{{ VIRTUAL_ENV_DIR }}", location.as_str())
            .replace(
                "{{ RELATIVE_SITE_PACKAGES }}",
                &format!("../lib/python{}.{}/site-packages", info.major, info.minor),
            );
        fs::write(bin_dir.join(name), activator)?;
    }
    fs::write(location.join(".gitignore"), "*")?;

    // pyvenv.cfg
    let pyvenv_cfg_data = &[
        (
            "home",
            base_python
                .parent()
                .context("The python interpreter needs to have a parent directory")?
                .to_string(),
        ),
        ("implementation", "CPython".to_string()),
        ("version_info", info.python_version),
        ("virtualenv-rs", env!("CARGO_PKG_VERSION").to_string()),
        // I wouldn't allow this option anyway
        ("include-system-site-packages", "false".to_string()),
        ("base-prefix", info.base_prefix),
        ("base-exec-prefix", info.base_exec_prefix),
        ("base-executable", base_python.to_string()),
    ];
    let mut pyvenv_cfg = BufWriter::new(File::create(location.join("pyvenv.cfg"))?);
    write_cfg(&mut pyvenv_cfg, pyvenv_cfg_data)?;
    drop(pyvenv_cfg);

    // TODO: This is different on windows
    let site_packages = location
        .join("lib")
        .join(format!("python{}.{}", info.major, info.minor))
        .join("site-packages");
    fs::create_dir_all(&site_packages)?;
    // Install _virtualenv.py patch.
    // Frankly no idea what that does, i just copied it from virtualenv knowing that
    // distutils/setuptools will have their cursed reasons
    fs::write(site_packages.join("_virtualenv.py"), VIRTUALENV_PATCH)?;
    fs::write(site_packages.join("_virtualenv.pth"), "import _virtualenv")?;

    if !bare {
        install_base_packages(&bin_dir, &venv_python, &site_packages)?;
    }

    Ok(())
}

/// Install wheel, pip and setuptools from the cache
fn install_base_packages(
    bin_dir: &Utf8Path,
    venv_python: &Utf8PathBuf,
    site_packages: &Utf8Path,
) -> anyhow::Result<()> {
    // Install packages
    // TODO: Implement our own logic:
    //  * Our own cache and logic to detect whether a wheel is present
    //  * Check if the version is recent (e.g. update if older than 1 month)
    //  * Query pypi API if no, parse versions (pep440) and their metadata
    //  * Download compatible wheel (py3-none-any should do)
    //  * Install into the cache directory
    let prefix = "virtualenv/wheel/3.11/image/1/CopyPipInstall/";
    let wheel_tag = "py3-none-any";
    let packages = &[
        ("pip", "23.2.1"),
        ("setuptools", "68.2.0"),
        ("wheel", "0.41.2"),
    ];
    let virtualenv_data_dir = data_dir()
        .and_then(|path| Utf8PathBuf::from_path_buf(path).ok())
        .context("Couldn't get data dir")?;
    for (name, version) in packages {
        // TODO: acquire lock
        let unpacked_wheel = virtualenv_data_dir
            .join(prefix)
            .join(format!("{name}-{version}-{wheel_tag}"));
        debug!("Installing {name} by copying from {unpacked_wheel}");
        copy_dir_all(&unpacked_wheel, site_packages.as_std_path())
            .with_context(|| format!("Failed to copy {unpacked_wheel} to {site_packages}"))?;

        // Generate launcher
        // virtualenv for some reason creates extra entrypoints that we don't
        // https://github.com/pypa/virtualenv/blob/025e96fbad37f85617364002ae2a0064b09fc984/src/virtualenv/seed/embed/via_app_data/pip_install/base.py#L74-L95
        let ini_text = fs::read_to_string(
            site_packages
                .join(format!("{name}-{version}.dist-info"))
                .join("entry_points.txt"),
        )
        .with_context(|| format!("{name} should have an entry_points.txt"))?;
        let entry_points_mapping = Ini::new_cs()
            .read(ini_text)
            .map_err(|err| format_err!("{name} entry_points.txt is invalid: {}", err))?;
        for (key, value) in entry_points_mapping
            .get("console_scripts")
            .cloned()
            .unwrap_or_default()
        {
            let (import_from, function) = value
                .as_ref()
                .and_then(|value| value.split_once(':'))
                .ok_or_else(|| {
                    format_err!("{name} entry_points.txt {key} has an invalid value {value:?}")
                })?;
            let launcher = bin_dir.join(key);
            let launcher_script = unix_launcher_script(venv_python, import_from, function);
            fs::write(&launcher, launcher_script)?;
            // We need to make the launcher executable
            #[cfg(target_family = "unix")]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(launcher, std::fs::Permissions::from_mode(0o755))?;
            }
        }
    }
    Ok(())
}

fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let location = cli.path.unwrap_or(Utf8PathBuf::from(".venv-rs"));
    let base_python = cli
        .python
        .unwrap_or(Utf8PathBuf::from("/home/konsti/.local/bin/python3.11"));
    let data = get_interpreter_info(&base_python)?;

    create_venv(&location, &base_python, data, cli.bare)?;

    Ok(())
}

fn main() -> ExitCode {
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env())
        .init();

    let start = Instant::now();
    let result = run();
    info!("Took {}ms", start.elapsed().as_millis());
    if let Err(err) = result {
        eprintln!("💥 virtualenv creator failed");
        for err in err.chain() {
            eprintln!("  Caused by: {}", err);
        }
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
