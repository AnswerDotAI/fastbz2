use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::{Parser, Subcommand};
use fastbz2::{DecodeOptions, Error, Index, Source, build_index, decompress_to_writer};
use tempfile::NamedTempFile;

#[derive(Parser)]
#[command(version, about = "Fast parallel and indexed bzip2 decompression")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Decompress INPUT, using - for stdin.
    Decode {
        input: String,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(short = 'c', long, conflicts_with = "output")]
        stdout: bool,
        #[arg(short = 'P', long, default_value_t = 0)]
        threads: usize,
        #[arg(long, default_value = "1G", value_parser = parse_size)]
        memory_limit: usize,
        #[arg(short, long)]
        force: bool,
    },
    /// Fully decode and validate INPUT without writing plaintext.
    Test {
        input: String,
        #[arg(short = 'P', long, default_value_t = 0)]
        threads: usize,
        #[arg(long, default_value = "1G", value_parser = parse_size)]
        memory_limit: usize,
    },
    /// Build a validated, source-bound block index.
    Index {
        input: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[arg(short = 'P', long, default_value_t = 0)]
        threads: usize,
        #[arg(long, default_value = "1G", value_parser = parse_size)]
        memory_limit: usize,
        #[arg(short, long)]
        force: bool,
    },
    /// Validate INPUT and show its stream/block layout.
    List {
        input: PathBuf,
        #[arg(short = 'P', long, default_value_t = 0)]
        threads: usize,
        #[arg(long, default_value = "1G", value_parser = parse_size)]
        memory_limit: usize,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("fastbz2: {error}");
            ExitCode::from(exit_status(&error))
        }
    }
}

fn run(cli: Cli) -> fastbz2::Result<()> {
    match cli.command {
        Command::Decode { input, output, stdout, threads, memory_limit, force } => {
            let options = DecodeOptions { threads, memory_limit };
            if input == "-" {
                if stdout || output.is_none() {
                    return decode_stdin(&mut io::stdout().lock());
                }
                return atomic_write(output.as_ref().unwrap(), force, decode_stdin);
            }
            let input_path = Path::new(&input);
            let source = Source::open(input_path)?;
            if stdout {
                decompress_to_writer(source.as_slice(), &mut io::stdout().lock(), options)?;
            } else {
                let output = output.unwrap_or_else(|| default_output(input_path));
                atomic_write(&output, force, |writer| decompress_to_writer(source.as_slice(), writer, options))?;
            }
        }
        Command::Test { input, threads, memory_limit } => {
            if input == "-" {
                decode_stdin(&mut io::sink())?;
            } else {
                let source = Source::open(input)?;
                decompress_to_writer(source.as_slice(), &mut io::sink(), DecodeOptions { threads, memory_limit })?;
            }
        }
        Command::Index { input, output, threads, memory_limit, force } => {
            let source = Source::open(&input)?;
            let index = build_index(source.as_slice(), DecodeOptions { threads, memory_limit })?;
            let output = output.unwrap_or_else(|| PathBuf::from(format!("{}.fbz2i", input.display())));
            atomic_write(&output, force, |writer| writer.write_all(&index.to_bytes()).map_err(Error::from))?;
        }
        Command::List { input, threads, memory_limit } => {
            let source = Source::open(input)?;
            let index = build_index(source.as_slice(), DecodeOptions { threads, memory_limit })?;
            print_index(&index);
        }
    }
    Ok(())
}

fn decode_stdin(output: &mut impl Write) -> fastbz2::Result<()> {
    let mut input = Vec::new();
    io::stdin().lock().read_to_end(&mut input)?;
    decompress_to_writer(&input, output, DecodeOptions::default())
}

fn atomic_write(path: &Path, force: bool, write: impl FnOnce(&mut fs::File) -> fastbz2::Result<()>) -> fastbz2::Result<()> {
    if path.exists() && !force {
        return Err(Error::Io(io::Error::new(io::ErrorKind::AlreadyExists, format!("{} already exists (use --force)", path.display()))));
    }
    let parent = path.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or_else(|| Path::new("."));
    let mut temporary = NamedTempFile::new_in(parent)?;
    write(temporary.as_file_mut())?;
    temporary.as_file_mut().flush()?;
    if force {
        temporary.persist(path).map_err(|error| Error::Io(error.error))?;
    } else {
        temporary.persist_noclobber(path).map_err(|error| Error::Io(error.error))?;
    }
    Ok(())
}

fn default_output(input: &Path) -> PathBuf {
    match input.extension().and_then(|extension| extension.to_str()) {
        Some("bz2") => input.with_extension(""),
        _ => PathBuf::from(format!("{}.out", input.display())),
    }
}

fn print_index(index: &Index) {
    println!("compressed_bytes\t{}", index.source_len);
    println!("decoded_bytes\t{}", index.decoded_len);
    println!("streams\t{}", index.streams.len());
    println!("blocks\t{}", index.blocks.len());
    for (number, stream) in index.streams.iter().enumerate() {
        println!(
            "stream\t{number}\theader={}\tlevel={}\tblocks={}\tdecoded={}",
            stream.compressed_header_byte, stream.block_size_100k, stream.block_count, stream.decoded_len
        );
    }
}

fn parse_size(value: &str) -> Result<usize, String> {
    let split = value.find(|character: char| !character.is_ascii_digit()).unwrap_or(value.len());
    let number: usize = value[..split].parse().map_err(|_| format!("invalid size {value:?}"))?;
    let multiplier = match value[split..].to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "k" | "kb" | "kib" => 1024,
        "m" | "mb" | "mib" => 1024 * 1024,
        "g" | "gb" | "gib" => 1024 * 1024 * 1024,
        _ => return Err(format!("invalid size suffix in {value:?}")),
    };
    number.checked_mul(multiplier).ok_or_else(|| format!("size {value:?} overflows this platform"))
}

fn exit_status(error: &Error) -> u8 {
    match error {
        Error::Io(source) if source.kind() != io::ErrorKind::InvalidData => 1,
        Error::InvalidConfiguration(_) => 2,
        Error::InvalidStreamHeader | Error::Decode { .. } | Error::InvalidIndex(_) => 3,
        _ => 4,
    }
}
