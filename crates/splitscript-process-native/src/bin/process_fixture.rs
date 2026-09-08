use std::{hint::black_box, io};

const MARKER: &[u8] = b"SplitScript native process probe";

fn main() -> io::Result<()> {
    let marker = Box::<[u8]>::from(MARKER);
    println!(
        "pid={};address={};length={};expected={}",
        std::process::id(),
        marker.as_ptr() as usize,
        marker.len(),
        str::from_utf8(&marker).unwrap(),
    );

    // Keep the allocation alive until the smoke-test parent closes stdin.
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    black_box(marker);
    Ok(())
}
