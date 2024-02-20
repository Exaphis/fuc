use std::{
    borrow::Cow,
    ffi::OsStr,
    fmt::Debug,
    io,
    marker::PhantomData,
    path::{Path, MAIN_SEPARATOR_STR},
};

use file_mode::{ModeError, ModePath};
use typed_builder::TypedBuilder;

use crate::{
    ops::{compat::DirectoryOp, IoErr},
    Error,
};

#[derive(Debug, Clone, Copy)]
pub enum ChmodMode<'a> {
    Octal(u32),
    Symbolic(&'a str),
}

impl<'a> ChmodMode<'a> {
    pub fn new(mode: &'a str) -> Self {
        match u32::from_str_radix(mode, 8) {
            Ok(number) => ChmodMode::Octal(number),
            Err(_) => ChmodMode::Symbolic(mode),
        }
    }
}

/// Removes a file or directory at this path, after removing all its contents.
///
/// This function does **not** follow symbolic links: it will simply remove
/// the symbolic link itself.
///
/// # Errors
///
/// Returns the underlying I/O errors that occurred.
pub fn chmod_file<P: AsRef<Path>>(path: P, mode: ChmodMode) -> Result<(), Error> {
    ChmodOp::builder()
        .files([Cow::Borrowed(path.as_ref())])
        .mode(mode)
        .build()
        .run()
}

#[derive(TypedBuilder, Debug)]
pub struct ChmodOp<'a, I: Into<Cow<'a, Path>> + 'a, F: IntoIterator<Item = I>> {
    files: F,
    mode: ChmodMode<'a>,
    #[builder(default = false)]
    force: bool,
    #[builder(default)]
    _marker: PhantomData<&'a I>,
}

impl<'a, I: Into<Cow<'a, Path>>, F: IntoIterator<Item = I>> ChmodOp<'a, I, F> {
    /// Consume and run this chmod operation.
    ///
    /// # Errors
    ///
    /// Returns the underlying I/O errors that occurred.
    pub fn run(self) -> Result<(), Error> {
        let chmod = compat::chmod_impl();
        let result = schedule_chmod(self, &chmod);
        chmod.finish().and(result)
    }
}

#[cfg_attr(
    feature = "tracing",
    tracing::instrument(level = "trace", skip(files, remove))
)]
fn schedule_chmod<'a, I: Into<Cow<'a, Path>>, F: IntoIterator<Item = I>>(
    ChmodOp {
        files,
        mode,
        force,
        _marker: _,
    }: ChmodOp<'a, I, F>,
    chmod: &impl DirectoryOp<(Cow<'a, Path>, ChmodMode<'a>)>,
) -> Result<(), Error> {
    for file in files {
        let file = file.into();
        let stripped_path = {
            let trailing_slash_stripped = file
                .as_os_str()
                .as_encoded_bytes()
                .strip_suffix(MAIN_SEPARATOR_STR.as_bytes())
                .unwrap_or(file.as_os_str().as_encoded_bytes());
            let path = unsafe { OsStr::from_encoded_bytes_unchecked(trailing_slash_stripped) };
            Path::new(path)
        };

        let is_dir = match stripped_path.symlink_metadata() {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if force {
                    continue;
                }

                return Err(Error::NotFound {
                    file: stripped_path.to_path_buf(),
                });
            }
            r => r,
        }
        .map_io_err(|| format!("Failed to read metadata for file: {stripped_path:?}"))?
        .is_dir();

        if is_dir {
            chmod.run(
                (if file.as_os_str().len() == stripped_path.as_os_str().len() {
                    file
                } else {
                    Cow::Owned(stripped_path.to_path_buf())
                }, mode)
            )?;
        } else {
            match mode {
                ChmodMode::Octal(mode) => stripped_path.set_mode(mode),
                ChmodMode::Symbolic(mode) => stripped_path.set_mode(mode),
            }.map_err(|e| {
                match e {
                    ModeError::IoError(e) => Error::Io {
                        error: e,
                        context: format!("Failed to chmod file: {stripped_path:?}").into(),
                    },
                    ModeError::ModeParseError(e) => e.into(),
                }
            })?;
        }
    }
    Ok(())
}

mod compat {
    use std::{borrow::Cow, path::Path};

    use file_mode::{ModeError, ModePath};
    use rayon::prelude::*;

    use crate::{
        ops::compat::DirectoryOp,
        Error,
    };

    use super::ChmodMode;

    struct Impl;

    pub fn chmod_impl<'a>() -> impl DirectoryOp<(Cow<'a, Path>, ChmodMode<'a>)> {
        Impl
    }

    impl DirectoryOp<(Cow<'_, Path>, ChmodMode<'_>)> for Impl {
        fn run(&self, (dir, mode): (Cow<Path>, ChmodMode)) -> Result<(), Error> {
            chmod_dir_all(&dir, mode).map_err(|e| {
                match e {
                    ModeError::IoError(e) => Error::Io {
                        error: e,
                        context: format!("Failed to chmod directory: {dir:?}").into(),
                    },
                    ModeError::ModeParseError(e) => e.into(),
                }
            })
        }

        fn finish(self) -> Result<(), Error> {
            Ok(())
        }
    }

    fn chmod_dir_all<P: AsRef<Path>>(path: P, mode: ChmodMode) -> Result<(), ModeError> {
        let path = path.as_ref();
        path.read_dir()?
            .par_bridge()
            .try_for_each(|dir_entry| -> Result<(), ModeError> {
                let dir_entry = dir_entry?;
                if dir_entry.file_type()?.is_dir() {
                    chmod_dir_all(dir_entry.path(), mode)?;
                } else {
                    match mode {
                        ChmodMode::Octal(mode) => dir_entry.path().set_mode(mode)?,
                        ChmodMode::Symbolic(mode) => dir_entry.path().set_mode(mode)?,
                    };
                }
                Ok(())
            })?;
        match mode {
            ChmodMode::Octal(mode) => path.set_mode(mode),
            ChmodMode::Symbolic(mode) => path.set_mode(mode),
        }.map(|_| ())
    }
}
