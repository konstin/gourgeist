use crate::{crate_cache_dir, Error};
use camino::{FromPathBufError, Utf8PathBuf};
use fs_err as fs;
use std::io;
use std::io::BufWriter;
use tempfile::NamedTempFile;
use tracing::info;

pub fn download_wheel_cached(filename: &str, url: &str) -> Result<Utf8PathBuf, Error> {
    let wheels_cache = crate_cache_dir()?.join("wheels");
    let cached_wheel = wheels_cache.join(filename);
    if cached_wheel.is_file() {
        info!("Using cached wheel at {cached_wheel}");
        return Ok(cached_wheel);
    }

    info!("Downloading wheel from {url} to {cached_wheel}");
    fs::create_dir_all(&wheels_cache)?;
    let mut tempfile = NamedTempFile::new_in(wheels_cache)?;
    let tempfile_path: Utf8PathBuf = tempfile
        .path()
        .to_path_buf()
        .try_into()
        .map_err(|err: FromPathBufError| err.into_io_error())?;
    let mut response = minreq::get(url).send_lazy()?;
    io::copy(&mut response, &mut BufWriter::new(&mut tempfile)).map_err(|err| {
        Error::WheelDownload {
            url: url.to_string(),
            path: tempfile_path.to_path_buf(),
            err,
        }
    })?;
    tempfile.persist(&cached_wheel)?;
    Ok(cached_wheel)
}
