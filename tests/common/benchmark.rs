pub fn elapsed(repeats: usize, mut run: impl FnMut()) -> std::time::Duration {
    run();
    let start = std::time::Instant::now();
    for _ in 0..repeats { run() }
    start.elapsed()
}
