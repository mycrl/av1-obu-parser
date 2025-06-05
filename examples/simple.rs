use tokio::{fs::OpenOptions, io::AsyncReadExt};

use clap::Parser;
use av1_obu_parser::{buffer::Buffer, obu::ObuParser};

#[derive(Parser)]
#[command(
    about = env!("CARGO_PKG_DESCRIPTION"),
    version = env!("CARGO_PKG_VERSION"),
    author = env!("CARGO_PKG_AUTHORS"),
)]
struct Configure {
    #[arg(long)]
    input: String,
}

#[tokio::main]
async fn main() {
    let config = Configure::parse();
    let mut parser = ObuParser::default();

    let mut file = OpenOptions::new()
        .read(true)
        .open(&config.input)
        .await
        .unwrap();

    let mut buf = [0u8; 4096];
    let size = file.read(&mut buf).await.unwrap();

    let mut buffer = Buffer::new(&buf[..size]);
    println!("{:#?}", parser.parse(&mut buffer).unwrap());
    println!("{:#?}", parser.parse(&mut buffer).unwrap());
    println!("{:#?}", parser.parse(&mut buffer).unwrap());
}
