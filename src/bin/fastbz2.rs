use std::{
    fs,
    io::{self, IsTerminal, Read, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    time::{Duration, Instant},
};

use clap::{ArgGroup, Parser};
use fastbz2::{
    DecodeOptions, DecodeProgress, Error, Index, Source, build_index_with_progress, decode_to_writer_with_progress, decompress_to_writer_with_progress,
};
use serde_json::{Value, json};
use tempfile::NamedTempFile;

#[derive(Parser)]
#[command(
    version,
    about = "Fast parallel and indexed bzip2 decompression",
    group(ArgGroup::new("mode").args(["test", "index", "list"]))
)]
struct Cli {
    /// Input files, or - for stdin when decoding or testing.
    #[arg(required = true, num_args = 1..)]
    inputs: Vec<String>,
    /// Fully decode and validate without writing plaintext.
    #[arg(long)]
    test: bool,
    /// Build validated, source-bound block indexes.
    #[arg(long)]
    index: bool,
    /// Validate and show stream/block layouts.
    #[arg(long)]
    list: bool,
    /// Output path, or - for stdout; requires one input.
    #[arg(short, long, conflicts_with_all = ["test", "list", "output_dir"])]
    output: Option<PathBuf>,
    /// Put decoded files in DIRECTORY.
    #[arg(short = 'C', long = "output-dir", conflicts_with_all = ["test", "index", "list", "output"])]
    output_dir: Option<PathBuf>,
    /// Worker threads; 0 uses all available CPUs.
    #[arg(short = 'P', long, default_value_t = 0)]
    threads: usize,
    /// Maximum speculative decoded output.
    #[arg(long, default_value = "1G", value_parser = parse_size)]
    memory_limit: usize,
    /// Refuse to decode more than SIZE bytes per input.
    #[arg(long, value_parser = parse_size)]
    max_output: Option<usize>,
    /// Replace existing output files.
    #[arg(short, long, conflicts_with_all = ["test", "list", "skip_existing"])]
    force: bool,
    /// Skip existing output files.
    #[arg(long, conflicts_with_all = ["test", "list", "force"])]
    skip_existing: bool,
    /// Remove compressed inputs after successful extraction.
    #[arg(long = "rm", conflicts_with_all = ["test", "index", "list"])]
    remove_input: bool,
    /// Suppress progress and skip notices.
    #[arg(short, long)]
    quiet: bool,
    /// Emit list output as JSON.
    #[arg(long, requires = "list")]
    json: bool,
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
    validate_cli(&cli)?;
    let options = DecodeOptions { threads: cli.threads, memory_limit: cli.memory_limit };
    if cli.test {
        for input in &cli.inputs {
            decode_input(input, &mut io::sink(), options, cli.max_output, cli.quiet)?;
        }
        return Ok(());
    }
    if cli.index {
        return run_index(&cli, options);
    }
    if cli.list {
        return run_list(&cli, options);
    }
    run_decode(&cli, options)
}

fn validate_cli(cli: &Cli) -> fastbz2::Result<()> {
    if cli.output.is_some() && cli.inputs.len() != 1 {
        return Err(invalid("--output requires exactly one input"));
    }
    if cli.inputs.iter().any(|input| input == "-") && cli.inputs.len() != 1 {
        return Err(invalid("stdin must be the only input"));
    }
    if (cli.index || cli.list) && cli.inputs.iter().any(|input| input == "-") {
        return Err(invalid("stdin is supported only for decoding and --test"));
    }
    Ok(())
}

fn run_decode(cli: &Cli, options: DecodeOptions) -> fastbz2::Result<()> {
    if let Some(directory) = &cli.output_dir {
        fs::create_dir_all(directory)?;
    }
    for input in &cli.inputs {
        if input == "-" || cli.output.as_deref() == Some(Path::new("-")) {
            decode_input(input, &mut io::stdout().lock(), options, cli.max_output, cli.quiet)?;
            if cli.remove_input && input != "-" {
                fs::remove_file(input)?;
            }
            continue;
        }
        let input_path = Path::new(input);
        let output = cli.output.clone().unwrap_or_else(|| output_in(input_path, cli.output_dir.as_deref()));
        if input_path == output {
            return Err(invalid(format!("input and output are both {}", input_path.display())));
        }
        if should_skip(&output, cli.skip_existing, cli.quiet) {
            continue;
        }
        let source = Source::open(input_path)?;
        atomic_write(&output, cli.force, |writer| decode_data(source.as_slice(), input, writer, options, cli.max_output, cli.quiet))?;
        preserve_metadata(input_path, &output)?;
        if cli.remove_input {
            fs::remove_file(input_path)?;
        }
    }
    Ok(())
}

fn run_index(cli: &Cli, options: DecodeOptions) -> fastbz2::Result<()> {
    for input in &cli.inputs {
        let input_path = Path::new(input);
        let output = cli.output.clone().unwrap_or_else(|| PathBuf::from(format!("{}.fbz2i", input_path.display())));
        if output != Path::new("-") && should_skip(&output, cli.skip_existing, cli.quiet) {
            continue;
        }
        let source = Source::open(input_path)?;
        let index = build_index_data(source.as_slice(), input, options, cli.max_output, cli.quiet)?;
        let encoded = index.to_bytes();
        if output == Path::new("-") {
            io::stdout().lock().write_all(&encoded)?;
        } else {
            atomic_write(&output, cli.force, |writer| writer.write_all(&encoded).map_err(Error::from))?;
        }
    }
    Ok(())
}

fn run_list(cli: &Cli, options: DecodeOptions) -> fastbz2::Result<()> {
    let mut values = Vec::new();
    for input in &cli.inputs {
        let source = Source::open(input)?;
        let index = build_index_data(source.as_slice(), input, options, cli.max_output, cli.quiet)?;
        if cli.json {
            values.push(index_json(input, &index));
        } else {
            print_index((cli.inputs.len() > 1).then_some(input), &index);
        }
    }
    if cli.json {
        let value = if values.len() == 1 { values.pop().unwrap() } else { Value::Array(values) };
        serde_json::to_writer_pretty(io::stdout().lock(), &value).map_err(|error| Error::Io(io::Error::other(error)))?;
        println!();
    }
    Ok(())
}

fn decode_input(input: &str, output: &mut impl Write, options: DecodeOptions, max_output: Option<usize>, quiet: bool) -> fastbz2::Result<()> {
    if input == "-" {
        let mut data = Vec::new();
        io::stdin().lock().read_to_end(&mut data)?;
        decode_data(&data, "stdin", output, options, max_output, quiet)
    } else {
        let source = Source::open(input)?;
        decode_data(source.as_slice(), input, output, options, max_output, quiet)
    }
}

fn decode_data(data: &[u8], label: &str, output: &mut impl Write, options: DecodeOptions, max_output: Option<usize>, quiet: bool) -> fastbz2::Result<()> {
    let mut output = LimitedWriter::new(output, max_output);
    let mut display = ProgressDisplay::new(label, data.len() as u64, quiet);
    let result = decompress_to_writer_with_progress(data, &mut output, options, |progress| display.update(progress));
    display.finish();
    result
}

fn build_index_data(data: &[u8], label: &str, options: DecodeOptions, max_output: Option<usize>, quiet: bool) -> fastbz2::Result<Index> {
    let mut display = ProgressDisplay::new(label, data.len() as u64, quiet);
    let result = if let Some(limit) = max_output {
        let mut sink = LimitedWriter::new(io::sink(), Some(limit));
        decode_to_writer_with_progress(data, &mut sink, options, |progress| display.update(progress))
    } else {
        build_index_with_progress(data, options, |progress| display.update(progress))
    };
    display.finish();
    result
}

struct LimitedWriter<W> {
    inner: W,
    written: usize,
    limit: Option<usize>,
}

impl<W> LimitedWriter<W> {
    fn new(inner: W, limit: Option<usize>) -> Self {
        Self { inner, written: 0, limit }
    }
}

impl<W: Write> Write for LimitedWriter<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if self.limit.is_some_and(|limit| self.written.saturating_add(buffer.len()) > limit) {
            return Err(io::Error::new(io::ErrorKind::InvalidData, format!("decoded output exceeds {}", format_bytes(self.limit.unwrap() as u64))));
        }
        let written = self.inner.write(buffer)?;
        self.written += written;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

struct ProgressDisplay {
    stderr: io::Stderr,
    label: String,
    total: u64,
    started: Instant,
    last_draw: Instant,
    enabled: bool,
    drawn: bool,
}

impl ProgressDisplay {
    fn new(label: &str, total: u64, quiet: bool) -> Self {
        let now = Instant::now();
        Self { stderr: io::stderr(), label: label.into(), total, started: now, last_draw: now, enabled: !quiet && io::stderr().is_terminal(), drawn: false }
    }

    fn update(&mut self, progress: DecodeProgress) {
        if !self.enabled {
            return;
        }
        let elapsed = self.started.elapsed();
        let finished = progress.compressed_bytes >= self.total;
        if (!self.drawn && elapsed < Duration::from_millis(200)) || (!finished && self.last_draw.elapsed() < Duration::from_millis(100)) {
            return;
        }
        let compressed = progress.compressed_bytes.min(self.total);
        let percent = if self.total == 0 { 100.0 } else { compressed as f64 * 100.0 / self.total as f64 };
        let seconds = elapsed.as_secs_f64().max(0.001);
        let rate = progress.decoded_bytes as f64 / seconds;
        let ratio = if compressed == 0 { 0.0 } else { progress.decoded_bytes as f64 / compressed as f64 };
        let eta = if compressed == 0 || finished { 0.0 } else { seconds * (self.total - compressed) as f64 / compressed as f64 };
        let _ = write!(
            self.stderr,
            "\r\x1b[2K{}: {:5.1}%  {} → {}  {}/s  {:.1}×  ETA {}",
            self.label,
            percent,
            format_bytes(compressed),
            format_bytes(progress.decoded_bytes),
            format_bytes(rate as u64),
            ratio,
            format_duration(eta),
        );
        let _ = self.stderr.flush();
        self.last_draw = Instant::now();
        self.drawn = true;
    }

    fn finish(&mut self) {
        if self.drawn {
            let _ = writeln!(self.stderr);
            self.drawn = false;
        }
    }
}

impl Drop for ProgressDisplay {
    fn drop(&mut self) {
        self.finish();
    }
}

fn should_skip(path: &Path, skip_existing: bool, quiet: bool) -> bool {
    if !path.exists() || !skip_existing {
        return false;
    }
    if !quiet {
        eprintln!("fastbz2: skipping existing {}", path.display());
    }
    true
}

fn preserve_metadata(input: &Path, output: &Path) -> fastbz2::Result<()> {
    let metadata = fs::metadata(input)?;
    if let Ok(modified) = metadata.modified() {
        fs::OpenOptions::new().write(true).open(output)?.set_times(fs::FileTimes::new().set_modified(modified))?;
    }
    fs::set_permissions(output, metadata.permissions())?;
    Ok(())
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

fn output_in(input: &Path, directory: Option<&Path>) -> PathBuf {
    let output = default_output(input.file_name().map(Path::new).unwrap_or(input));
    directory.map_or_else(|| default_output(input), |directory| directory.join(output))
}

fn default_output(input: &Path) -> PathBuf {
    match input.extension().and_then(|extension| extension.to_str()) {
        Some("bz2") => input.with_extension(""),
        _ => PathBuf::from(format!("{}.out", input.display())),
    }
}

fn print_index(input: Option<&String>, index: &Index) {
    if let Some(input) = input {
        println!("input\t{input}");
    }
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

fn index_json(input: &str, index: &Index) -> Value {
    json!({
        "input": input,
        "source_bytes": index.source_len,
        "source_hash": hex(&index.source_hash),
        "decoded_bytes": index.decoded_len,
        "streams": index.streams.iter().enumerate().map(|(number, stream)| json!({
            "number": number,
            "header_byte": stream.compressed_header_byte,
            "block_size_100k": stream.block_size_100k,
            "first_block": stream.first_block,
            "block_count": stream.block_count,
            "decoded_start": stream.decoded_start,
            "decoded_bytes": stream.decoded_len,
            "eos_bit": stream.eos_bit,
            "expected_crc": stream.expected_stream_crc,
        })).collect::<Vec<_>>(),
        "blocks": index.blocks.iter().enumerate().map(|(number, block)| json!({
            "number": number,
            "compressed_start_bit": block.compressed_start_bit,
            "compressed_end_bit": block.compressed_end_bit,
            "decoded_start": block.decoded_start,
            "decoded_bytes": block.decoded_len,
            "expected_crc": block.expected_crc,
            "stream": block.stream,
        })).collect::<Vec<_>>(),
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} B") } else { format!("{value:.1} {}", UNITS[unit]) }
}

fn format_duration(seconds: f64) -> String {
    let seconds = seconds.max(0.0).round() as u64;
    if seconds >= 3600 {
        format!("{}h{:02}m", seconds / 3600, seconds / 60 % 60)
    } else if seconds >= 60 {
        format!("{}m{:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{seconds}s")
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
        "t" | "tb" | "tib" => 1024_usize.pow(4),
        _ => return Err(format!("invalid size suffix in {value:?}")),
    };
    number.checked_mul(multiplier).ok_or_else(|| format!("size {value:?} overflows this platform"))
}

fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidConfiguration(message.into())
}

fn exit_status(error: &Error) -> u8 {
    match error {
        Error::Io(source) if source.kind() == io::ErrorKind::InvalidData => 3,
        Error::Io(_) => 1,
        Error::InvalidConfiguration(_) => 2,
        Error::InvalidStreamHeader | Error::Decode { .. } | Error::InvalidIndex(_) => 3,
        _ => 4,
    }
}
