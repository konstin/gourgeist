//! Create a bare virtualenv without any packages install

use crate::interpreter::InterpreterInfo;
use anyhow::Context;
use camino::{Utf8Path, Utf8PathBuf};
use fs_err as fs;
use fs_err::os::unix::fs::symlink;
use fs_err::File;
use std::io;
use std::io::{BufWriter, Write};
use std::path::Path;

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
const VIRTUALENV_PATCH: &str = include_str!("_virtualenv.py");

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

/// https://stackoverflow.com/a/65192210/3549270
pub fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> io::Result<()> {
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
pub fn unix_launcher_script(python: &Utf8Path, import_from: &str, function: &str) -> String {
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

/// Absolute paths of the virtualenv
pub struct VenvPaths {
    /// The location of the virtualenv, e.g. `.venv`
    pub root: Utf8PathBuf,
    /// The python interpreter.rs inside the virtualenv, on unix `.venv/bin/python`
    pub interpreter: Utf8PathBuf,
    /// The directory with the scripts, on unix `.venv/bin`
    pub bin: Utf8PathBuf,
    /// The site-packages directory where all the packages are installed to, on unix
    /// and python 3.11 `.venv/lib/python3.11/site-packages`
    pub site_packages: Utf8PathBuf,
}

/// Write all the files that belong to a venv without any packages installed.
pub fn create_bare_venv(
    location: &Utf8PathBuf,
    base_python: &Utf8PathBuf,
    info: InterpreterInfo,
) -> anyhow::Result<VenvPaths> {
    // TODO: I bet on windows we'll have to strip the prefix again
    let location = location
        .canonicalize_utf8()
        .context("Failed to canonicalize virtualenv path, does it exist?")?;
    if location.exists() {
        fs::remove_dir_all(&location)?;
    }
    fs::create_dir_all(&location)?;
    let bin_dir = {
        #[cfg(unix)]
        {
            location.join("bin")
        }
        #[cfg(windows)]
        {
            location.join("Bin")
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
                .context("The python interpreter.rs needs to have a parent directory")?
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

    Ok(VenvPaths {
        root: location.to_path_buf(),
        interpreter: venv_python,
        bin: bin_dir,
        site_packages,
    })
}
