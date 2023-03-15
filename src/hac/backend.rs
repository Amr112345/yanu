use eyre::Result;
use once_cell::sync::Lazy;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::Command;
#[cfg(unix)]
use tempfile::tempdir;

#[cfg(target_family = "unix")]
use crate::utils::set_executable_bit;
use crate::{cache::Cache, defines};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Hacpack,
    Hactool,
    #[cfg(all(
        target_arch = "x86_64",
        any(target_os = "windows", target_os = "linux")
    ))]
    Hactoolnet,
    #[cfg(all(
        target_arch = "x86_64",
        any(target_os = "windows", target_os = "linux")
    ))]
    Hac2l,
}

impl BackendKind {
    fn to_filename(&self) -> String {
        #[cfg(unix)]
        {
            format!("{:?}", self).to_lowercase()
        }
        #[cfg(windows)]
        {
            format!("{:?}.exe", self).to_lowercase()
        }
    }
}

pub struct Backend {
    kind: BackendKind,
    path: PathBuf,
}

impl Backend {
    pub fn new(kind: BackendKind) -> Result<Self> {
        let filename = kind.to_filename();
        let path = if Cache::is_cached(&filename) {
            Cache::path(&filename)?
        } else {
            #[cfg(all(target_arch = "x86_64", target_os = "windows"))]
            {
                match kind {
                    BackendKind::Hacpack => Cache::store_bytes(defines::HACPACK, &filename)?,
                    BackendKind::Hactool => Cache::store_bytes(defines::HACTOOL, &filename)?,
                    BackendKind::Hactoolnet => Cache::store_bytes(defines::HACTOOLNET, &filename)?,
                    BackendKind::Hac2l => Cache::store_bytes(defines::HAC2L, &filename)?,
                }
            }
            #[cfg(unix)]
            {
                let path = match kind {
                    BackendKind::Hacpack => Cache::store_path(make_hacpack()?)?,
                    BackendKind::Hactool => Cache::store_path(make_hactool()?)?,
                    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
                    BackendKind::Hactoolnet => Cache::store_bytes(defines::HACTOOLNET, &filename)?,
                    #[cfg(all(target_arch = "x86_64", target_os = "linux"))]
                    BackendKind::Hac2l => Cache::store_path(make_hac2l(["linux_x64_release"])?)?,
                    // #[cfg(feature = "android-proot")]
                    // BackendKind::Hac2l => Cache::store_bytes(defines::HAC2L, &filename)?,
                };
                set_executable_bit(&path, true)?;
                path
            }
        };

        Ok(Self { kind, path })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn kind(&self) -> BackendKind {
        self.kind
    }
}

#[cfg(unix)]
static NPROC: Lazy<Result<u8>> = Lazy::new(|| {
    Ok(String::from_utf8(Command::new("nproc").output()?.stdout)?
        .trim()
        .parse()?)
});

#[cfg(unix)]
pub fn make_hacpack() -> Result<PathBuf> {
    use crate::{defines::APP_CACHE_DIR, utils::move_file};
    use eyre::{bail, eyre};
    use fs_err as fs;
    use tracing::info;

    let name = format!("{:?}", BackendKind::Hacpack).to_lowercase();
    info!("Building {}", name);
    let src_dir = tempdir()?;

    if !Command::new("git")
        .args(["clone", "https://github.com/The-4n/hacPack"])
        .arg(src_dir.path())
        .status()?
        .success()
    {
        bail!("Failed to clone {} repo", name);
    }

    info!("Renaming config file");
    fs::rename(
        src_dir.path().join("config.mk.template"),
        src_dir.path().join("config.mk"),
    )?;

    info!("Running make");
    if !Command::new("make")
        .args([
            "-j",
            &(NPROC.as_ref().map_err(|err| eyre!(err))? / 2).to_string(),
        ])
        .current_dir(&src_dir)
        .status()?
        .success()
    {
        bail!("Failed to build {}", name);
    }

    //* Moving bin from temp dir to cache dir
    let dest = APP_CACHE_DIR.join(&name);
    move_file(src_dir.path().join(&name), &dest)?;

    Ok(dest)
}

#[cfg(unix)]
pub fn make_hactool() -> Result<PathBuf> {
    use crate::{defines::APP_CACHE_DIR, utils::move_file};
    use eyre::{bail, eyre};
    use fs_err as fs;
    use tracing::info;

    let name = format!("{:?}", BackendKind::Hactool).to_lowercase();
    info!("Building {}", name);
    let src_dir = tempdir()?;

    if !Command::new("git")
        .args(["clone", "https://github.com/SciresM/hactool"])
        .arg(src_dir.path())
        .status()?
        .success()
    {
        bail!("Failed to clone {} repo", name);
    }

    info!("Renaming config file");
    fs::rename(
        src_dir.path().join("config.mk.template"),
        src_dir.path().join("config.mk"),
    )?;

    // removing line 372 as it causes build to fail on android
    #[cfg(target_os = "android")]
    {
        use std::io::{BufRead, BufReader};

        info!("Removing line 372 from `main.c`");
        let reader = BufReader::new(fs::File::open(src_dir.path().join("main.c"))?);
        //* can't use advance_by yet
        let fixed_main = reader
            .lines()
            .enumerate()
            .filter_map(|(i, ln)| {
                if i != 371 {
                    // i.e ln 372
                    return Some(ln);
                }
                None
            })
            .collect::<Result<Vec<_>, _>>()?
            .join("\n");
        fs::write(src_dir.path().join("main.c"), fixed_main.as_bytes())?;
    }

    info!("Running make");
    if !Command::new("make")
        .args([
            "-j",
            &(NPROC.as_ref().map_err(|err| eyre!(err))? / 2).to_string(),
        ])
        .current_dir(&src_dir)
        .status()?
        .success()
    {
        bail!("Failed to build {}", name);
    }

    //* Moving bin from temp dir to cache dir
    let dest = APP_CACHE_DIR.join(&name);
    move_file(src_dir.path().join(&name), &dest)?;

    Ok(dest)
}

#[cfg(all(target_arch = "x86_64", unix))]
pub fn make_hac2l<I, S>(args: I) -> Result<PathBuf>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    use eyre::{bail, eyre};
    use tracing::info;

    use crate::{defines::APP_CACHE_DIR, utils::move_file};

    let name = format!("{:?}", BackendKind::Hac2l).to_lowercase();
    info!("Building {}", name);
    let src_dir = tempdir()?;

    if !Command::new("git")
        .args(["clone", "https://github.com/Atmosphere-NX/Atmosphere.git"])
        .arg(src_dir.path())
        .status()?
        .success()
    {
        bail!("Failed to clone Atmosphere repo");
    }

    let hac2l_src_dir = src_dir.path().join("tools/hac2l");
    if !Command::new("git")
        .args(["clone", "https://github.com/Atmosphere-NX/hac2l.git"])
        .arg(&hac2l_src_dir)
        .status()?
        .success()
    {
        bail!("Failed to clone {} repo", name);
    }

    info!("Running make");

    if !Command::new("make")
        .args([
            "-j",
            &(NPROC.as_ref().map_err(|err| eyre!(err))? / 2).to_string(),
        ])
        .current_dir(&hac2l_src_dir)
        .status()?
        .success()
    {
        bail!("Failed to build {}", name);
    }

    //* Moving bin from temp dir to cache dir
    let dest = APP_CACHE_DIR.join(&name);
    move_file(
        hac2l_src_dir
            .join("out/generic_linux_x64/release")
            .join(&name),
        &dest,
    )?;

    Ok(dest)
}
