use std::{collections::HashSet, fs, io::Write, path::Path};

use fbz::{Bzip2EncodeReport, Bzip2Encoder, EncodeOptions, Error, Result, gzip, lz4};

use super::archive_create::archive_name;

fn invalid(message: impl Into<String>) -> Error { Error::InvalidConfiguration(message.into()) }

fn append_inputs<W: Write>(inputs: &[String], encoder: W) -> Result<W> {
    let mut archive = tar::Builder::new(encoder);
    let mut names = HashSet::new();
    for input in inputs {
        if input == "-" { return Err(invalid("stdin cannot be used as a tar archive entry")); }
        let path = Path::new(input);
        let name = archive_name(path)?;
        if !names.insert(name.clone()) { return Err(invalid(format!("duplicate archive root {}", name.display()))); }
        let metadata = fs::symlink_metadata(path)?;
        if metadata.is_dir() { archive.append_dir_all(&name, path)?; } else { archive.append_path_with_name(path, &name)?; }
    }
    archive.into_inner().map_err(Error::from)
}

pub(super) fn pack_gzip<W: Write + ?Sized>(inputs: &[String], output: &mut W, options: EncodeOptions) -> Result<gzip::EncodeReport> {
    append_inputs(inputs, gzip::Encoder::new(output, options)?)?.finish().map(|(_, report)| report)
}

pub(super) fn pack_lz4<W: Write + ?Sized>(inputs: &[String], output: &mut W, options: EncodeOptions) -> Result<lz4::EncodeReport> {
    append_inputs(inputs, lz4::Encoder::new(output, options)?)?.finish().map(|(_, report)| report)
}

pub(super) fn pack_bzip2<W: Write + ?Sized>(inputs: &[String], output: &mut W, options: EncodeOptions) -> Result<Bzip2EncodeReport> {
    append_inputs(inputs, Bzip2Encoder::new(output, options)?)?.finish().map(|(_, report)| report)
}
