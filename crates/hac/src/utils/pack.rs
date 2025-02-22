use common::defines::DEFAULT_PRODKEYS_PATH;
use config::Config;
use eyre::{eyre, Result};
use fs_err as fs;
use std::path::Path;
use tracing::debug;

use crate::{
    backend::{Backend, BackendKind},
    utils::hacpack_cleanup_install,
    vfs::{
        nacp::{get_nacp_file, NacpData},
        nca::{self, Nca},
        nsp::Nsp,
        PROGRAMID_LEN,
    },
};

/// Pack romfs/exefs back to NSP.
pub fn pack_fs_data<N, E, R, O>(
    control_path: N,
    mut program_id: String,
    romfs_dir: R,
    exefs_dir: E,
    outdir: O,
    cfg: &Config,
) -> Result<(Nsp, NacpData)>
where
    N: AsRef<Path>,
    E: AsRef<Path>,
    R: AsRef<Path>,
    O: AsRef<Path>,
{
    let curr_dir = std::env::current_dir()?;
    let _hacpack_cleanup_bind = hacpack_cleanup_install!(curr_dir);

    #[cfg(all(
        target_arch = "x86_64",
        any(target_os = "windows", target_os = "linux")
    ))]
    let readers = vec![
        Backend::try_new(BackendKind::Hactoolnet)?,
        Backend::try_new(BackendKind::Hac2l)?,
    ];
    #[cfg(feature = "android-proot")]
    let readers = vec![Backend::try_new(BackendKind::Hac2l)?];
    #[cfg(not(feature = "android-proot"))]
    let nca_extractor = Backend::try_new(BackendKind::from(cfg.nca_extractor))?;
    #[cfg(feature = "android-proot")]
    let nca_extractor = Backend::try_new(BackendKind::Hac2l)?;
    let packer = Backend::try_new(BackendKind::Hacpack)?;

    // Validating NCA as Control Type
    let control_nca = readers
        .iter()
        .map(|reader| Nca::try_new(reader, control_path.as_ref()).ok())
        .find(|nca| matches!(nca, Some(nca) if nca.content_type == nca::ContentType::Control))
        .flatten()
        .ok_or_else(|| {
            eyre!(
                "'{}' is not a Control Type NCA",
                control_path.as_ref().display()
            )
        })?;

    program_id.truncate(PROGRAMID_LEN as _);
    debug!(?program_id, "Selected ProgramID for packing");

    // Getting Nacp data
    let control_romfs_dir = tempfile::tempdir_in(&cfg.temp_dir)?;
    control_nca.unpack_romfs(&nca_extractor, control_romfs_dir.path())?;
    let nacp_data =
        NacpData::try_new(get_nacp_file(control_romfs_dir.path()).ok_or_else(|| {
            eyre!("Couldn't find NACP file, should be due to improper extraction")
        })?)?;

    let temp_dir = tempfile::tempdir_in(&cfg.temp_dir)?;

    // !Packing fs files to NCA
    let patched_nca = Nca::pack_program(
        readers.iter(),
        &packer,
        &program_id,
        DEFAULT_PRODKEYS_PATH.as_path(),
        romfs_dir.as_ref(),
        exefs_dir.as_ref(),
        temp_dir.path(),
    )?;

    // !Generating Meta NCA
    Nca::create_meta(
        &packer,
        &program_id,
        DEFAULT_PRODKEYS_PATH.as_path(),
        &patched_nca,
        &control_nca,
        temp_dir.path(),
        &cfg.temp_dir,
    )?;

    // !Copying Control NCA
    let control_filename = control_nca
        .path
        .file_name()
        .expect("File should've a filename");
    fs::copy(&control_nca.path, temp_dir.path().join(control_filename))?;

    // !Packing NCAs to NSP
    let packed_nsp = Nsp::pack(
        &packer,
        &program_id,
        DEFAULT_PRODKEYS_PATH.as_path(),
        temp_dir.path(),
        outdir.as_ref(),
    )?;

    Ok((packed_nsp, nacp_data))
}
