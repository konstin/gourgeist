use anyhow::{format_err, Context};
use camino::{Utf8Path, Utf8PathBuf};
use configparser::ini::Ini;
use dirs::data_dir;
use fs_err as fs;
use interpreter::InterpreterInfo;
use tracing::debug;

mod bare;
mod interpreter;

pub use interpreter::get_interpreter_info;

/// Create a virtualenv and if requested, install `wheel`, `pip` and `setuptools`.
pub fn create_venv(
    location: &Utf8PathBuf,
    base_python: &Utf8PathBuf,
    info: InterpreterInfo,
    bare: bool,
) -> anyhow::Result<()> {
    let paths = bare::create_bare_venv(location, base_python, info)?;

    if !bare {
        install_base_packages(&paths.bin, &paths.interpreter, &paths.site_packages)?;
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
        bare::copy_dir_all(&unpacked_wheel, site_packages.as_std_path())
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
            let launcher_script = bare::unix_launcher_script(venv_python, import_from, function);
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
