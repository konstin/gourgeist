use crate::bare::create_bare_venv;
#[cfg(feature = "install")]
use crate::packages::download_wheel_cached;
use camino::{Utf8Path, Utf8PathBuf};
use dirs::cache_dir;
#[cfg(feature = "install")]
use install_wheel_rs::install_wheel_in_venv;
use interpreter::InterpreterInfo;
use std::io;
use tempfile::PersistError;
use thiserror::Error;

pub use interpreter::get_interpreter_info;

mod bare;
mod interpreter;
#[cfg(feature = "install")]
mod packages;
#[cfg(not(feature = "install"))]
mod virtualenv_cache;

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    IO(#[from] io::Error),
    /// It's effectively an io error with extra info
    #[error(transparent)]
    Persist(#[from] PersistError),
    /// Adds url and target path to the io error
    #[error("Failed to download wheel from {url} to {path}")]
    WheelDownload {
        url: String,
        path: Utf8PathBuf,
        #[source]
        err: io::Error,
    },
    #[error("Failed to query python interpreter at {interpreter}")]
    PythonSubcommand {
        interpreter: Utf8PathBuf,
        #[source]
        err: io::Error,
    },
    #[cfg(feature = "install")]
    #[error("Failed to contact pypi")]
    MinReq(#[from] minreq::Error),
    #[cfg(feature = "install")]
    #[error("Failed to install {package}")]
    InstallWheel {
        package: String,
        #[source]
        err: install_wheel_rs::Error,
    },
}

pub(crate) fn crate_cache_dir() -> io::Result<Utf8PathBuf> {
    Ok(cache_dir()
        .and_then(|path| Utf8PathBuf::from_path_buf(path).ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "Couldn't detect cache dir"))?
        .join(env!("CARGO_PKG_NAME")))
}

/// Create a virtualenv and if not bare, install `wheel`, `pip` and `setuptools`.
pub fn create_venv(
    location: &Utf8Path,
    base_python: &Utf8Path,
    info: &InterpreterInfo,
    bare: bool,
) -> Result<(), Error> {
    let paths = create_bare_venv(location, base_python, info)?;

    if !bare {
        #[cfg(feature = "install")]
        {
            // TODO: Use the json api instead
            // TODO: Only check the json API so often (monthly? daily?)
            let packages = [
                ("pip-23.2.1-py3-none-any.whl", "https://files.pythonhosted.org/packages/50/c2/e06851e8cc28dcad7c155f4753da8833ac06a5c704c109313b8d5a62968a/pip-23.2.1-py3-none-any.whl"),
                ("setuptools-68.2.2-py3-none-any.whl", "https://files.pythonhosted.org/packages/bb/26/7945080113158354380a12ce26873dd6c1ebd88d47f5bc24e2c5bb38c16a/setuptools-68.2.2-py3-none-any.whl"),
                ("wheel-0.41.2-py3-none-any.whl", "https://files.pythonhosted.org/packages/b8/8b/31273bf66016be6ad22bb7345c37ff350276cfd46e389a0c2ac5da9d9073/wheel-0.41.2-py3-none-any.whl"),
            ];
            for (filename, url) in packages {
                let wheel_file = download_wheel_cached(filename, url)?;
                install_wheel_in_venv(
                    wheel_file.as_std_path(),
                    location.as_std_path(),
                    paths.interpreter.as_std_path(),
                    info.major,
                    info.minor,
                )
                .map_err(|err| Error::InstallWheel {
                    package: filename.to_string(),
                    err,
                })?;
            }
        }
        #[cfg(not(feature = "install"))]
        {
            virtualenv_cache::install_base_packages(
                &paths.bin,
                &paths.interpreter,
                &paths.site_packages,
            )?;
        }
    }

    Ok(())
}
